use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueId(pub u16);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GameModeKind {
    Rift,
    Aram,
    AramMayhem,
    Other,
}

impl GameModeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rift => "rift",
            Self::Aram => "aram",
            Self::AramMayhem => "aram_mayhem",
            Self::Other => "other",
        }
    }

    pub fn expected_queue(self) -> Option<QueueId> {
        match self {
            Self::Rift | Self::Other => None,
            Self::Aram => Some(QueueId(450)),
            Self::AramMayhem => Some(QueueId(2400)),
        }
    }

    pub fn accepts_queue(self, queue_id: Option<QueueId>) -> bool {
        match (self, self.expected_queue(), queue_id) {
            (_, _, Some(QueueId(0))) => true,
            (Self::Rift | Self::Other, _, _) => true,
            (_, Some(expected), Some(actual)) => expected == actual,
            _ => false,
        }
    }
}

impl std::str::FromStr for GameModeKind {
    type Err = ModeError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "rift" => Ok(Self::Rift),
            "aram" => Ok(Self::Aram),
            "aram_mayhem" | "aram-mayhem" | "mayhem" => Ok(Self::AramMayhem),
            "other" => Ok(Self::Other),
            _ => Err(ModeError::Unknown(value.to_owned())),
        }
    }
}

#[derive(Debug, Error)]
pub enum ModeError {
    #[error("unknown game mode `{0}`")]
    Unknown(String),
}

#[cfg(test)]
mod tests {
    use crate::modes::{GameModeKind, QueueId};

    #[test]
    fn knows_aram_queues() {
        assert!(GameModeKind::Aram.accepts_queue(Some(QueueId(450))));
        assert!(GameModeKind::AramMayhem.accepts_queue(Some(QueueId(2400))));
        assert!(GameModeKind::Aram.accepts_queue(Some(QueueId(0))));
        assert!(!GameModeKind::Aram.accepts_queue(Some(QueueId(2400))));
    }
}
