use super::keys_for;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Player {
    One,
    Two,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaneRole {
    Key,
    Scratch,
}

/// A play option that permutes the `Key`-role lanes of a chart once per play,
/// leaving `Scratch` lanes and player boundaries untouched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Modifier {
    #[default]
    Off,
    Random,
    Mirror,
}

impl std::fmt::Display for Modifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Modifier::Off => "off",
            Modifier::Random => "random",
            Modifier::Mirror => "mirror",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lane {
    pub player: Player,
    pub role: LaneRole,
}

impl Lane {
    pub fn ordinary(count: usize) -> Vec<Self> {
        vec![
            Self {
                player: Player::One,
                role: LaneRole::Key
            };
            count
        ]
    }

    pub fn sides(p1: usize, p2: Option<usize>, scratch1: bool, scratch2: bool) -> Vec<Self> {
        [
            (Player::One, p1, scratch1),
            (Player::Two, p2.unwrap_or(0), scratch2),
        ]
        .into_iter()
        .flat_map(|(player, count, scratch)| {
            (0..count).map(move |index| Self {
                player,
                role: if scratch && index + 1 == count {
                    LaneRole::Scratch
                } else {
                    LaneRole::Key
                },
            })
        })
        .collect()
    }

    pub(crate) fn binding_layout(lanes: &[Self]) -> String {
        let mut layout = String::new();
        let mut player = Player::One;
        for lane in lanes {
            if lane.player != player {
                layout.push('/');
                player = lane.player;
            }
            layout.push(match lane.role {
                LaneRole::Key => 'K',
                LaneRole::Scratch => 'S',
            });
        }
        layout
    }

    /// Builds a permutation mapping each original lane index to its lane
    /// index under `modifier`. `Scratch` lanes and player boundaries are
    /// always fixed points; only each player's `Key` lanes are shuffled
    /// (`Modifier::Random`) or reversed (`Modifier::Mirror`), independently
    /// per player side so double-play sides don't mirror each other.
    pub fn permutation(lanes: &[Self], modifier: Modifier, seed: u64) -> Vec<usize> {
        let mut perm: Vec<usize> = (0..lanes.len()).collect();
        if modifier == Modifier::Off {
            return perm;
        }
        for (side, player) in [Player::One, Player::Two].into_iter().enumerate() {
            let key_indexes: Vec<usize> = lanes
                .iter()
                .enumerate()
                .filter(|(_, lane)| lane.player == player && lane.role == LaneRole::Key)
                .map(|(index, _)| index)
                .collect();
            let shuffled = if modifier == Modifier::Random {
                shuffled_indexes(&key_indexes, seed, side as u64)
            } else {
                key_indexes.iter().copied().rev().collect()
            };
            for (&from, &to) in key_indexes.iter().zip(&shuffled) {
                perm[from] = to;
            }
        }
        perm
    }

    pub fn bindings(lanes: &[Self]) -> Vec<Vec<char>> {
        let double = lanes.iter().any(|lane| lane.player == Player::Two);
        [Player::One, Player::Two]
            .into_iter()
            .flat_map(|player| {
                let side: Vec<_> = lanes.iter().filter(|lane| lane.player == player).collect();
                let count = side
                    .iter()
                    .filter(|lane| lane.role == LaneRole::Key)
                    .count();
                let scratch = side.iter().any(|lane| lane.role == LaneRole::Scratch);
                let mut keys = match (player, count, scratch, double) {
                    (Player::Two, 4, _, _) => single_keys("ZXNM"),
                    (Player::Two, 5, false, _) => single_keys("ZXCNM"),
                    (Player::Two, 5, true, _) => single_keys("ZXCVB"),
                    (Player::Two, 7, _, _) => single_keys("ZXCVBNM"),
                    (Player::One, 5, false, true) => single_keys("SDFKL"),
                    (_, 0, _, _) => Vec::new(),
                    _ => keys_for(count),
                };
                if scratch {
                    keys.push(match (player, double) {
                        (Player::One, false) => vec!['A', ';'],
                        (Player::One, true) => vec!['A'],
                        (Player::Two, _) => vec![';'],
                    });
                }
                keys
            })
            .collect()
    }
}

fn single_keys(keys: &str) -> Vec<Vec<char>> {
    keys.chars().map(|key| vec![key]).collect()
}

/// Golden-ratio constant used to decorrelate per-side RNG state from a
/// shared seed, mirroring the offset BMS branch resolution uses.
const RNG_OFFSET: u64 = 0x9E3779B97F4A7C15;

fn xorshift_next(state: &mut u64) -> u64 {
    let mut x = *state;
    if x == 0 {
        x = 0xdead_beef;
    }
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    x
}

/// Fisher-Yates shuffle of `indexes`, seeded deterministically from `seed`
/// and `side` so each player's lanes shuffle independently.
fn shuffled_indexes(indexes: &[usize], seed: u64, side: u64) -> Vec<usize> {
    let mut shuffled = indexes.to_vec();
    let mut rng = seed.wrapping_add(RNG_OFFSET.wrapping_mul(side.wrapping_add(1)));
    for i in (1..shuffled.len()).rev() {
        let j = (xorshift_next(&mut rng) % (i as u64 + 1)) as usize;
        shuffled.swap(i, j);
    }
    shuffled
}

#[cfg(test)]
mod tests;
