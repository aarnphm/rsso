use crate::models::{
    GameRow, LeaderboardRow, LiveGameUpdate, MatchRecord, NewGame, NewPlayer, PlayerRow,
    PlayerStatsRow, RosterPlayer,
};
use async_trait::async_trait;
use rsso_domain::{GameId, GameModeKind, TeamAssignment, TeamSide};
use thiserror::Error;

pub type StorageResult<T> = Result<T, StorageError>;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("storage backend error: {0}")]
    Backend(String),
    #[error("not found")]
    NotFound,
    #[error("an active game already exists for this guild")]
    ActiveGameExists,
    #[error("game transition conflicted with another writer")]
    Conflict,
    #[error("invalid storage row: {0}")]
    InvalidRow(String),
}

#[async_trait(?Send)]
pub trait Storage {
    async fn upsert_player(&self, player: NewPlayer) -> StorageResult<()>;
    async fn get_player(&self, guild_id: &str, discord_user_id: &str) -> StorageResult<PlayerRow>;
    async fn create_game(&self, game: NewGame, users: &[String]) -> StorageResult<()>;
    async fn add_player(
        &self,
        game_id: &GameId,
        guild_id: &str,
        discord_user_id: &str,
        now: i64,
    ) -> StorageResult<()>;
    async fn open_game_for_guild(&self, guild_id: &str) -> StorageResult<Option<GameRow>>;
    async fn latest_game_with_match_for_guild(
        &self,
        guild_id: &str,
    ) -> StorageResult<Option<GameRow>>;
    async fn game_by_riot_match_id(
        &self,
        guild_id: &str,
        riot_match_id: &str,
    ) -> StorageResult<Option<GameRow>>;
    async fn game_by_id(&self, game_id: &GameId) -> StorageResult<GameRow>;
    async fn roster(&self, game_id: &GameId) -> StorageResult<Vec<RosterPlayer>>;
    async fn assign_teams(
        &self,
        game_id: &GameId,
        assignments: &[TeamAssignment],
        now: i64,
    ) -> StorageResult<()>;
    async fn record_vote(
        &self,
        game_id: &GameId,
        discord_user_id: &str,
        winner: TeamSide,
        now: i64,
    ) -> StorageResult<()>;
    async fn mark_reported(
        &self,
        game_id: &GameId,
        winner: TeamSide,
        now: i64,
    ) -> StorageResult<()>;
    async fn finalize_game(
        &self,
        game_id: &GameId,
        winner: TeamSide,
        riot_match_id: Option<&str>,
        now: i64,
    ) -> StorageResult<()>;
    async fn record_match(
        &self,
        game_id: &GameId,
        record: MatchRecord,
        now: i64,
    ) -> StorageResult<()>;
    async fn cancel_game(&self, game_id: &GameId, now: i64) -> StorageResult<()>;
    async fn stats_for_player(
        &self,
        guild_id: &str,
        discord_user_id: &str,
        mode: Option<GameModeKind>,
    ) -> StorageResult<Option<PlayerStatsRow>>;
    async fn leaderboard(
        &self,
        guild_id: &str,
        mode: Option<GameModeKind>,
        limit: u8,
    ) -> StorageResult<Vec<LeaderboardRow>>;
    async fn active_games(&self) -> StorageResult<Vec<GameRow>>;
    async fn mark_ingame(
        &self,
        game_id: &GameId,
        update: LiveGameUpdate,
        now: i64,
    ) -> StorageResult<()>;
    async fn bump_404(&self, game_id: &GameId, now: i64) -> StorageResult<()>;
    async fn emit_event(
        &self,
        guild_id: &str,
        game_id: Option<&str>,
        actor_id: Option<&str>,
        kind: &str,
        payload_json: &str,
        now: i64,
    ) -> StorageResult<()>;
}
