use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TeamSide {
    Blue,
    Red,
}

impl TeamSide {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Blue => "blue",
            Self::Red => "red",
        }
    }

    pub fn opponent(self) -> Self {
        match self {
            Self::Blue => Self::Red,
            Self::Red => Self::Blue,
        }
    }
}

impl std::str::FromStr for TeamSide {
    type Err = StateError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "blue" => Ok(Self::Blue),
            "red" => Ok(Self::Red),
            _ => Err(StateError::InvalidTeam(value.to_owned())),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GameStatus {
    Lobby,
    Randomized,
    Ingame,
    Reported,
    Finalized,
    Cancelled,
    Ambiguous,
}

impl GameStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Lobby => "lobby",
            Self::Randomized => "randomized",
            Self::Ingame => "ingame",
            Self::Reported => "reported",
            Self::Finalized => "finalized",
            Self::Cancelled => "cancelled",
            Self::Ambiguous => "ambiguous",
        }
    }

    pub fn is_open(self) -> bool {
        matches!(
            self,
            Self::Lobby | Self::Randomized | Self::Ingame | Self::Reported | Self::Ambiguous
        )
    }

    pub fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::Lobby, Self::Randomized)
                | (Self::Lobby, Self::Cancelled)
                | (Self::Randomized, Self::Ingame)
                | (Self::Randomized, Self::Reported)
                | (Self::Randomized, Self::Finalized)
                | (Self::Randomized, Self::Cancelled)
                | (Self::Ingame, Self::Reported)
                | (Self::Ingame, Self::Ambiguous)
                | (Self::Ingame, Self::Finalized)
                | (Self::Reported, Self::Finalized)
                | (Self::Reported, Self::Cancelled)
                | (Self::Ambiguous, Self::Finalized)
                | (Self::Ambiguous, Self::Cancelled)
        )
    }
}

impl std::str::FromStr for GameStatus {
    type Err = StateError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "lobby" => Ok(Self::Lobby),
            "randomized" => Ok(Self::Randomized),
            "ingame" => Ok(Self::Ingame),
            "reported" => Ok(Self::Reported),
            "finalized" => Ok(Self::Finalized),
            "cancelled" => Ok(Self::Cancelled),
            "ambiguous" => Ok(Self::Ambiguous),
            _ => Err(StateError::InvalidStatus(value.to_owned())),
        }
    }
}

#[derive(Debug, Error)]
pub enum StateError {
    #[error("invalid game status `{0}`")]
    InvalidStatus(String),
    #[error("invalid team side `{0}`")]
    InvalidTeam(String),
    #[error("cannot move game from {from:?} to {to:?}")]
    InvalidTransition { from: GameStatus, to: GameStatus },
}

pub fn ensure_transition(from: GameStatus, to: GameStatus) -> Result<(), StateError> {
    if from.can_transition_to(to) {
        Ok(())
    } else {
        Err(StateError::InvalidTransition { from, to })
    }
}

#[cfg(test)]
mod tests {
    use crate::state::{ensure_transition, GameStatus};

    #[test]
    fn permits_planned_transitions() {
        ensure_transition(GameStatus::Lobby, GameStatus::Randomized).expect("lobby randomizes");
        ensure_transition(GameStatus::Randomized, GameStatus::Finalized)
            .expect("finish can short-circuit");
    }

    #[test]
    fn rejects_reopening_finalized_games() {
        assert!(ensure_transition(GameStatus::Finalized, GameStatus::Lobby).is_err());
    }
}
