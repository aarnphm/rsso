use crate::interaction::{ApplicationCommandData, CommandOption};
use rsso_domain::{
    normalize_riot_match_id, DiscordUserId, GameId, GameModeKind, MatchIdError, TeamSide,
};
use serde_json::Value;
use std::str::FromStr;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscordCommand {
    RegisterSummoners { riot_id: String },
    Create(CreateCommand),
    Game(GameCommand),
    Add(AddCommand),
    Randomize { game_id: GameId },
    Winner(WinnerCommand),
    Result { game_id: GameId, winner: TeamSide },
    Results(ResultsCommand),
    Finish(FinishCommand),
    Hydrate(HydrateCommand),
    LinkMatch(LinkMatchCommand),
    End { game_id: GameId },
    Status { game_id: Option<GameId> },
    Stats(StatsCommand),
    Leaderboards { mode: Option<GameModeKind> },
    Analysis { mode: Option<GameModeKind> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateCommand {
    pub mode: GameModeKind,
    pub users: Vec<DiscordUserId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameCommand {
    pub mode: GameModeKind,
    pub users: Vec<DiscordUserId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddCommand {
    pub game_id: Option<GameId>,
    pub users: Vec<DiscordUserId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WinnerCommand {
    pub game_id: GameId,
    pub winner: TeamSide,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinishCommand {
    pub riot_match_id: String,
    pub game_id: Option<GameId>,
    pub winner: Option<TeamSide>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResultsCommand {
    pub game_id: Option<GameId>,
    pub winner: Option<TeamSide>,
    pub riot_match_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HydrateCommand {
    pub game_id: Option<GameId>,
    pub riot_match_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkMatchCommand {
    pub game_id: Option<GameId>,
    pub riot_match_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatsCommand {
    pub user: Option<DiscordUserId>,
    pub name: Option<String>,
    pub mode: Option<GameModeKind>,
}

#[derive(Debug, Error)]
pub enum CommandError {
    #[error("missing command data")]
    MissingData,
    #[error("unknown command `{0}`")]
    UnknownCommand(String),
    #[error("missing option `{0}`")]
    MissingOption(&'static str),
    #[error("option `{0}` must be a string")]
    ExpectedString(&'static str),
    #[error("option `{0}` must be a Discord user")]
    ExpectedUser(&'static str),
    #[error("invalid option `{name}`: {reason}")]
    InvalidOption { name: &'static str, reason: String },
    #[error("`/create` needs an even roster between 2 and 10 users")]
    InvalidCreateRoster,
    #[error("`/game` needs between 2 and 10 users")]
    InvalidGameRoster,
    #[error("`/add` needs between 1 and 10 users")]
    InvalidAddRoster,
}

pub fn parse_command(data: &ApplicationCommandData) -> Result<DiscordCommand, CommandError> {
    match data.name.as_str() {
        "register-summoners" => Ok(DiscordCommand::RegisterSummoners {
            riot_id: string_option(&data.options, "riot_id")?.to_owned(),
        }),
        "create" => parse_create(data),
        "game" => parse_game(data),
        "add" => parse_add(data),
        "randomize" => Ok(DiscordCommand::Randomize {
            game_id: GameId::new(string_option(&data.options, "game_id")?),
        }),
        "winner" => Ok(DiscordCommand::Winner(WinnerCommand {
            game_id: GameId::new(string_option(&data.options, "game_id")?),
            winner: team_option(&data.options, "winner")?,
        })),
        "result" => Ok(DiscordCommand::Result {
            game_id: GameId::new(string_option(&data.options, "game_id")?),
            winner: team_option(&data.options, "winner")?,
        }),
        "results" => parse_results(data),
        "finish" => parse_finish(data),
        "hydrate" => parse_hydrate(data),
        "link-match" => parse_link_match(data),
        "end" => Ok(DiscordCommand::End {
            game_id: GameId::new(string_option(&data.options, "game_id")?),
        }),
        "status" => Ok(DiscordCommand::Status {
            game_id: optional_string_option(&data.options, "game_id")?.map(GameId::new),
        }),
        "stats" => parse_stats(data),
        "leaderboards" => Ok(DiscordCommand::Leaderboards {
            mode: optional_mode_option(&data.options, "mode")?,
        }),
        "analysis" => Ok(DiscordCommand::Analysis {
            mode: optional_mode_option(&data.options, "mode")?,
        }),
        other => Err(CommandError::UnknownCommand(other.to_owned())),
    }
}

fn parse_create(data: &ApplicationCommandData) -> Result<DiscordCommand, CommandError> {
    let mode = optional_mode_option(&data.options, "mode")?.unwrap_or(GameModeKind::AramMayhem);
    let users = positional_users(data)?;
    if !(2..=10).contains(&users.len()) || users.len() % 2 != 0 {
        return Err(CommandError::InvalidCreateRoster);
    }
    Ok(DiscordCommand::Create(CreateCommand { mode, users }))
}

fn parse_game(data: &ApplicationCommandData) -> Result<DiscordCommand, CommandError> {
    let mode = mode_option(&data.options, "mode")?;
    let users = positional_users(data)?;
    if !(2..=10).contains(&users.len()) {
        return Err(CommandError::InvalidGameRoster);
    }
    Ok(DiscordCommand::Game(GameCommand { mode, users }))
}

fn parse_add(data: &ApplicationCommandData) -> Result<DiscordCommand, CommandError> {
    let mut users = positional_users(data)?;
    if users.is_empty() {
        if let Some(user) = optional_user_option(&data.options, "user")? {
            users.push(user);
        }
    }
    if !(1..=10).contains(&users.len()) {
        return Err(CommandError::InvalidAddRoster);
    }
    Ok(DiscordCommand::Add(AddCommand {
        game_id: optional_string_option(&data.options, "game_id")?.map(GameId::new),
        users,
    }))
}

fn parse_stats(data: &ApplicationCommandData) -> Result<DiscordCommand, CommandError> {
    let user = optional_user_option(&data.options, "user")?;
    let name = optional_string_option(&data.options, "name")?
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned);
    if user.is_some() && name.is_some() {
        return Err(CommandError::InvalidOption {
            name: "name",
            reason: "use either `name` or `user`, not both".to_owned(),
        });
    }
    Ok(DiscordCommand::Stats(StatsCommand {
        user,
        name,
        mode: optional_mode_option(&data.options, "mode")?,
    }))
}

fn parse_finish(data: &ApplicationCommandData) -> Result<DiscordCommand, CommandError> {
    let riot_match_id = normalize_match_id_option(&data.options, "riot_match_id")?;
    Ok(DiscordCommand::Finish(FinishCommand {
        riot_match_id,
        game_id: optional_string_option(&data.options, "game_id")?.map(GameId::new),
        winner: optional_team_option(&data.options, "winner")?,
    }))
}

fn parse_results(data: &ApplicationCommandData) -> Result<DiscordCommand, CommandError> {
    let riot_match_id = optional_normalize_match_id_option(&data.options, "riot_match_id")?;
    Ok(DiscordCommand::Results(ResultsCommand {
        game_id: optional_string_option(&data.options, "game_id")?.map(GameId::new),
        winner: optional_team_option(&data.options, "winner")?,
        riot_match_id,
    }))
}

fn parse_hydrate(data: &ApplicationCommandData) -> Result<DiscordCommand, CommandError> {
    Ok(DiscordCommand::Hydrate(HydrateCommand {
        game_id: optional_string_option(&data.options, "game_id")?.map(GameId::new),
        riot_match_id: optional_normalize_match_id_option(&data.options, "riot_match_id")?,
    }))
}

fn parse_link_match(data: &ApplicationCommandData) -> Result<DiscordCommand, CommandError> {
    Ok(DiscordCommand::LinkMatch(LinkMatchCommand {
        game_id: optional_string_option(&data.options, "game_id")?.map(GameId::new),
        riot_match_id: normalize_match_id_option(&data.options, "riot_match_id")?,
    }))
}

fn positional_users(data: &ApplicationCommandData) -> Result<Vec<DiscordUserId>, CommandError> {
    data.options
        .iter()
        .filter(|option| option.name.starts_with("user_"))
        .map(|option| value_as_user(option, "user_n"))
        .collect()
}

fn normalize_match_id_option(
    options: &[CommandOption],
    name: &'static str,
) -> Result<String, CommandError> {
    normalize_match_id(
        string_option(options, name)?,
        optional_string_option(options, "region")?,
        name,
    )
}

fn optional_normalize_match_id_option(
    options: &[CommandOption],
    name: &'static str,
) -> Result<Option<String>, CommandError> {
    let region = optional_string_option(options, "region")?;
    let Some(value) = optional_string_option(options, name)? else {
        if region.is_some() {
            return Err(CommandError::InvalidOption {
                name: "region",
                reason: "`region` only applies when `riot_match_id` is set".to_owned(),
            });
        }
        return Ok(None);
    };
    normalize_match_id(value, region, name).map(Some)
}

fn normalize_match_id(
    value: &str,
    region: Option<&str>,
    name: &'static str,
) -> Result<String, CommandError> {
    normalize_riot_match_id(value, region).map_err(|err| CommandError::InvalidOption {
        name: match &err {
            MatchIdError::UnknownRegion(_) | MatchIdError::RegionMismatch { .. } => "region",
            _ => name,
        },
        reason: err.to_string(),
    })
}

fn string_option<'a>(
    options: &'a [CommandOption],
    name: &'static str,
) -> Result<&'a str, CommandError> {
    let option = find_option(options, name)?;
    value_as_str(option, name)
}

fn optional_string_option<'a>(
    options: &'a [CommandOption],
    name: &'static str,
) -> Result<Option<&'a str>, CommandError> {
    options
        .iter()
        .find(|option| option.name == name)
        .map(|option| value_as_str(option, name))
        .transpose()
}

fn optional_user_option(
    options: &[CommandOption],
    name: &'static str,
) -> Result<Option<DiscordUserId>, CommandError> {
    options
        .iter()
        .find(|option| option.name == name)
        .map(|option| value_as_user(option, name))
        .transpose()
}

fn mode_option(
    options: &[CommandOption],
    name: &'static str,
) -> Result<GameModeKind, CommandError> {
    parse_mode(string_option(options, name)?, name)
}

fn optional_mode_option(
    options: &[CommandOption],
    name: &'static str,
) -> Result<Option<GameModeKind>, CommandError> {
    optional_string_option(options, name)?
        .map(|value| parse_mode(value, name))
        .transpose()
}

fn team_option(options: &[CommandOption], name: &'static str) -> Result<TeamSide, CommandError> {
    parse_team(string_option(options, name)?, name)
}

fn optional_team_option(
    options: &[CommandOption],
    name: &'static str,
) -> Result<Option<TeamSide>, CommandError> {
    optional_string_option(options, name)?
        .map(|value| parse_team(value, name))
        .transpose()
}

fn find_option<'a>(
    options: &'a [CommandOption],
    name: &'static str,
) -> Result<&'a CommandOption, CommandError> {
    options
        .iter()
        .find(|option| option.name == name)
        .ok_or(CommandError::MissingOption(name))
}

fn value_as_str<'a>(
    option: &'a CommandOption,
    name: &'static str,
) -> Result<&'a str, CommandError> {
    option
        .value
        .as_ref()
        .and_then(Value::as_str)
        .ok_or(CommandError::ExpectedString(name))
}

fn value_as_user(
    option: &CommandOption,
    name: &'static str,
) -> Result<DiscordUserId, CommandError> {
    option
        .value
        .as_ref()
        .and_then(Value::as_str)
        .map(DiscordUserId::new)
        .ok_or(CommandError::ExpectedUser(name))
}

fn parse_mode(value: &str, name: &'static str) -> Result<GameModeKind, CommandError> {
    GameModeKind::from_str(value).map_err(|err| CommandError::InvalidOption {
        name,
        reason: err.to_string(),
    })
}

fn parse_team(value: &str, name: &'static str) -> Result<TeamSide, CommandError> {
    TeamSide::from_str(value).map_err(|err| CommandError::InvalidOption {
        name,
        reason: err.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use crate::command::{parse_command, DiscordCommand};
    use crate::interaction::{ApplicationCommandData, CommandOption};
    use rsso_domain::{GameModeKind, TeamSide};
    use serde_json::json;

    #[test]
    fn parses_game_users() {
        let data = ApplicationCommandData {
            name: "game".to_owned(),
            options: vec![
                CommandOption {
                    name: "mode".to_owned(),
                    value: Some(json!("aram")),
                    options: vec![],
                },
                CommandOption {
                    name: "user_1".to_owned(),
                    value: Some(json!("1")),
                    options: vec![],
                },
                CommandOption {
                    name: "user_2".to_owned(),
                    value: Some(json!("2")),
                    options: vec![],
                },
            ],
            resolved: None,
        };
        let parsed = parse_command(&data).expect("valid game command");
        let DiscordCommand::Game(command) = parsed else {
            panic!("expected game");
        };
        assert_eq!(command.mode, GameModeKind::Aram);
        assert_eq!(command.users.len(), 2);
    }

    #[test]
    fn parses_create_users() {
        let data = ApplicationCommandData {
            name: "create".to_owned(),
            options: vec![
                CommandOption {
                    name: "user_1".to_owned(),
                    value: Some(json!("1")),
                    options: vec![],
                },
                CommandOption {
                    name: "user_2".to_owned(),
                    value: Some(json!("2")),
                    options: vec![],
                },
                CommandOption {
                    name: "user_3".to_owned(),
                    value: Some(json!("3")),
                    options: vec![],
                },
                CommandOption {
                    name: "user_4".to_owned(),
                    value: Some(json!("4")),
                    options: vec![],
                },
            ],
            resolved: None,
        };
        let parsed = parse_command(&data).expect("valid create command");
        let DiscordCommand::Create(command) = parsed else {
            panic!("expected create");
        };
        assert_eq!(command.mode, GameModeKind::AramMayhem);
        assert_eq!(command.users.len(), 4);
    }

    #[test]
    fn parses_winner() {
        let data = ApplicationCommandData {
            name: "winner".to_owned(),
            options: vec![
                CommandOption {
                    name: "game_id".to_owned(),
                    value: Some(json!("1283")),
                    options: vec![],
                },
                CommandOption {
                    name: "winner".to_owned(),
                    value: Some(json!("red")),
                    options: vec![],
                },
            ],
            resolved: None,
        };
        let parsed = parse_command(&data).expect("valid winner command");
        let DiscordCommand::Winner(command) = parsed else {
            panic!("expected winner");
        };
        assert_eq!(command.game_id.as_str(), "1283");
        assert_eq!(command.winner, TeamSide::Red);
    }

    #[test]
    fn parses_finish_optional_winner() {
        let data = ApplicationCommandData {
            name: "finish".to_owned(),
            options: vec![
                CommandOption {
                    name: "riot_match_id".to_owned(),
                    value: Some(json!("NA1_4901234567")),
                    options: vec![],
                },
                CommandOption {
                    name: "winner".to_owned(),
                    value: Some(json!("blue")),
                    options: vec![],
                },
            ],
            resolved: None,
        };
        let parsed = parse_command(&data).expect("valid finish command");
        let DiscordCommand::Finish(command) = parsed else {
            panic!("expected finish");
        };
        assert_eq!(command.winner, Some(TeamSide::Blue));
    }

    #[test]
    fn parses_multi_add_without_game_id() {
        let data = ApplicationCommandData {
            name: "add".to_owned(),
            options: vec![
                CommandOption {
                    name: "user_1".to_owned(),
                    value: Some(json!("1")),
                    options: vec![],
                },
                CommandOption {
                    name: "user_2".to_owned(),
                    value: Some(json!("2")),
                    options: vec![],
                },
            ],
            resolved: None,
        };
        let parsed = parse_command(&data).expect("valid add command");
        let DiscordCommand::Add(command) = parsed else {
            panic!("expected add");
        };
        assert_eq!(command.game_id, None);
        assert_eq!(command.users.len(), 2);
    }

    #[test]
    fn parses_stats_name() {
        let data = ApplicationCommandData {
            name: "stats".to_owned(),
            options: vec![CommandOption {
                name: "name".to_owned(),
                value: Some(json!("Cyracen")),
                options: vec![],
            }],
            resolved: None,
        };
        let parsed = parse_command(&data).expect("valid stats command");
        let DiscordCommand::Stats(command) = parsed else {
            panic!("expected stats");
        };
        assert_eq!(command.name, Some("Cyracen".to_owned()));
        assert_eq!(command.user, None);
    }

    #[test]
    fn parses_finish_numeric_game_id_with_region() {
        let data = ApplicationCommandData {
            name: "finish".to_owned(),
            options: vec![
                CommandOption {
                    name: "riot_match_id".to_owned(),
                    value: Some(json!("5561312307")),
                    options: vec![],
                },
                CommandOption {
                    name: "region".to_owned(),
                    value: Some(json!("NA")),
                    options: vec![],
                },
            ],
            resolved: None,
        };
        let parsed = parse_command(&data).expect("valid finish command");
        let DiscordCommand::Finish(command) = parsed else {
            panic!("expected finish");
        };
        assert_eq!(command.riot_match_id, "NA1_5561312307");
    }

    #[test]
    fn parses_results_with_optional_match_id() {
        let data = ApplicationCommandData {
            name: "results".to_owned(),
            options: vec![
                CommandOption {
                    name: "game_id".to_owned(),
                    value: Some(json!("g_123")),
                    options: vec![],
                },
                CommandOption {
                    name: "riot_match_id".to_owned(),
                    value: Some(json!("NA1_4901234567")),
                    options: vec![],
                },
            ],
            resolved: None,
        };
        let parsed = parse_command(&data).expect("valid results command");
        let DiscordCommand::Results(command) = parsed else {
            panic!("expected results");
        };
        assert_eq!(
            command.game_id.map(|id| id.into_inner()),
            Some("g_123".to_owned())
        );
        assert_eq!(command.winner, None);
        assert_eq!(command.riot_match_id, Some("NA1_4901234567".to_owned()));
    }

    #[test]
    fn parses_hydrate_numeric_match_id() {
        let data = ApplicationCommandData {
            name: "hydrate".to_owned(),
            options: vec![CommandOption {
                name: "riot_match_id".to_owned(),
                value: Some(json!("5561726994")),
                options: vec![],
            }],
            resolved: None,
        };
        let parsed = parse_command(&data).expect("valid hydrate command");
        let DiscordCommand::Hydrate(command) = parsed else {
            panic!("expected hydrate");
        };
        assert_eq!(command.riot_match_id, Some("NA1_5561726994".to_owned()));
    }

    #[test]
    fn parses_link_match_numeric_match_id() {
        let data = ApplicationCommandData {
            name: "link-match".to_owned(),
            options: vec![
                CommandOption {
                    name: "riot_match_id".to_owned(),
                    value: Some(json!("5561727000")),
                    options: vec![],
                },
                CommandOption {
                    name: "game_id".to_owned(),
                    value: Some(json!("g_session")),
                    options: vec![],
                },
            ],
            resolved: None,
        };
        let parsed = parse_command(&data).expect("valid link-match command");
        let DiscordCommand::LinkMatch(command) = parsed else {
            panic!("expected link-match");
        };
        assert_eq!(
            command.game_id.map(|id| id.into_inner()),
            Some("g_session".to_owned())
        );
        assert_eq!(command.riot_match_id, "NA1_5561727000");
    }
}
