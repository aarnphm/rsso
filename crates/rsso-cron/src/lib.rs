use async_trait::async_trait;
use rsso_domain::{GameId, GameModeKind, GameStatus, QueueId, TeamSide};
use rsso_storage::{
    GameRow, LiveGameUpdate, MatchRecord, ParticipantRecord, RosterPlayer, Storage, StorageError,
};
use std::collections::{HashMap, HashSet};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronSummary {
    pub inspected: usize,
    pub marked_live: usize,
    pub bumped_not_found: usize,
    pub finalized: usize,
    pub missing_puuid: usize,
    pub probe_errors: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveGameStatus {
    Live(LiveGame),
    NotFound,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveGame {
    pub riot_match_id: Option<String>,
    pub queue_id: Option<u16>,
    pub map_id: Option<u16>,
    pub game_mode: Option<String>,
    pub game_type: Option<String>,
    pub participant_puuids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinishedMatch {
    pub riot_match_id: String,
    pub queue_id: Option<u16>,
    pub map_id: Option<u16>,
    pub game_mode: Option<String>,
    pub game_type: Option<String>,
    pub payload_json: Option<String>,
    pub participants: Vec<FinishedParticipant>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinishedParticipant {
    pub puuid: String,
    pub team_id: Option<u16>,
    pub champion_id: Option<u16>,
    pub champion_name: Option<String>,
    pub win: Option<bool>,
    pub kills: Option<u16>,
    pub deaths: Option<u16>,
    pub assists: Option<u16>,
    pub total_damage: Option<u32>,
    pub gold_earned: Option<u32>,
    pub total_minions: Option<u32>,
    pub vision_score: Option<u32>,
    pub raw_json: Option<String>,
}

#[async_trait(?Send)]
pub trait LiveGameProbe {
    async fn active_game_by_puuid(&self, puuid: &str) -> Result<LiveGameStatus, String>;
}

#[async_trait(?Send)]
pub trait FinishedMatchProbe {
    async fn finished_match(&self, riot_match_id: &str) -> Result<Option<FinishedMatch>, String>;
}

#[derive(Debug, Clone, Copy)]
pub struct NoopLiveGameProbe;

#[async_trait(?Send)]
impl LiveGameProbe for NoopLiveGameProbe {
    async fn active_game_by_puuid(&self, _puuid: &str) -> Result<LiveGameStatus, String> {
        Ok(LiveGameStatus::NotFound)
    }
}

#[async_trait(?Send)]
impl FinishedMatchProbe for NoopLiveGameProbe {
    async fn finished_match(&self, _riot_match_id: &str) -> Result<Option<FinishedMatch>, String> {
        Ok(None)
    }
}

#[derive(Debug, Error)]
pub enum CronError {
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error("invalid game row: {0}")]
    InvalidRow(String),
    #[error("invalid finished match: {0}")]
    InvalidMatch(String),
}

pub async fn run_poll<S, P>(storage: &S, probe: &P, now: i64) -> Result<CronSummary, CronError>
where
    S: Storage + ?Sized,
    P: LiveGameProbe + FinishedMatchProbe + ?Sized,
{
    let games = storage.active_games().await?;
    let inspected = games.len();
    let mut marked_live = 0;
    let mut bumped_not_found = 0;
    let mut finalized = 0;
    let mut missing_puuid = 0;
    let mut probe_errors = 0;

    for game in games {
        let status = game.status().map_err(CronError::InvalidRow)?;
        if !matches!(status, GameStatus::Randomized | GameStatus::Ingame) {
            continue;
        }

        let game_id = GameId::new(game.game_id.clone());
        let roster = storage.roster(&game_id).await?;
        let Some(puuid) = roster
            .iter()
            .find_map(|player| player.riot_puuid.as_deref())
        else {
            storage
                .emit_event(
                    &game.guild_id,
                    Some(game_id.as_str()),
                    None,
                    "cron_missing_puuid",
                    "{}",
                    now,
                )
                .await?;
            missing_puuid += 1;
            continue;
        };

        match probe.active_game_by_puuid(puuid).await {
            Ok(LiveGameStatus::Live(live_game)) => {
                if !roster_matches_live_game(&roster, &live_game) {
                    let payload = live_mismatch_payload(&roster, &live_game);
                    storage
                        .emit_event(
                            &game.guild_id,
                            Some(game_id.as_str()),
                            None,
                            "riot_spectator_roster_mismatch",
                            &payload,
                            now,
                        )
                        .await?;
                    probe_errors += 1;
                    continue;
                }

                let mode = game.mode().map_err(CronError::InvalidRow)?;
                if !mode.accepts_queue(live_game.queue_id.map(QueueId)) {
                    let payload = serde_json::json!({
                        "expected_mode": game.mode,
                        "riot_match_id": live_game.riot_match_id,
                        "queue_id": live_game.queue_id,
                        "map_id": live_game.map_id,
                        "game_mode": live_game.game_mode,
                        "game_type": live_game.game_type,
                    })
                    .to_string();
                    storage
                        .emit_event(
                            &game.guild_id,
                            Some(game_id.as_str()),
                            None,
                            "riot_spectator_mode_mismatch",
                            &payload,
                            now,
                        )
                        .await?;
                    probe_errors += 1;
                    continue;
                }

                storage
                    .mark_ingame(
                        &game_id,
                        LiveGameUpdate {
                            riot_match_id: live_game.riot_match_id,
                            queue_id: live_game.queue_id.map(i64::from),
                            map_id: live_game.map_id.map(i64::from),
                            riot_game_mode: live_game.game_mode,
                            riot_game_type: live_game.game_type,
                        },
                        now,
                    )
                    .await?;
                marked_live += 1;
            }
            Ok(LiveGameStatus::NotFound) => {
                if status == GameStatus::Ingame
                    && game.consecutive_404 + 1 >= 2
                    && try_finalize_finished_match(storage, probe, &game, &game_id, &roster, now)
                        .await?
                {
                    finalized += 1;
                    continue;
                }
                storage.bump_404(&game_id, now).await?;
                bumped_not_found += 1;
            }
            Err(error) => {
                let payload = serde_json::json!({ "error": error }).to_string();
                storage
                    .emit_event(
                        &game.guild_id,
                        Some(game_id.as_str()),
                        None,
                        "riot_spectator_error",
                        &payload,
                        now,
                    )
                    .await?;
                probe_errors += 1;
            }
        }
    }

    Ok(CronSummary {
        inspected,
        marked_live,
        bumped_not_found,
        finalized,
        missing_puuid,
        probe_errors,
    })
}

async fn try_finalize_finished_match<S, P>(
    storage: &S,
    probe: &P,
    game: &GameRow,
    game_id: &GameId,
    roster: &[RosterPlayer],
    now: i64,
) -> Result<bool, CronError>
where
    S: Storage + ?Sized,
    P: FinishedMatchProbe + ?Sized,
{
    let Some(riot_match_id) = game.riot_match_id.as_deref() else {
        return Ok(false);
    };
    let finished_match = match probe.finished_match(riot_match_id).await {
        Ok(Some(finished_match)) => finished_match,
        Ok(None) => return Ok(false),
        Err(error) => {
            let payload = serde_json::json!({ "error": error }).to_string();
            storage
                .emit_event(
                    &game.guild_id,
                    Some(game_id.as_str()),
                    None,
                    "riot_match_fetch_error",
                    &payload,
                    now,
                )
                .await?;
            return Ok(false);
        }
    };
    let Some(winner) = derive_winner(&finished_match) else {
        storage
            .emit_event(
                &game.guild_id,
                Some(game_id.as_str()),
                None,
                "riot_match_missing_winner",
                "{}",
                now,
            )
            .await?;
        return Ok(false);
    };
    let match_record = match build_match_record(game, roster, finished_match) {
        Ok(match_record) => match_record,
        Err(CronError::InvalidMatch(error)) => {
            let payload = serde_json::json!({ "error": error }).to_string();
            storage
                .emit_event(
                    &game.guild_id,
                    Some(game_id.as_str()),
                    None,
                    "riot_match_validation_error",
                    &payload,
                    now,
                )
                .await?;
            return Ok(false);
        }
        Err(error) => return Err(error),
    };
    storage.record_match(game_id, match_record, now).await?;
    storage
        .finalize_game(game_id, winner, Some(riot_match_id), now)
        .await?;
    storage
        .emit_event(
            &game.guild_id,
            Some(game_id.as_str()),
            None,
            "riot_match_auto_finalized",
            "{}",
            now,
        )
        .await?;
    Ok(true)
}

fn roster_matches_live_game(roster: &[RosterPlayer], live_game: &LiveGame) -> bool {
    let known_puuids = roster
        .iter()
        .filter_map(|player| player.riot_puuid.as_deref())
        .collect::<Vec<_>>();
    let matched_known = known_puuids
        .iter()
        .filter(|known| {
            live_game
                .participant_puuids
                .iter()
                .any(|participant| participant == **known)
        })
        .count();
    matched_known == known_puuids.len()
}

fn live_mismatch_payload(roster: &[RosterPlayer], live_game: &LiveGame) -> String {
    let known_puuids = roster
        .iter()
        .filter_map(|player| player.riot_puuid.as_deref())
        .collect::<Vec<_>>();
    let matched_known = known_puuids
        .iter()
        .filter(|known| {
            live_game
                .participant_puuids
                .iter()
                .any(|participant| participant == **known)
        })
        .count();
    serde_json::json!({
        "known_puuids": known_puuids.len(),
        "matched_puuids": matched_known,
        "riot_match_id": live_game.riot_match_id,
    })
    .to_string()
}

fn derive_winner(finished_match: &FinishedMatch) -> Option<TeamSide> {
    finished_match.participants.iter().find_map(|participant| {
        let won = participant.win?;
        if !won {
            return None;
        }
        side_from_team_id(participant.team_id)
    })
}

fn build_match_record(
    game: &GameRow,
    roster: &[RosterPlayer],
    finished_match: FinishedMatch,
) -> Result<MatchRecord, CronError> {
    let mode = game.mode().map_err(CronError::InvalidRow)?;
    validate_match_mode(mode, &finished_match)?;
    let roster_by_puuid = roster
        .iter()
        .filter_map(|player| {
            player
                .riot_puuid
                .as_ref()
                .map(|puuid| (puuid.as_str(), player))
        })
        .collect::<HashMap<_, _>>();
    let match_puuids = finished_match
        .participants
        .iter()
        .map(|participant| participant.puuid.as_str())
        .collect::<HashSet<_>>();
    let missing_roster_puuids = roster_by_puuid
        .keys()
        .filter(|puuid| !match_puuids.contains(**puuid))
        .count();
    if missing_roster_puuids > 0 {
        return Err(CronError::InvalidMatch(format!(
            "match is missing {missing_roster_puuids} registered roster player(s)"
        )));
    }

    let participants = finished_match
        .participants
        .into_iter()
        .map(|participant| {
            let roster_player = roster_by_puuid.get(participant.puuid.as_str());
            ParticipantRecord {
                puuid: participant.puuid,
                discord_user_id: roster_player.map(|player| player.discord_user_id.clone()),
                team: side_from_team_id(participant.team_id)
                    .or_else(|| roster_player.and_then(|player| player.team_side())),
                champion_id: participant.champion_id.map(i64::from),
                champion_name: participant.champion_name,
                win: participant.win,
                kills: participant.kills.map(i64::from),
                deaths: participant.deaths.map(i64::from),
                assists: participant.assists.map(i64::from),
                total_damage: participant.total_damage.map(i64::from),
                gold_earned: participant.gold_earned.map(i64::from),
                total_minions: participant.total_minions.map(i64::from),
                vision_score: participant.vision_score.map(i64::from),
                raw_json: participant.raw_json,
            }
        })
        .collect();
    Ok(MatchRecord {
        riot_match_id: finished_match.riot_match_id,
        guild_id: game.guild_id.clone(),
        mode,
        queue_id: finished_match.queue_id.map(i64::from),
        map_id: finished_match.map_id.map(i64::from),
        riot_game_mode: finished_match.game_mode,
        riot_game_type: finished_match.game_type,
        data_source: "match_v5".to_owned(),
        payload_json: finished_match.payload_json,
        participants,
    })
}

fn validate_match_mode(
    expected: GameModeKind,
    finished_match: &FinishedMatch,
) -> Result<(), CronError> {
    if expected.accepts_queue(finished_match.queue_id.map(QueueId)) {
        Ok(())
    } else {
        Err(CronError::InvalidMatch(format!(
            "Riot match queue {:?} does not match expected mode {}",
            finished_match.queue_id,
            expected.as_str()
        )))
    }
}

fn side_from_team_id(team_id: Option<u16>) -> Option<TeamSide> {
    match team_id {
        Some(100) => Some(TeamSide::Blue),
        Some(200) => Some(TeamSide::Red),
        _ => None,
    }
}

pub async fn run_poll_without_probe<S: Storage + ?Sized>(
    storage: &S,
    now: i64,
) -> Result<CronSummary, CronError> {
    run_poll(storage, &NoopLiveGameProbe, now).await
}
