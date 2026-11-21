use crate::chart::{Chart, Lane};
use anyhow::{ensure, Context, Result};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::Path;

#[derive(Default)]
pub struct Keymap(HashMap<String, Vec<Vec<char>>>);

impl Keymap {
    pub fn load(path: Option<&Path>) -> Result<Self> {
        let Some(path) = path else {
            return Ok(Self::default());
        };
        let mut bytes = Vec::new();
        crate::fs_guard::open(path)
            .and_then(|mut file| file.read_to_end(&mut bytes))
            .with_context(|| format!("Cannot read keymap {}", path.display()))?;
        Self::parse(&bytes).with_context(|| format!("Invalid keymap {}", path.display()))
    }

    pub fn apply(&self, chart: &mut Chart) {
        if let Some(keys) = self.0.get(&Lane::binding_layout(&chart.lanes)) {
            chart.keys.clone_from(keys);
        }
    }

    fn parse(bytes: &[u8]) -> Result<Self> {
        let overrides: Vec<LayoutOverride> = serde_json::from_slice(bytes)?;
        let mut layouts = HashMap::new();
        for entry in overrides {
            let keys = entry
                .bindings()
                .with_context(|| format!("Layout {:?}", entry.layout))?;
            let layout = entry.layout;
            ensure!(
                !layouts.contains_key(&layout),
                "Duplicate layout {layout:?}"
            );
            layouts.insert(layout, keys);
        }
        Ok(Self(layouts))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LayoutOverride {
    layout: String,
    keys: Vec<String>,
}

impl LayoutOverride {
    fn bindings(&self) -> Result<Vec<Vec<char>>> {
        let mut sides = self.layout.split('/');
        ensure!(
            sides.clone().count() <= 2
                && sides
                    .all(|side| !side.is_empty() && side.bytes().all(|c| matches!(c, b'K' | b'S'))),
            "Expected one or two lane sequences made of K and S, separated by /"
        );
        let lanes = self.layout.bytes().filter(|&c| c != b'/').count();
        ensure!(self.keys.len() == lanes, "Expected {lanes} lane bindings");

        let mut assigned = HashSet::new();
        self.keys
            .iter()
            .enumerate()
            .map(|(index, aliases)| {
                parse_aliases(aliases, &mut assigned).with_context(|| format!("Lane {}", index + 1))
            })
            .collect()
    }
}

fn parse_aliases(aliases: &str, assigned: &mut HashSet<char>) -> Result<Vec<char>> {
    ensure!(!aliases.is_empty(), "At least one key is required");
    aliases
        .chars()
        .map(|key| {
            ensure!(
                key.is_ascii_graphic() || key == ' ',
                "Expected a printable ASCII key or space, got {key:?}"
            );
            let key = key.to_ascii_uppercase();
            ensure!(key != 'Q', "Q is reserved for quitting");
            ensure!(
                assigned.insert(key),
                "Key {key:?} is assigned more than once"
            );
            Ok(key)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applies_only_the_matching_player_boundaries_and_lane_roles() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/keymap.json");
        let keymap = Keymap::load(Some(&path)).unwrap();
        let mut chart = crate::chart::built_in(120.0, 0.0);
        keymap.apply(&mut chart);
        assert_eq!(chart.keys, [vec!['U'], vec!['I'], vec!['O'], vec!['P']]);

        chart.lanes = Lane::sides(6, Some(7), true, false);
        chart.keys = Lane::bindings(&chart.lanes);
        keymap.apply(&mut chart);
        assert_eq!(chart.keys[5], ['Z', '/']);
        assert_eq!(chart.keys[6], ['7']);
        assert_eq!(chart.keys[12], [' ']);

        for lanes in [
            Lane::sides(6, Some(7), false, false),
            Lane::sides(7, Some(6), false, true),
        ] {
            chart.keys = Lane::bindings(&lanes);
            chart.lanes = lanes;
            let defaults = chart.keys.clone();
            keymap.apply(&mut chart);
            assert_eq!(chart.keys, defaults);
        }
    }

    #[test]
    fn rejects_invalid_entries_before_play() {
        for (input, expected) in [
            (r#"[{"layout":"K/","keys":["a"]}]"#, "lane sequences"),
            (
                r#"[{"layout":"KK","keys":["a"]}]"#,
                "Expected 2 lane bindings",
            ),
            (r#"[{"layout":"K","keys":[""]}]"#, "At least one key"),
            (
                r#"[{"layout":"K/K","keys":["a","A"]}]"#,
                "assigned more than once",
            ),
            (r#"[{"layout":"S","keys":["q"]}]"#, "reserved for quitting"),
            (r#"[{"layout":"S","keys":["\u001b"]}]"#, "printable ASCII"),
            (
                r#"[{"layout":"K","keys":["a"]},{"layout":"K","keys":["b"]}]"#,
                "Duplicate layout",
            ),
        ] {
            let error = Keymap::parse(input.as_bytes()).err().unwrap();
            assert!(format!("{error:#}").contains(expected), "{error:#}");
        }
    }
}
