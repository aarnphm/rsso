use crate::ids::DiscordUserId;
use crate::state::TeamSide;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub trait Rng {
    fn next_u32(&mut self) -> u32;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamAssignment {
    pub discord_user_id: DiscordUserId,
    pub team: TeamSide,
    pub slot: u8,
}

#[derive(Debug, Error)]
pub enum ShuffleError {
    #[error("at least two players are required")]
    TooFewPlayers,
    #[error("player count must be even to randomize")]
    OddPlayerCount,
}

pub fn fisher_yates<T>(items: &mut [T], rng: &mut impl Rng) {
    for i in (1..items.len()).rev() {
        let upper = i + 1;
        let j = unbiased_index(rng, upper);
        items.swap(i, j);
    }
}

pub fn split_even_teams(
    players: &[DiscordUserId],
    rng: &mut impl Rng,
) -> Result<Vec<TeamAssignment>, ShuffleError> {
    if players.len() < 2 {
        return Err(ShuffleError::TooFewPlayers);
    }
    if players.len() % 2 != 0 {
        return Err(ShuffleError::OddPlayerCount);
    }

    let mut shuffled = players.to_vec();
    fisher_yates(&mut shuffled, rng);
    let midpoint = shuffled.len() / 2;
    let assignments = shuffled
        .into_iter()
        .enumerate()
        .map(|(idx, discord_user_id)| {
            let (team, slot) = if idx < midpoint {
                (TeamSide::Blue, idx)
            } else {
                (TeamSide::Red, idx - midpoint)
            };
            TeamAssignment {
                discord_user_id,
                team,
                slot: slot as u8,
            }
        })
        .collect();
    Ok(assignments)
}

fn unbiased_index(rng: &mut impl Rng, upper: usize) -> usize {
    let upper = upper as u32;
    let zone = u32::MAX - (u32::MAX % upper);
    loop {
        let value = rng.next_u32();
        if value < zone {
            return (value % upper) as usize;
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::ids::DiscordUserId;
    use crate::shuffle::{split_even_teams, Rng};
    use crate::state::TeamSide;
    use std::collections::HashSet;

    struct SeqRng {
        values: Vec<u32>,
        idx: usize,
    }

    impl Rng for SeqRng {
        fn next_u32(&mut self) -> u32 {
            let value = self.values[self.idx % self.values.len()];
            self.idx += 1;
            value
        }
    }

    #[test]
    fn splits_even_teams_without_duplication() {
        let players = (1..=10)
            .map(|n| DiscordUserId::new(n.to_string()))
            .collect::<Vec<_>>();
        let mut rng = SeqRng {
            values: vec![4, 3, 2, 1, 0],
            idx: 0,
        };
        let assignments = split_even_teams(&players, &mut rng).expect("even players");
        assert_eq!(assignments.len(), 10);
        assert_eq!(
            assignments
                .iter()
                .filter(|assignment| assignment.team == TeamSide::Blue)
                .count(),
            5
        );
        assert_eq!(
            assignments
                .iter()
                .map(|assignment| assignment.discord_user_id.as_str())
                .collect::<HashSet<_>>()
                .len(),
            10
        );
    }

    #[test]
    fn rejects_odd_player_counts() {
        let players = (1..=3)
            .map(|n| DiscordUserId::new(n.to_string()))
            .collect::<Vec<_>>();
        let mut rng = SeqRng {
            values: vec![0],
            idx: 0,
        };
        assert!(split_even_teams(&players, &mut rng).is_err());
    }
}
