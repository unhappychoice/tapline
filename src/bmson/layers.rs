use super::timing::Timeline;
use crate::chart::{Chart, Note};
use anyhow::{ensure, Context, Result};
use std::collections::HashSet;

pub(super) struct PlayableLayer {
    pub pulse: u32,
    pub length: u32,
    pub lane: usize,
    pub keysound: u32,
}

pub(super) fn merge(
    chart: &mut Chart,
    mut layers: Vec<PlayableLayer>,
    timeline: &Timeline,
) -> Result<()> {
    let playable: HashSet<_> = layers.iter().map(|layer| layer.keysound).collect();
    let mut background = HashSet::new();
    chart
        .bgm
        .retain(|e| !playable.contains(&e.keysound) && background.insert(e.keysound));

    layers.sort_by_key(|layer| (layer.pulse, layer.lane));
    for group in layers.chunk_by(|a, b| a.pulse == b.pulse && a.lane == b.lane) {
        let first = &group[0];
        ensure!(
            group.iter().all(|layer| layer.length == first.length),
            "Layered notes have unequal lengths"
        );
        let mut seen = HashSet::new();
        let sounds: Vec<_> = group
            .iter()
            .map(|layer| layer.keysound)
            .filter(|id| seen.insert(*id))
            .collect();
        let keysound = if sounds.len() > 1 {
            let id = u32::try_from(chart.sample_slices.len() + chart.sound_layers.len())
                .context("Too many sound layers")?;
            chart.sound_layers.insert(id, sounds);
            id
        } else {
            first.keysound
        };
        let pulse = u64::from(first.pulse);
        chart.notes.push(Note {
            time_ms: timeline.time(pulse),
            lane: first.lane,
            keysound: Some(keysound),
            end_ms: (first.length > 0).then(|| timeline.time(pulse + u64::from(first.length))),
        });
    }
    Ok(())
}
