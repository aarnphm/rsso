use crate::state::TeamSide;
use serde::{Deserialize, Serialize};

pub const DEFAULT_RATING: i32 = 1500;
pub const DEFAULT_K: f64 = 24.0;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TeamRating {
    pub blue: f64,
    pub red: f64,
}

pub fn expected_score(own_rating: f64, opponent_rating: f64) -> f64 {
    1.0 / (1.0 + 10_f64.powf((opponent_rating - own_rating) / 400.0))
}

pub fn rating_delta(winner: TeamSide, ratings: TeamRating, k: f64) -> i32 {
    let expected_blue = expected_score(ratings.blue, ratings.red);
    let actual_blue = if winner == TeamSide::Blue { 1.0 } else { 0.0 };
    (k * (actual_blue - expected_blue)).round() as i32
}

#[cfg(test)]
mod tests {
    use crate::elo::{rating_delta, TeamRating, DEFAULT_K};
    use crate::state::TeamSide;

    #[test]
    fn equal_ratings_move_by_half_k() {
        let delta = rating_delta(
            TeamSide::Blue,
            TeamRating {
                blue: 1500.0,
                red: 1500.0,
            },
            DEFAULT_K,
        );
        assert_eq!(delta, 12);
    }
}
