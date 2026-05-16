use crate::interaction::{ApplicationCommandData, CommandOption};
use rsso_domain::{
    normalize_riot_match_id, DiscordUserId, GameId, GameModeKind, MatchIdError, TeamSide,
};
use serde_json::Value;
use std::str::FromStr;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscordCommand {
    RegisterSummoners {
        riot_id: String,
    },
    Game(GameCommand),
    Add {
        game_id: GameId,
        user: DiscordUserId,
    },
    Randomize {
        game_id: GameId,
    },
    Result {
        game_id: GameId,
        winner: TeamSide,
    },
    Results(ResultsCommand),
    Finish(FinishCommand),
    End {
        game_id: GameId,
    },
    Status {
        game_id: Option<GameId>,
    },
    Stats {
        user: Option<DiscordUserId>,
        mode: Option<GameModeKind>,
    },
    Leaderboards {
        mode: Option<GameModeKind>,
    },
    Analysis {
        mode: Option<GameModeKind>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameCommand {
    pub mode: GameModeKind,
    pub users: Vec<DiscordUserId>,
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
    #[error("`/game` needs between 2 and 10 users")]
    InvalidGameRoster,
}

pub fn parse_command(data: &ApplicationCommandData) -> Result<DiscordCommand, CommandError> {
    match data.name.as_str() {
        "register-summoners" => Ok(DiscordCommand::RegisterSummoners {
            riot_id: string_option(&data.options, "riot_id")?.to_owned(),
        }),
        "game" => parse_game(data),
        "add" => Ok(DiscordCommand::Add {
            game_id: GameId::new(string_option(&data.options, "game_id")?),
            user: user_option(&data.options, "user")?,
        }),
        "randomize" => Ok(DiscordCommand::Randomize {
            game_id: GameId::new(string_option(&data.options, "game_id")?),
        }),
        "result" => Ok(DiscordCommand::Result {
            game_id: GameId::new(string_option(&data.options, "game_id")?),
            winner: team_option(&data.options, "winner")?,
        }),
        "results" => parse_results(data),
        "finish" => parse_finish(data),
        "end" => Ok(DiscordCommand::End {
            game_id: GameId::new(string_option(&data.options, "game_id")?),
        }),
        "status" => Ok(DiscordCommand::Status {
            game_id: optional_string_option(&data.options, "game_id")?.map(GameId::new),
        }),
        "stats" => Ok(DiscordCommand::Stats {
            user: optional_user_option(&data.options, "user")?,
            mode: optional_mode_option(&data.options, "mode")?,
        }),
        "leaderboards" => Ok(DiscordCommand::Leaderboards {
            mode: optional_mode_option(&data.options, "mode")?,
        }),
        "analysis" => Ok(DiscordCommand::Analysis {
            mode: optional_mode_option(&data.options, "mode")?,
        }),
        other => Err(CommandError::UnknownCommand(other.to_owned())),
    }
}

fn parse_game(data: &ApplicationCommandData) -> Result<DiscordCommand, CommandError> {
    let mode = mode_option(&data.options, "mode")?;
    let users = data
        .options
        .iter()
        .filter(|option| option.name.starts_with("user_"))
        .map(|option| value_as_user(option, "user_n"))
        .collect::<Result<Vec<_>, _>>()?;
    if !(2..=10).contains(&users.len()) {
        return Err(CommandError::InvalidGameRoster);
    }
    Ok(DiscordCommand::Game(GameCommand { mode, users }))
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

fn user_option(
    options: &[CommandOption],
    name: &'static str,
) -> Result<DiscordUserId, CommandError> {
    let option = find_option(options, name)?;
    value_as_user(option, name)
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
}
