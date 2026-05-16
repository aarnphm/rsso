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
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}
