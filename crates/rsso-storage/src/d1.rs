use crate::models::{
    GameRow, LeaderboardRow, LiveGameUpdate, MatchRecord, NewGame, NewPlayer, PlayerRow,
    PlayerStatsRow, RosterPlayer,
};
use crate::repository::{Storage, StorageError, StorageResult};
use async_trait::async_trait;
use rsso_domain::{GameId, GameModeKind, GameStatus, TeamAssignment, TeamSide};
use wasm_bindgen::JsValue;
use worker::D1Database;

#[derive(Debug)]
pub struct D1Storage {
    db: D1Database,
}

impl D1Storage {
    pub fn new(db: D1Database) -> Self {
        Self { db }
    }
}

#[async_trait(?Send)]
impl Storage for D1Storage {
    async fn upsert_player(&self, player: NewPlayer) -> StorageResult<()> {
        let sql = "
            INSERT INTO players (
                guild_id, discord_user_id, riot_puuid, riot_game_name, riot_tag_line,
                claim_status, consented_at, created_at, updated_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, 'trusted', ?6, ?6, ?6)
            ON CONFLICT(guild_id, discord_user_id) DO UPDATE SET
                riot_puuid = excluded.riot_puuid,
                riot_game_name = excluded.riot_game_name,
                riot_tag_line = excluded.riot_tag_line,
                updated_at = excluded.updated_at
        ";
        run(self.db.prepare(sql).bind(&[
            js(&player.guild_id),
            js(&player.discord_user_id),
            opt_js(player.riot_puuid.as_deref()),
            js(&player.riot_game_name),
            js(&player.riot_tag_line),
            js_i64(player.now),
        ])?)
        .await
    }

    async fn get_player(&self, guild_id: &str, discord_user_id: &str) -> StorageResult<PlayerRow> {
        first(
            self.db
                .prepare(
                    "
                    SELECT guild_id, discord_user_id, riot_puuid, riot_game_name, riot_tag_line,
                           rating, wins, losses
                    FROM players
                    WHERE guild_id = ?1 AND discord_user_id = ?2
                    ",
                )
                .bind(&[js(guild_id), js(discord_user_id)])?,
        )
        .await?
        .ok_or(StorageError::NotFound)
    }

    async fn create_game(&self, game: NewGame, users: &[String]) -> StorageResult<()> {
        let insert_game = self
            .db
            .prepare(
                "
                INSERT INTO games (
                    game_id, guild_id, channel_id, creator_discord_id, status, mode,
                    created_at, updated_at
                )
                VALUES (?1, ?2, ?3, ?4, 'lobby', ?5, ?6, ?6)
                ",
            )
            .bind(&[
                js(&game.game_id),
                js(&game.guild_id),
                js(&game.channel_id),
                js(&game.creator_discord_id),
                js(game.mode.as_str()),
                js_i64(game.now),
            ])?;
        run(insert_game).await.map_err(map_active_game_error)?;

        for discord_user_id in users {
            self.add_player(
                &GameId::new(game.game_id.clone()),
                &game.guild_id,
                discord_user_id,
                game.now,
            )
            .await?;
        }
        Ok(())
    }

    async fn add_player(
        &self,
        game_id: &GameId,
        guild_id: &str,
        discord_user_id: &str,
        now: i64,
    ) -> StorageResult<()> {
        let game = self.game_by_id(game_id).await?;
        if game.guild_id != guild_id {
            return Err(StorageError::NotFound);
        }
        let status = game.status().map_err(StorageError::InvalidRow)?;
        if !matches!(status, GameStatus::Lobby | GameStatus::Randomized) {
            return Err(StorageError::Conflict);
        }
        let player = self.get_player(guild_id, discord_user_id).await?;
        run(self
            .db
            .prepare(
                "
                    INSERT INTO game_players (
                        game_id, guild_id, discord_user_id, riot_puuid, pre_rating, joined_at
                    )
                    VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                    ",
            )
            .bind(&[
                js(game_id.as_str()),
                js(guild_id),
                js(discord_user_id),
                opt_js(player.riot_puuid.as_deref()),
                js_i64(i64::from(player.rating)),
                js_i64(now),
            ])?)
        .await?;

        if status == GameStatus::Randomized {
            run(self
                .db
                .prepare(
                    "
                    UPDATE game_players
                    SET team = NULL, slot = NULL
                    WHERE game_id = ?1
                    ",
                )
                .bind(&[js(game_id.as_str())])?)
            .await?;
            run(self
                .db
                .prepare(
                    "
                    UPDATE games
                    SET status = 'lobby',
                        randomized_at = NULL,
                        updated_at = ?2,
                        version = version + 1
                    WHERE game_id = ?1
                    ",
                )
                .bind(&[js(game_id.as_str()), js_i64(now)])?)
            .await?;
        }
        Ok(())
    }

