mod layers;
mod metadata;
mod model;
mod timing;

use crate::assets::AssetIndex;
use crate::bms::ChartMeta;
use crate::chart::{keys_for, BgaEvent, BgaLayer, BgmEvent, Chart, Note, SampleSlice};
use anyhow::{ensure, Context, Result};
use model::{Bmson, SoundChannel};
use std::path::{Component, Path, PathBuf};
use timing::Timeline;

pub fn is_bmson(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("bmson"))
}

pub fn load(path: &Path, lead_in_ms: f64) -> Result<Chart> {
    let data = read(path)?;
    let timeline = Timeline::new(&data, lead_in_ms)?;
    let meta = metadata::convert(path, &data.info)?;
    let mode = data.info.mode_hint.as_deref().unwrap_or("beat-7k");
    let mut chart = Chart {
        asset_dir: path.parent().unwrap_or(Path::new(".")).to_path_buf(),
        title: meta.title,
        subtitle: meta.subtitle,
        artist: meta.artist,
        subartist: meta.subartist,
        genre: meta.genre,
        bpm: meta.bpm,
        stagefile: data.info.eyecatch_image.clone().unwrap_or_default(),
        banner: meta.banner,
        playlevel: meta.playlevel,
        difficulty: meta.difficulty,
        lane_count: meta.lane_count,
        keys: keys_for(meta.lane_count),
        ..Chart::default()
    };
    let dir = path.parent().unwrap_or(Path::new("."));
    let assets = AssetIndex::new(dir);
    for channel in &data.sound_channels {
        add_channel(&mut chart, channel, &timeline, dir, &assets, mode)?;
    }
    layers::merge(&mut chart)?;
    for image in &data.bga.bga_header {
        chart
            .bmp_paths
            .insert(image.id, asset_path(dir, &assets, &image.name, &[])?);
    }
    for (events, layer) in [
        (&data.bga.bga_events, BgaLayer::Base),
        (&data.bga.layer_events, BgaLayer::Overlay),
        (&data.bga.poor_events, BgaLayer::Poor),
    ] {
        chart.bga.extend(events.iter().map(|e| BgaEvent {
            time_ms: timeline.time(e.y.into()),
            layer,
            bmp_id: e.id,
        }));
    }
    chart.notes.sort_by(|a, b| a.time_ms.total_cmp(&b.time_ms));
    chart.bgm.sort_by(|a, b| a.time_ms.total_cmp(&b.time_ms));
    chart.bga.sort_by(|a, b| a.time_ms.total_cmp(&b.time_ms));
    chart.duration_ms = chart
        .notes
        .iter()
        .map(|n| n.end_ms.unwrap_or(n.time_ms))
        .chain(chart.bgm.iter().map(|e| e.time_ms))
        .chain(chart.bga.iter().map(|e| e.time_ms))
        .fold(lead_in_ms, f64::max)
        + 2500.0;
    ensure!(chart.duration_ms.is_finite(), "Chart duration overflow");
    Ok(chart)
}

pub fn read_meta(path: &Path) -> Result<ChartMeta> {
    let data = read(path)?;
    Timeline::new(&data, 0.0)?;
    metadata::convert(path, &data.info)
}

fn read(path: &Path) -> Result<Bmson> {
    let data: Bmson = serde_json::from_slice(&crate::chart_file::read(path)?)
        .with_context(|| format!("Invalid bmson: {}", path.display()))?;
    ensure!(
        data.version == "1.0.0",
        "Unsupported bmson version: {}",
        data.version
    );
    Ok(data)
}

fn add_channel(
    chart: &mut Chart,
    channel: &SoundChannel,
    timeline: &Timeline,
    dir: &Path,
    assets: &AssetIndex,
    mode: &str,
) -> Result<()> {
    #[cfg(test)]
    tests::CHANNEL_BUILDS.with(|count| count.set(count.get() + 1));
    let path = asset_path(dir, assets, &channel.name, &["wav", "ogg", "mp3"])?;
    let mut notes: Vec<_> = channel.notes.iter().collect();
    notes.sort_by_key(|n| n.y);
    let mut restart = notes.first().map_or(0.0, |n| timeline.time(n.y.into()));
    let mut index = 0;
    while index < notes.len() {
        let end = index + notes[index..].partition_point(|n| n.y == notes[index].y);
        let time = timeline.time(notes[index].y.into());
        if notes[index..end].iter().any(|n| !n.c) {
            restart = time;
        }
        let id = u32::try_from(chart.sample_slices.len()).context("Too many sound slices")?;
        chart.wav_paths.insert(id, path.clone());
        chart.sample_slices.insert(
            id,
            SampleSlice {
                start_ms: time - restart,
                duration_ms: notes.get(end).map(|n| timeline.time(n.y.into()) - time),
            },
        );
        for note in &notes[index..end] {
            if let Some(lane) = lane(note.x.unwrap_or(0), chart.lane_count, mode) {
                chart.notes.push(Note {
                    time_ms: time,
                    lane,
                    keysound: Some(id),
                    end_ms: (note.l > 0)
                        .then(|| timeline.time(u64::from(note.y) + u64::from(note.l))),
                    ..Note::default()
                });
            } else {
                chart.bgm.push(BgmEvent {
                    time_ms: time,
                    keysound: id,
                });
            }
        }
        index = end;
    }
    Ok(())
}

fn lane(x: u32, count: usize, mode: &str) -> Option<usize> {
    let x = x as usize;
    match mode {
        "beat-14k" if (9..=15).contains(&x) => Some(x - 2),
        "beat-14k" if (1..=7).contains(&x) => Some(x - 1),
        "beat-14k" => None,
        _ if (1..=count).contains(&x) => Some(x - 1),
        _ => None,
    }
}

fn asset_path(dir: &Path, assets: &AssetIndex, name: &str, extensions: &[&str]) -> Result<PathBuf> {
    #[cfg(test)]
    tests::ASSET_RESOLUTIONS.with(|count| count.set(count.get() + 1));
    let normalized = name.replace('\\', "/");
    let path = Path::new(&normalized);
    ensure!(
        !normalized.contains([':', '\0'])
            && path
                .components()
                .all(|c| matches!(c, Component::Normal(_) | Component::CurDir)),
        "Invalid asset path"
    );
    Ok(assets
        .resolve_path(name, extensions)
        .unwrap_or_else(|| dir.join(path)))
}

#[cfg(test)]
mod tests;
