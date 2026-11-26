use super::keys_for;

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

#[cfg(test)]
mod tests;
