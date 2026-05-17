use serde::Serialize;

pub const EPHEMERAL_FLAG: u64 = 1 << 6;

#[derive(Debug, Clone, Serialize)]
pub struct InteractionResponse {
    #[serde(rename = "type")]
    pub kind: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<InteractionResponseData>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InteractionResponseData {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flags: Option<u64>,
}

pub fn pong_response() -> InteractionResponse {
    InteractionResponse {
        kind: 1,
        data: None,
    }
}

pub fn deferred_response(ephemeral: bool) -> InteractionResponse {
    InteractionResponse {
        kind: 5,
        data: Some(InteractionResponseData {
            content: None,
            flags: ephemeral.then_some(EPHEMERAL_FLAG),
        }),
    }
}

pub fn deferred_update_response() -> InteractionResponse {
    InteractionResponse {
        kind: 6,
        data: None,
    }
}

pub fn message_response(content: impl Into<String>, ephemeral: bool) -> InteractionResponse {
    InteractionResponse {
        kind: 4,
        data: Some(InteractionResponseData {
            content: Some(content.into()),
            flags: ephemeral.then_some(EPHEMERAL_FLAG),
        }),
    }
}
