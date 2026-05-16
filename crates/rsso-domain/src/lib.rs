pub mod elo;
pub mod ids;
pub mod match_id;
pub mod modes;
pub mod shuffle;
pub mod state;
pub mod stats;

pub use elo::{expected_score, rating_delta, TeamRating};
pub use ids::{
    ChannelId, DiscordUserId, GameId, GuildId, Puuid, RiotMatchId, RiotPlatform, RiotRegional,
    RoundId,
};
pub use match_id::{parse_riot_match_id, MatchIdError};
pub use modes::{GameModeKind, QueueId};
pub use shuffle::{fisher_yates, split_even_teams, Rng, ShuffleError, TeamAssignment};
pub use state::{GameStatus, StateError, TeamSide};
