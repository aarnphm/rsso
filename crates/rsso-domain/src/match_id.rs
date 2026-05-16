use crate::ids::RiotMatchId;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedRiotMatchId {
    pub platform: String,
    pub numeric_id: String,
}

#[derive(Debug, Error)]
pub enum MatchIdError {
    #[error("riot match id must look like NA1_4901234567")]
    InvalidShape,
    #[error("riot match id has an empty platform or numeric segment")]
    EmptySegment,
    #[error("riot match id numeric segment must contain only digits")]
    NonNumeric,
}

pub fn parse_riot_match_id(value: &str) -> Result<ParsedRiotMatchId, MatchIdError> {
    let (platform, numeric_id) = value.split_once('_').ok_or(MatchIdError::InvalidShape)?;
    if platform.is_empty() || numeric_id.is_empty() {
        return Err(MatchIdError::EmptySegment);
    }
    if !platform.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(MatchIdError::InvalidShape);
    }
    if !numeric_id.chars().all(|c| c.is_ascii_digit()) {
        return Err(MatchIdError::NonNumeric);
    }
    Ok(ParsedRiotMatchId {
        platform: platform.to_owned(),
        numeric_id: numeric_id.to_owned(),
    })
}

impl RiotMatchId {
    pub fn parse(value: &str) -> Result<Self, MatchIdError> {
        parse_riot_match_id(value)?;
        Ok(Self::new(value))
    }
}

#[cfg(test)]
mod tests {
    use crate::match_id::parse_riot_match_id;

    #[test]
    fn accepts_platform_numeric_shape() {
        let parsed = parse_riot_match_id("NA1_4901234567").expect("valid match id");
        assert_eq!(parsed.platform, "NA1");
        assert_eq!(parsed.numeric_id, "4901234567");
    }

    #[test]
    fn rejects_slop() {
        assert!(parse_riot_match_id("NA1-nope").is_err());
        assert!(parse_riot_match_id("NA1_abc").is_err());
    }
}