    async fn open_game_for_guild(&self, guild_id: &str) -> StorageResult<Option<GameRow>> {
        first(
            self.db
                .prepare(
                    "
                    SELECT game_id, guild_id, channel_id, creator_discord_id, status, mode,
                           winning_side, version, riot_match_id, consecutive_404
                    FROM games
                    WHERE guild_id = ?1 AND is_open IS NOT NULL
                    ORDER BY created_at DESC
                    LIMIT 1
                    ",
                )
                .bind(&[js(guild_id)])?,
        )
        .await
    }

    async fn game_by_id(&self, game_id: &GameId) -> StorageResult<GameRow> {
        first(
            self.db
                .prepare(
                    "
                    SELECT game_id, guild_id, channel_id, creator_discord_id, status, mode,
                           winning_side, version, riot_match_id, consecutive_404
                    FROM games
                    WHERE game_id = ?1
                    ",
                )
                .bind(&[js(game_id.as_str())])?,
        )
        .await?
        .ok_or(StorageError::NotFound)
    }

    async fn roster(&self, game_id: &GameId) -> StorageResult<Vec<RosterPlayer>> {
        all(self
            .db
            .prepare(
                "
                    SELECT gp.discord_user_id, gp.riot_puuid, gp.team, p.rating
                    FROM game_players gp
                    JOIN players p
                      ON p.guild_id = gp.guild_id
                     AND p.discord_user_id = gp.discord_user_id
                    WHERE gp.game_id = ?1
                    ORDER BY gp.joined_at, gp.discord_user_id
                    ",
            )
            .bind(&[js(game_id.as_str())])?)
        .await
    }

    async fn assign_teams(
        &self,
        game_id: &GameId,
        assignments: &[TeamAssignment],
        now: i64,
    ) -> StorageResult<()> {
        for assignment in assignments {
            run(self
                .db
                .prepare(
                    "
                        UPDATE game_players
                        SET team = ?1, slot = ?2
                        WHERE game_id = ?3 AND discord_user_id = ?4
                        ",
                )
                .bind(&[
                    js(assignment.team.as_str()),
                    js_i64(i64::from(assignment.slot)),
                    js(game_id.as_str()),
                    js(assignment.discord_user_id.as_str()),
                ])?)
            .await?;
        }
        run(self
            .db
            .prepare(
                "
                    UPDATE games
                    SET status = 'randomized',
                        randomized_at = ?2,
                        updated_at = ?2,
                        version = version + 1
                    WHERE game_id = ?1 AND status IN ('lobby','randomized')
                    ",
            )
            .bind(&[js(game_id.as_str()), js_i64(now)])?)
        .await
    }

    async fn record_vote(
        &self,
        game_id: &GameId,
        discord_user_id: &str,
        winner: TeamSide,
        now: i64,
    ) -> StorageResult<()> {
        run(self
            .db
            .prepare(
                "
                    UPDATE game_players
                    SET result_vote = ?1, voted_at = ?2
                    WHERE game_id = ?3 AND discord_user_id = ?4
                    ",
            )
            .bind(&[
                js(winner.as_str()),
                js_i64(now),
                js(game_id.as_str()),
                js(discord_user_id),
            ])?)
        .await
    }

    async fn mark_reported(
        &self,
        game_id: &GameId,
        winner: TeamSide,
        now: i64,
    ) -> StorageResult<()> {
        run(self
            .db
            .prepare(
                "
                    UPDATE games
                    SET status = 'reported',
                        winning_side = ?2,
                        updated_at = ?3,
                        version = version + 1
                    WHERE game_id = ?1 AND status IN ('randomized','ingame','ambiguous','reported')
                    ",
            )
            .bind(&[js(game_id.as_str()), js(winner.as_str()), js_i64(now)])?)
        .await
    }

