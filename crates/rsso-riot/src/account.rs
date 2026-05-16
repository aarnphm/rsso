use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RiotIdParts {
    pub game_name: String,
    pub tag_line: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RiotIdError {
    #[error("riot id must look like GameName#TAG")]
    MissingTag,
    #[error("riot id has an empty name or tag segment")]
    EmptySegment,
}

pub fn parse_riot_id(value: &str) -> Result<RiotIdParts, RiotIdError> {
    let (game_name, tag_line) = value.split_once('#').ok_or(RiotIdError::MissingTag)?;
    if game_name.trim().is_empty() || tag_line.trim().is_empty() {
        return Err(RiotIdError::EmptySegment);
    }
    Ok(RiotIdParts {
        game_name: game_name.trim().to_owned(),
        tag_line: tag_line.trim().to_owned(),
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountDto {
    pub puuid: String,
    pub game_name: String,
    pub tag_line: String,
}

#[cfg(test)]
mod tests {
    use crate::account::parse_riot_id;

    #[test]
    fn parses_riot_id() {
        let parsed = parse_riot_id("Faker#KR1").expect("valid riot id");
        assert_eq!(parsed.game_name, "Faker");
        assert_eq!(parsed.tag_line, "KR1");
    }
}
