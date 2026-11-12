mod layers;
mod model;
mod timing;

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
    let data: Bmson = serde_json::from_slice(&std::fs::read(path)?)
        .with_context(|| format!("Invalid bmson: {}", path.display()))?;
    ensure!(
        data.version == "1.0.0",
        "Unsupported bmson version: {}",
        data.version
    );
    let timeline = Timeline::new(&data, lead_in_ms)?;
    let mode = data.info.mode_hint.as_deref().unwrap_or("beat-7k");
    let lanes = match mode {
        "beat-7k" | "generic-7keys" => 7,
        "beat-14k" | "generic-14keys" => 14,
        "beat-5k" | "popn-5k" | "generic-5keys" => 5,
        "generic-4keys" => 4,
        _ => anyhow::bail!("Unsupported bmson mode: {mode}"),
    };
    let mut chart = Chart {
        asset_dir: path.parent().unwrap_or(Path::new(".")).to_path_buf(),
        title: data.info.title.clone(),
        subtitle: data.info.subtitle.clone(),
        artist: data.info.artist.clone(),
        subartist: data.info.subartists.join(", "),
        genre: data.info.genre.clone(),
        bpm: data.info.init_bpm,
        stagefile: data.info.eyecatch_image.clone().unwrap_or_default(),
        banner: data.info.banner_image.clone().unwrap_or_default(),
        playlevel: data.info.level,
        difficulty: data.info.chart_name.as_deref().and_then(|name| {
            ["BEGINNER", "NORMAL", "HYPER", "ANOTHER", "INSANE"]
                .iter()
                .position(|n| name.eq_ignore_ascii_case(n))
                .map(|i| i as u8 + 1)
        }),
        lane_count: lanes,
        keys: keys_for(lanes),
        ..Chart::default()
    };
    let dir = path.parent().unwrap_or(Path::new("."));
    for channel in &data.sound_channels {
        add_channel(&mut chart, channel, &timeline, dir, mode)?;
    }
    layers::merge(&mut chart)?;
    for image in &data.bga.bga_header {
        chart
            .bmp_paths
            .insert(image.id, asset_path(dir, &image.name)?);
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
    let c = load(path, 0.0)?;
    Ok(ChartMeta {
        path: path.into(),
        random_seed: 0,
        preview: None,
        title: c.title,
        subtitle: c.subtitle,
        artist: c.artist,
        subartist: c.subartist,
        genre: c.genre,
        bpm: c.bpm,
        playlevel: c.playlevel,
        difficulty: c.difficulty,
        lane_count: c.lane_count,
        banner: c.banner,
        comment: c.comment,
    })
}

fn add_channel(
    chart: &mut Chart,
    channel: &SoundChannel,
    timeline: &Timeline,
    dir: &Path,
    mode: &str,
) -> Result<()> {
    let direct = asset_path(dir, &channel.name)?;
    let path = std::iter::once(direct.clone())
        .chain(["wav", "ogg", "mp3"].map(|ext| direct.with_extension(ext)))
        .find(|p| p.is_file())
        .unwrap_or(direct);
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

fn asset_path(dir: &Path, name: &str) -> Result<PathBuf> {
    let normalized = name.replace('\\', "/");
    let path = Path::new(&normalized);
    ensure!(
        !normalized.contains([':', '\0'])
            && path
                .components()
                .all(|c| matches!(c, Component::Normal(_) | Component::CurDir)),
        "Invalid asset path"
    );
    Ok(dir.join(path))
}