    async fn finalize_game(
        &self,
        game_id: &GameId,
        winner: TeamSide,
        riot_match_id: Option<&str>,
        now: i64,
    ) -> StorageResult<()> {
        let roster = self.roster(game_id).await?;
        let blue_sum = roster
            .iter()
            .filter(|player| player.team.as_deref() == Some("blue"))
            .map(|player| player.rating)
            .sum::<i32>();
        let red_sum = roster
            .iter()
            .filter(|player| player.team.as_deref() == Some("red"))
            .map(|player| player.rating)
            .sum::<i32>();
        let blue_count = roster
            .iter()
            .filter(|player| player.team.as_deref() == Some("blue"))
            .count()
            .max(1);
        let red_count = roster
            .iter()
            .filter(|player| player.team.as_deref() == Some("red"))
            .count()
            .max(1);
        let delta = rsso_domain::rating_delta(
            winner,
            rsso_domain::TeamRating {
                blue: f64::from(blue_sum) / blue_count as f64,
                red: f64::from(red_sum) / red_count as f64,
            },
            rsso_domain::elo::DEFAULT_K,
        );

        for player in roster {
            let Some(team) = player.team_side() else {
                continue;
            };
            let signed_delta = if team == winner { delta } else { -delta };
            let won = team == winner;
            run(self
                .db
                .prepare(
                    "
                        UPDATE players
                        SET rating = rating + ?1,
                            wins = wins + ?2,
                            losses = losses + ?3,
                            updated_at = ?4
                        WHERE guild_id = (SELECT guild_id FROM games WHERE game_id = ?5)
                          AND discord_user_id = ?6
                        ",
                )
                .bind(&[
                    js_i64(i64::from(signed_delta)),
                    js_i64(if won { 1 } else { 0 }),
                    js_i64(if won { 0 } else { 1 }),
                    js_i64(now),
                    js(game_id.as_str()),
                    js(&player.discord_user_id),
                ])?)
            .await?;
            run(self
                .db
                .prepare(
                    "
                        UPDATE game_players
                        SET post_rating = pre_rating + ?1
                        WHERE game_id = ?2 AND discord_user_id = ?3
                        ",
                )
                .bind(&[
                    js_i64(i64::from(signed_delta)),
                    js(game_id.as_str()),
                    js(&player.discord_user_id),
                ])?)
            .await?;
        }

        run(self
            .db
            .prepare(
                "
                    UPDATE games
                    SET status = 'finalized',
                        winning_side = ?2,
                        riot_match_id = COALESCE(?3, riot_match_id),
                        ended_at = ?4,
                        updated_at = ?4,
                        version = version + 1
                    WHERE game_id = ?1 AND status IN ('randomized','ingame','reported','ambiguous')
                    ",
            )
            .bind(&[
                js(game_id.as_str()),
                js(winner.as_str()),
                opt_js(riot_match_id),
                js_i64(now),
            ])?)
        .await
    }

    async fn cancel_game(&self, game_id: &GameId, now: i64) -> StorageResult<()> {
        run(self
            .db
            .prepare(
                "
                    UPDATE games
                    SET status = 'cancelled', ended_at = ?2, updated_at = ?2, version = version + 1
                    WHERE game_id = ?1 AND is_open IS NOT NULL
                    ",
            )
            .bind(&[js(game_id.as_str()), js_i64(now)])?)
        .await
    }

