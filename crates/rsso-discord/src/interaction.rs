use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "u8", into = "u8")]
pub enum InteractionType {
    Ping,
    ApplicationCommand,
    MessageComponent,
    Other(u8),
}

impl From<u8> for InteractionType {
    fn from(value: u8) -> Self {
        match value {
            1 => Self::Ping,
            2 => Self::ApplicationCommand,
            3 => Self::MessageComponent,
            other => Self::Other(other),
        }
    }
}

impl From<InteractionType> for u8 {
    fn from(value: InteractionType) -> Self {
        match value {
            InteractionType::Ping => 1,
            InteractionType::ApplicationCommand => 2,
            InteractionType::MessageComponent => 3,
            InteractionType::Other(other) => other,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Interaction {
    pub id: String,
    #[serde(default)]
    pub application_id: String,
    #[serde(rename = "type")]
    pub kind: InteractionType,
    #[serde(default)]
    pub token: String,
    #[serde(default)]
    pub guild_id: Option<String>,
    #[serde(default)]
    pub channel_id: Option<String>,
    #[serde(default)]
    pub member: Option<Member>,
    #[serde(default)]
    pub user: Option<User>,
    #[serde(default)]
    pub data: Option<ApplicationCommandData>,
}

impl Interaction {
    pub fn actor_user_id(&self) -> Option<&str> {
        self.member
            .as_ref()
            .map(|member| member.user.id.as_str())
            .or_else(|| self.user.as_ref().map(|user| user.id.as_str()))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Member {
    pub user: User,
    #[serde(default)]
    pub permissions: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    #[serde(default)]
    pub username: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplicationCommandData {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub options: Vec<CommandOption>,
    #[serde(default)]
    pub custom_id: Option<String>,
    #[serde(default)]
    pub component_type: Option<u8>,
    #[serde(default)]
    pub resolved: Option<ResolvedData>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandOption {
    pub name: String,
    #[serde(default)]
    pub value: Option<Value>,
    #[serde(default)]
    pub options: Vec<CommandOption>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedData {
    #[serde(default)]
    pub users: BTreeMap<String, User>,
}
