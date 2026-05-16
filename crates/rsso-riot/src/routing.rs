use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RiotRouteError {
    #[error("unknown riot regional route `{0}`")]
    UnknownRegional(String),
    #[error("unknown riot platform route `{0}`")]
    UnknownPlatform(String),
}

pub fn regional_host(route: &str) -> Result<&'static str, RiotRouteError> {
    match route.to_ascii_uppercase().as_str() {
        "AMERICAS" => Ok("americas.api.riotgames.com"),
        "ASIA" => Ok("asia.api.riotgames.com"),
        "EUROPE" => Ok("europe.api.riotgames.com"),
        "SEA" => Ok("sea.api.riotgames.com"),
        other => Err(RiotRouteError::UnknownRegional(other.to_owned())),
    }
}

pub fn platform_host(route: &str) -> Result<&'static str, RiotRouteError> {
    match route.to_ascii_uppercase().as_str() {
        "BR1" => Ok("br1.api.riotgames.com"),
        "EUN1" => Ok("eun1.api.riotgames.com"),
        "EUW1" => Ok("euw1.api.riotgames.com"),
        "JP1" => Ok("jp1.api.riotgames.com"),
        "KR" => Ok("kr.api.riotgames.com"),
        "LA1" => Ok("la1.api.riotgames.com"),
        "LA2" => Ok("la2.api.riotgames.com"),
        "NA1" => Ok("na1.api.riotgames.com"),
        "OC1" => Ok("oc1.api.riotgames.com"),
        "TR1" => Ok("tr1.api.riotgames.com"),
        "RU" => Ok("ru.api.riotgames.com"),
        "PH2" => Ok("ph2.api.riotgames.com"),
        "SG2" => Ok("sg2.api.riotgames.com"),
        "TH2" => Ok("th2.api.riotgames.com"),
        "TW2" => Ok("tw2.api.riotgames.com"),
        "VN2" => Ok("vn2.api.riotgames.com"),
        other => Err(RiotRouteError::UnknownPlatform(other.to_owned())),
    }
}

pub fn match_regional_route_for_platform(
    platform_route: &str,
) -> Result<&'static str, RiotRouteError> {
    match platform_route.to_ascii_uppercase().as_str() {
        "BR1" | "LA1" | "LA2" | "NA1" => Ok("AMERICAS"),
        "EUN1" | "EUW1" | "RU" | "TR1" => Ok("EUROPE"),
        "JP1" | "KR" => Ok("ASIA"),
        "OC1" | "PH2" | "SG2" | "TH2" | "TW2" | "VN2" => Ok("SEA"),
        other => Err(RiotRouteError::UnknownPlatform(other.to_owned())),
    }
}

pub fn account_by_riot_id_url(
    regional_route: &str,
    game_name: &str,
    tag_line: &str,
) -> Result<String, RiotRouteError> {
    let host = regional_host(regional_route)?;
    Ok(format!(
        "https://{host}/riot/account/v1/accounts/by-riot-id/{}/{}",
        urlencoding(game_name),
        urlencoding(tag_line)
    ))
}

pub fn match_ids_by_puuid_url(
    regional_route: &str,
    puuid: &str,
    start_time: Option<i64>,
    queue: Option<u16>,
    count: u8,
) -> Result<String, RiotRouteError> {
    let host = regional_host(regional_route)?;
    let mut url = format!(
        "https://{host}/lol/match/v5/matches/by-puuid/{}/ids?start=0&count={}",
        urlencoding(puuid),
        count
    );
    if let Some(start_time) = start_time {
        url.push_str("&startTime=");
        url.push_str(&start_time.to_string());
    }
    if let Some(queue) = queue {
        url.push_str("&queue=");
        url.push_str(&queue.to_string());
    }
    Ok(url)
}

pub fn match_detail_url(regional_route: &str, match_id: &str) -> Result<String, RiotRouteError> {
    let host = regional_host(regional_route)?;
    Ok(format!(
        "https://{host}/lol/match/v5/matches/{}",
        urlencoding(match_id)
    ))
}

pub fn active_game_url(platform_route: &str, puuid: &str) -> Result<String, RiotRouteError> {
    let host = platform_host(platform_route)?;
    Ok(format!(
        "https://{host}/lol/spectator/v5/active-games/by-summoner/{}",
        urlencoding(puuid)
    ))
}

fn urlencoding(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(char::from(byte))
            }
            _ => {
                encoded.push('%');
                encoded.push(char::from(HEX[usize::from(byte >> 4)]));
                encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
            }
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use crate::routing::{account_by_riot_id_url, match_regional_route_for_platform};

    #[test]
    fn path_encoding_uses_percent_spaces() {
        let url = account_by_riot_id_url("AMERICAS", "must trust a", "fart")
            .expect("valid regional route");
        assert!(url.ends_with("/must%20trust%20a/fart"));
    }

    #[test]
    fn maps_match_platform_to_regional_route() {
        assert_eq!(
            match_regional_route_for_platform("NA1").expect("known platform"),
            "AMERICAS"
        );
        assert_eq!(
            match_regional_route_for_platform("EUW1").expect("known platform"),
            "EUROPE"
        );
    }
}