    async fn record_match(
        &self,
        game_id: &GameId,
        record: MatchRecord,
        now: i64,
    ) -> StorageResult<()> {
        run(self
            .db
            .prepare(
                "
                    INSERT INTO matches (
                        riot_match_id, game_id, guild_id, mode, queue_id, map_id,
                        riot_game_mode, riot_game_type, data_source, payload_json, finalized_at
                    )
                    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                    ON CONFLICT(riot_match_id) DO UPDATE SET
                        game_id = excluded.game_id,
                        guild_id = excluded.guild_id,
                        mode = excluded.mode,
                        queue_id = excluded.queue_id,
                        map_id = excluded.map_id,
                        riot_game_mode = excluded.riot_game_mode,
                        riot_game_type = excluded.riot_game_type,
                        data_source = excluded.data_source,
                        payload_json = excluded.payload_json,
                        finalized_at = excluded.finalized_at
                    ",
            )
            .bind(&[
                js(&record.riot_match_id),
                js(game_id.as_str()),
                js(&record.guild_id),
                js(record.mode.as_str()),
                opt_i64(record.queue_id),
                opt_i64(record.map_id),
                opt_js(record.riot_game_mode.as_deref()),
                opt_js(record.riot_game_type.as_deref()),
                js(&record.data_source),
                opt_js(record.payload_json.as_deref()),
                js_i64(now),
            ])?)
        .await?;

        run(self
            .db
            .prepare("DELETE FROM match_participants WHERE riot_match_id = ?1")
            .bind(&[js(&record.riot_match_id)])?)
        .await?;

        for participant in &record.participants {
            run(self
                .db
                .prepare(
                    "
                        INSERT INTO match_participants (
                            riot_match_id, puuid, discord_user_id, team, champion_id,
                            champion_name, win, kills, deaths, assists, total_damage,
                            gold_earned, total_minions, vision_score, raw_json
                        )
                        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
                        ",
                )
                .bind(&[
                    js(&record.riot_match_id),
                    js(&participant.puuid),
                    opt_js(participant.discord_user_id.as_deref()),
                    opt_js(participant.team.map(TeamSide::as_str)),
                    opt_i64(participant.champion_id),
                    opt_js(participant.champion_name.as_deref()),
                    opt_bool(participant.win),
                    opt_i64(participant.kills),
                    opt_i64(participant.deaths),
                    opt_i64(participant.assists),
                    opt_i64(participant.total_damage),
                    opt_i64(participant.gold_earned),
                    opt_i64(participant.total_minions),
                    opt_i64(participant.vision_score),
                    opt_js(participant.raw_json.as_deref()),
                ])?)
            .await?;
        }

        run(self
            .db
            .prepare(
                "
                    UPDATE games
                    SET queue_id = COALESCE(?2, queue_id),
                        map_id = COALESCE(?3, map_id),
                        riot_game_mode = COALESCE(?4, riot_game_mode),
                        riot_game_type = COALESCE(?5, riot_game_type),
                        riot_match_id = ?6,
                        updated_at = ?7
                    WHERE game_id = ?1
                    ",
            )
            .bind(&[
                js(game_id.as_str()),
                opt_i64(record.queue_id),
                opt_i64(record.map_id),
                opt_js(record.riot_game_mode.as_deref()),
                opt_js(record.riot_game_type.as_deref()),
                js(&record.riot_match_id),
                js_i64(now),
            ])?)
        .await
    }

    async fn stats_for_player(
        &self,
        guild_id: &str,
        discord_user_id: &str,
        mode: Option<GameModeKind>,
    ) -> StorageResult<Option<PlayerStatsRow>> {
        if let Some(mode) = mode {
            first(
                self.db
                    .prepare(
                        "
                        SELECT p.guild_id, p.discord_user_id, p.riot_game_name, p.riot_tag_line,
                               p.rating,
                               COALESCE(m.wins, 0) AS wins,
                               COALESCE(m.losses, 0) AS losses,
                               COALESCE(m.win_rate, 0.0) AS win_rate,
                               a.avg_kills,
                               a.avg_deaths,
                               a.avg_assists,
                               a.avg_total_damage
                        FROM players p
                        LEFT JOIN mode_leaderboard_view m
                          ON m.guild_id = p.guild_id
                         AND m.discord_user_id = p.discord_user_id
                         AND m.mode = ?3
                        LEFT JOIN (
                            SELECT m.guild_id,
                                   m.mode,
                                   mp.discord_user_id,
                                   AVG(mp.kills) AS avg_kills,
                                   AVG(mp.deaths) AS avg_deaths,
                                   AVG(mp.assists) AS avg_assists,
                                   AVG(mp.total_damage) AS avg_total_damage
                            FROM match_participants mp
                            JOIN matches m ON m.riot_match_id = mp.riot_match_id
                            GROUP BY m.guild_id, m.mode, mp.discord_user_id
                        ) a
                          ON a.guild_id = p.guild_id
                         AND a.discord_user_id = p.discord_user_id
                         AND a.mode = ?3
                        WHERE p.guild_id = ?1 AND p.discord_user_id = ?2
                        ",
                    )
                    .bind(&[js(guild_id), js(discord_user_id), js(mode.as_str())])?,
            )
            .await
        } else {
            first(
                self.db
                    .prepare(
                        "
                        SELECT p.guild_id, p.discord_user_id, p.riot_game_name, p.riot_tag_line,
                               p.rating, p.wins, p.losses, p.win_rate,
                               a.avg_kills,
                               a.avg_deaths,
                               a.avg_assists,
                               a.avg_total_damage
                        FROM player_record_view p
                        LEFT JOIN (
                            SELECT m.guild_id,
                                   mp.discord_user_id,
                                   AVG(mp.kills) AS avg_kills,
                                   AVG(mp.deaths) AS avg_deaths,
                                   AVG(mp.assists) AS avg_assists,
                                   AVG(mp.total_damage) AS avg_total_damage
                            FROM match_participants mp
                            JOIN matches m ON m.riot_match_id = mp.riot_match_id
                            GROUP BY m.guild_id, mp.discord_user_id
                        ) a
                          ON a.guild_id = p.guild_id
                         AND a.discord_user_id = p.discord_user_id
                        WHERE p.guild_id = ?1 AND p.discord_user_id = ?2
                        ",
                    )
                    .bind(&[js(guild_id), js(discord_user_id)])?,
            )
            .await
        }
    }

