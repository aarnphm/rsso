use rsso_domain::{GameModeKind, GameStatus, TeamSide};
use serde::{Deserialize, Serialize};

pub const PENDING_RIOT_TAG_LINE: &str = "__PENDING__";

pub fn pending_riot_game_name(discord_user_id: &str) -> String {
    format!("__pending_{discord_user_id}")
}

pub fn is_pending_riot_id(
    discord_user_id: &str,
    riot_game_name: &str,
    riot_tag_line: &str,
) -> bool {
    riot_tag_line == PENDING_RIOT_TAG_LINE
        && riot_game_name == pending_riot_game_name(discord_user_id)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewPlayer {
    pub guild_id: String,
    pub discord_user_id: String,
    pub riot_puuid: Option<String>,
    pub riot_game_name: String,
    pub riot_tag_line: String,
    pub now: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerRow {
    pub guild_id: String,
    pub discord_user_id: String,
    pub riot_puuid: Option<String>,
    pub riot_game_name: String,
    pub riot_tag_line: String,
    pub rating: i32,
    pub wins: i32,
    pub losses: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewGame {
    pub game_id: String,
    pub guild_id: String,
    pub channel_id: String,
    pub creator_discord_id: String,
    pub mode: GameModeKind,
    pub now: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameRow {
    pub game_id: String,
    pub guild_id: String,
    pub channel_id: String,
    pub creator_discord_id: String,
    pub status: String,
    pub mode: String,
    pub winning_side: Option<String>,
    pub version: i64,
    pub riot_match_id: Option<String>,
    pub consecutive_404: i64,
}

impl GameRow {
    pub fn status(&self) -> Result<GameStatus, String> {
        self.status.parse().map_err(|err| format!("{err}"))
    }

    pub fn mode(&self) -> Result<GameModeKind, String> {
        self.mode.parse().map_err(|err| format!("{err}"))
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LiveGameUpdate {
    pub riot_match_id: Option<String>,
    pub queue_id: Option<i64>,
    pub map_id: Option<i64>,
    pub riot_game_mode: Option<String>,
    pub riot_game_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RosterPlayer {
    pub discord_user_id: String,
    pub riot_puuid: Option<String>,
    pub team: Option<String>,
    pub rating: i32,
}

impl RosterPlayer {
    pub fn team_side(&self) -> Option<TeamSide> {
        self.team.as_deref().and_then(|team| team.parse().ok())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerStatsRow {
    pub guild_id: String,
    pub discord_user_id: String,
    pub riot_game_name: String,
    pub riot_tag_line: String,
    pub rating: i32,
    pub wins: i32,
    pub losses: i32,
    pub win_rate: f64,
    pub avg_kills: Option<f64>,
    pub avg_deaths: Option<f64>,
    pub avg_assists: Option<f64>,
    pub avg_total_damage: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeammateStatsRow {
    pub discord_user_id: String,
    pub riot_game_name: String,
    pub riot_tag_line: String,
    pub games: i64,
    pub wins: i64,
    pub losses: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaderboardRow {
    pub discord_user_id: String,
    pub riot_game_name: String,
    pub riot_tag_line: String,
    pub rating: i32,
    pub wins: i32,
    pub losses: i32,
    pub win_rate: f64,
    pub avg_kills: Option<f64>,
    pub avg_deaths: Option<f64>,
    pub avg_assists: Option<f64>,
    pub avg_total_damage: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchRecord {
    pub riot_match_id: String,
    pub guild_id: String,
    pub mode: GameModeKind,
    pub queue_id: Option<i64>,
    pub map_id: Option<i64>,
    pub riot_game_mode: Option<String>,
    pub riot_game_type: Option<String>,
    pub data_source: String,
    pub payload_json: Option<String>,
    pub participants: Vec<ParticipantRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchLinkRow {
    pub riot_match_id: String,
    pub data_source: String,
    pub queue_id: Option<i64>,
    pub map_id: Option<i64>,
    pub riot_game_mode: Option<String>,
    pub riot_game_type: Option<String>,
    pub finalized_at: i64,
    pub participant_count: i64,
}

impl MatchLinkRow {
    pub fn needs_hydration(&self) -> bool {
        self.data_source != "match_v5" || self.participant_count == 0
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParticipantRecord {
    pub puuid: String,
    pub discord_user_id: Option<String>,
    pub team: Option<TeamSide>,
    pub champion_id: Option<i64>,
    pub champion_name: Option<String>,
    pub win: Option<bool>,
    pub kills: Option<i64>,
    pub deaths: Option<i64>,
    pub assists: Option<i64>,
    pub total_damage: Option<i64>,
    pub gold_earned: Option<i64>,
    pub total_minions: Option<i64>,
    pub vision_score: Option<i64>,
    pub raw_json: Option<String>,
}
