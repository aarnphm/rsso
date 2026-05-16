pub mod account;
pub mod match_v5;
pub mod routing;

pub use account::{parse_riot_id, AccountDto, RiotIdParts};
pub use match_v5::{
    CurrentGameInfoDto, CurrentGameParticipantDto, MatchDto, MatchInfoDto, ParticipantDto,
};
pub use routing::{
    match_regional_route_for_platform, platform_host, regional_host, RiotRouteError,
};