    async fn leaderboard(
        &self,
        guild_id: &str,
        mode: Option<GameModeKind>,
        limit: u8,
    ) -> StorageResult<Vec<LeaderboardRow>> {
        if let Some(mode) = mode {
            all(self
                .db
                .prepare(
                    "
                        SELECT p.discord_user_id, p.riot_game_name, p.riot_tag_line,
                               p.rating,
                               COALESCE(m.wins, 0) AS wins,
                               COALESCE(m.losses, 0) AS losses,
                               COALESCE(m.win_rate, 0.0) AS win_rate,
                               a.avg_kills,
                               a.avg_deaths,
                               a.avg_assists,
                               a.avg_total_damage
                        FROM players p
                        LEFT JOIN mode_leaderboard_view m
                          ON m.guild_id = p.guild_id
                         AND m.discord_user_id = p.discord_user_id
                         AND m.mode = ?2
                        LEFT JOIN (
                            SELECT m.guild_id,
                                   m.mode,
                                   mp.discord_user_id,
                                   AVG(mp.kills) AS avg_kills,
                                   AVG(mp.deaths) AS avg_deaths,
                                   AVG(mp.assists) AS avg_assists,
                                   AVG(mp.total_damage) AS avg_total_damage
                            FROM match_participants mp
                            JOIN matches m ON m.riot_match_id = mp.riot_match_id
                            GROUP BY m.guild_id, m.mode, mp.discord_user_id
                        ) a
                          ON a.guild_id = p.guild_id
                         AND a.discord_user_id = p.discord_user_id
                         AND a.mode = ?2
                        WHERE p.guild_id = ?1
                        ORDER BY p.rating DESC, wins DESC
                        LIMIT ?3
                        ",
                )
                .bind(&[js(guild_id), js(mode.as_str()), js_i64(i64::from(limit))])?)
            .await
        } else {
            all(self
                .db
                .prepare(
                    "
                        SELECT p.discord_user_id, p.riot_game_name, p.riot_tag_line,
                               p.rating, p.wins, p.losses, p.win_rate,
                               a.avg_kills,
                               a.avg_deaths,
                               a.avg_assists,
                               a.avg_total_damage
                        FROM leaderboard_view p
                        LEFT JOIN (
                            SELECT m.guild_id,
                                   mp.discord_user_id,
                                   AVG(mp.kills) AS avg_kills,
                                   AVG(mp.deaths) AS avg_deaths,
                                   AVG(mp.assists) AS avg_assists,
                                   AVG(mp.total_damage) AS avg_total_damage
                            FROM match_participants mp
                            JOIN matches m ON m.riot_match_id = mp.riot_match_id
                            GROUP BY m.guild_id, mp.discord_user_id
                        ) a
                          ON a.guild_id = p.guild_id
                         AND a.discord_user_id = p.discord_user_id
                        WHERE p.guild_id = ?1
                        LIMIT ?2
                        ",
                )
                .bind(&[js(guild_id), js_i64(i64::from(limit))])?)
            .await
        }
    }

    async fn active_games(&self) -> StorageResult<Vec<GameRow>> {
        all(self.db.prepare(
            "
            SELECT game_id, guild_id, channel_id, creator_discord_id, status, mode,
                   winning_side, version, riot_match_id, consecutive_404
            FROM games
            WHERE is_open IS NOT NULL
            ORDER BY created_at
            ",
        ))
        .await
    }

