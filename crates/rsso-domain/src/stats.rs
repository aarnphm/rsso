use crate::state::TeamSide;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlayerRecord {
    pub games: u32,
    pub wins: u32,
    pub losses: u32,
}

impl PlayerRecord {
    pub fn empty() -> Self {
        Self {
            games: 0,
            wins: 0,
            losses: 0,
        }
    }

    pub fn win_rate(&self) -> f64 {
        if self.games == 0 {
            0.0
        } else {
            f64::from(self.wins) / f64::from(self.games)
        }
    }

    pub fn record_game(&mut self, side: TeamSide, winner: TeamSide) {
        self.games += 1;
        if side == winner {
            self.wins += 1;
        } else {
            self.losses += 1;
        }
    }
}
