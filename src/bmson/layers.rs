use crate::chart::Chart;
use anyhow::{ensure, Context, Result};
use std::collections::HashSet;

pub(super) fn merge(chart: &mut Chart) -> Result<()> {
    let playable: HashSet<_> = chart.notes.iter().filter_map(|n| n.keysound).collect();
    let mut background = HashSet::new();
    chart
        .bgm
        .retain(|e| !playable.contains(&e.keysound) && background.insert(e.keysound));
    chart
        .notes
        .sort_by(|a, b| a.time_ms.total_cmp(&b.time_ms).then(a.lane.cmp(&b.lane)));
    let mut notes = Vec::new();
    let mut index = 0;
    while index < chart.notes.len() {
        let mut note = chart.notes[index];
        let end = index
            + chart.notes[index..]
                .partition_point(|n| n.time_ms == note.time_ms && n.lane == note.lane);
        let mut sounds = Vec::new();
        for layer in &chart.notes[index..end] {
            ensure!(
                layer.end_ms == note.end_ms,
                "Layered notes have unequal lengths"
            );
            if let Some(id) = layer.keysound.filter(|id| !sounds.contains(id)) {
                sounds.push(id);
            }
        }
        if sounds.len() > 1 {
            let id = u32::try_from(chart.sample_slices.len() + chart.sound_layers.len())
                .context("Too many sound layers")?;
            chart.sound_layers.insert(id, sounds);
            note.keysound = Some(id);
        }
        notes.push(note);
        index = end;
    }
    chart.notes = notes;
    Ok(())
}