    async fn mark_ingame(
        &self,
        game_id: &GameId,
        update: LiveGameUpdate,
        now: i64,
    ) -> StorageResult<()> {
        run(self
            .db
            .prepare(
                "
                    UPDATE games
                    SET status = 'ingame',
                        started_at = COALESCE(started_at, ?2),
                        riot_match_id = COALESCE(?3, riot_match_id),
                        queue_id = COALESCE(?4, queue_id),
                        map_id = COALESCE(?5, map_id),
                        riot_game_mode = COALESCE(?6, riot_game_mode),
                        riot_game_type = COALESCE(?7, riot_game_type),
                        consecutive_404 = 0,
                        updated_at = ?2,
                        version = version + 1
                    WHERE game_id = ?1 AND status IN ('randomized','ingame')
                    ",
            )
            .bind(&[
                js(game_id.as_str()),
                js_i64(now),
                opt_js(update.riot_match_id.as_deref()),
                opt_i64(update.queue_id),
                opt_i64(update.map_id),
                opt_js(update.riot_game_mode.as_deref()),
                opt_js(update.riot_game_type.as_deref()),
            ])?)
        .await
    }

    async fn bump_404(&self, game_id: &GameId, now: i64) -> StorageResult<()> {
        run(self
            .db
            .prepare(
                "
                    UPDATE games
                    SET consecutive_404 = consecutive_404 + 1,
                        status = CASE
                            WHEN status = 'ingame' AND consecutive_404 + 1 >= 2 THEN 'reported'
                            ELSE status
                        END,
                        updated_at = ?2,
                        version = version + 1
                    WHERE game_id = ?1 AND is_open IS NOT NULL
                    ",
            )
            .bind(&[js(game_id.as_str()), js_i64(now)])?)
        .await
    }

    async fn emit_event(
        &self,
        guild_id: &str,
        game_id: Option<&str>,
        actor_id: Option<&str>,
        kind: &str,
        payload_json: &str,
        now: i64,
    ) -> StorageResult<()> {
        run(self
            .db
            .prepare(
                "
                    INSERT INTO events (guild_id, game_id, actor_id, kind, payload_json, created_at)
                    VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                    ",
            )
            .bind(&[
                js(guild_id),
                opt_js(game_id),
                opt_js(actor_id),
                js(kind),
                js(payload_json),
                js_i64(now),
            ])?)
        .await
    }
}

async fn run(statement: worker::D1PreparedStatement) -> StorageResult<()> {
    let result = statement.run().await.map_err(to_backend)?;
    if result.success() {
        Ok(())
    } else {
        Err(StorageError::Backend(
            result
                .error()
                .unwrap_or_else(|| "unknown D1 error".to_owned()),
        ))
    }
}

async fn first<T>(statement: worker::D1PreparedStatement) -> StorageResult<Option<T>>
where
    T: for<'de> serde::Deserialize<'de>,
{
    statement.first(None).await.map_err(to_backend)
}

async fn all<T>(statement: worker::D1PreparedStatement) -> StorageResult<Vec<T>>
where
    T: for<'de> serde::Deserialize<'de>,
{
    statement
        .all()
        .await
        .map_err(to_backend)?
        .results()
        .map_err(to_backend)
}

fn map_active_game_error(error: StorageError) -> StorageError {
    match error {
        StorageError::Backend(message) if message.contains("uniq_one_open_per_guild") => {
            StorageError::ActiveGameExists
        }
        other => other,
    }
}

fn to_backend(error: worker::Error) -> StorageError {
    StorageError::Backend(error.to_string())
}

impl From<worker::Error> for StorageError {
    fn from(error: worker::Error) -> Self {
        to_backend(error)
    }
}

fn js(value: &str) -> JsValue {
    JsValue::from_str(value)
}

fn opt_js(value: Option<&str>) -> JsValue {
    value.map_or(JsValue::NULL, JsValue::from_str)
}

fn js_i64(value: i64) -> JsValue {
    JsValue::from_f64(value as f64)
}

fn opt_i64(value: Option<i64>) -> JsValue {
    value.map_or(JsValue::NULL, js_i64)
}

fn opt_bool(value: Option<bool>) -> JsValue {
    value.map_or(JsValue::NULL, JsValue::from_bool)
}
