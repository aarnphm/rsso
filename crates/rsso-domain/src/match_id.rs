use crate::ids::RiotMatchId;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedRiotMatchId {
    pub platform: String,
    pub numeric_id: String,
}

#[derive(Debug, Error)]
pub enum MatchIdError {
    #[error("riot match id must look like NA1_4901234567 or a numeric game id")]
    InvalidShape,
    #[error("riot match id has an empty platform or numeric segment")]
    EmptySegment,
    #[error("riot match id numeric segment must contain only digits")]
    NonNumeric,
    #[error("unknown riot region `{0}`")]
    UnknownRegion(String),
    #[error("riot region `{region}` resolves to `{resolved}`, but match id uses `{platform}`")]
    RegionMismatch {
        region: String,
        resolved: String,
        platform: String,
    },
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

pub fn normalize_riot_match_id(value: &str, region: Option<&str>) -> Result<String, MatchIdError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(MatchIdError::InvalidShape);
    }
    if value.contains('_') {
        let parsed = parse_riot_match_id(value)?;
        let platform = normalize_riot_platform(&parsed.platform)?;
        if let Some(region) = region {
            let resolved = normalize_riot_platform(region)?;
            if resolved != platform {
                return Err(MatchIdError::RegionMismatch {
                    region: region.to_owned(),
                    resolved,
                    platform,
                });
            }
        }
        return Ok(format!("{platform}_{}", parsed.numeric_id));
    }
    if !value.chars().all(|c| c.is_ascii_digit()) {
        return Err(MatchIdError::NonNumeric);
    }
    let platform = normalize_riot_platform(region.unwrap_or("NA"))?;
    Ok(format!("{platform}_{value}"))
}

pub fn normalize_riot_platform(value: &str) -> Result<String, MatchIdError> {
    let normalized = match value.trim().to_ascii_uppercase().as_str() {
        "BR" | "BR1" => "BR1",
        "EUNE" | "EUN" | "EUN1" => "EUN1",
        "EUW" | "EUW1" => "EUW1",
        "JP" | "JP1" => "JP1",
        "KR" => "KR",
        "LAN" | "LA1" => "LA1",
        "LAS" | "LA2" => "LA2",
        "NA" | "NA1" => "NA1",
        "OC" | "OCE" | "OC1" => "OC1",
        "TR" | "TR1" => "TR1",
        "RU" => "RU",
        "PH" | "PH2" => "PH2",
        "SG" | "SG2" => "SG2",
        "TH" | "TH2" => "TH2",
        "TW" | "TW2" => "TW2",
        "VN" | "VN2" => "VN2",
        other => return Err(MatchIdError::UnknownRegion(other.to_owned())),
    };
    Ok(normalized.to_owned())
}

impl RiotMatchId {
    pub fn parse(value: &str) -> Result<Self, MatchIdError> {
        parse_riot_match_id(value)?;
        Ok(Self::new(value))
    }
}

#[cfg(test)]
mod tests {
    use crate::match_id::{normalize_riot_match_id, parse_riot_match_id};

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

    #[test]
    fn normalizes_numeric_game_id_with_default_na_region() {
        let normalized = normalize_riot_match_id("5561312307", None).expect("valid game id");
        assert_eq!(normalized, "NA1_5561312307");
    }

    #[test]
    fn normalizes_numeric_game_id_with_region_alias() {
        let normalized = normalize_riot_match_id("5561312307", Some("EUW")).expect("valid game id");
        assert_eq!(normalized, "EUW1_5561312307");
    }

    #[test]
    fn rejects_region_mismatch_for_full_match_id() {
        assert!(normalize_riot_match_id("NA1_5561312307", Some("EUW")).is_err());
    }
}
