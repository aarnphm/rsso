use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchDto {
    pub metadata: MatchMetadataDto,
    pub info: MatchInfoDto,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchMetadataDto {
    pub match_id: String,
    pub participants: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchInfoDto {
    pub game_creation: Option<i64>,
    pub game_start_timestamp: Option<i64>,
    pub game_end_timestamp: Option<i64>,
    pub queue_id: Option<u16>,
    pub map_id: Option<u16>,
    pub game_mode: Option<String>,
    pub game_type: Option<String>,
    pub participants: Vec<ParticipantDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParticipantDto {
    pub puuid: String,
    pub summoner_id: Option<String>,
    pub riot_id_game_name: Option<String>,
    pub riot_id_tagline: Option<String>,
    pub team_id: Option<u16>,
    pub champion_id: Option<u16>,
    pub champion_name: Option<String>,
    pub win: Option<bool>,
    pub kills: Option<u16>,
    pub deaths: Option<u16>,
    pub assists: Option<u16>,
    pub total_damage_dealt_to_champions: Option<u32>,
    pub gold_earned: Option<u32>,
    pub total_minions_killed: Option<u32>,
    pub neutral_minions_killed: Option<u32>,
    pub vision_score: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentGameInfoDto {
    pub game_id: i64,
    pub game_type: Option<String>,
    pub game_start_time: Option<i64>,
    pub map_id: Option<u16>,
    pub game_length: Option<i64>,
    pub platform_id: Option<String>,
    pub game_mode: Option<String>,
    pub game_queue_config_id: Option<u16>,
    #[serde(default)]
    pub participants: Vec<CurrentGameParticipantDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentGameParticipantDto {
    pub puuid: Option<String>,
    pub team_id: Option<u16>,
    pub champion_id: Option<u16>,
    pub summoner_name: Option<String>,
}
