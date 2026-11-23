mod layers;
mod metadata;
mod model;
mod timing;

use crate::assets::AssetIndex;
use crate::bga::{Extensions, OverlayTransparency};
use crate::bms::ChartMeta;
use crate::chart::{BgaEvent, BgaLayer, BgmEvent, Chart, Lane, SampleSlice};
use crate::game::JudgmentWindows;
use anyhow::{ensure, Context, Result};
use model::{Bmson, SoundChannel};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use timing::Timeline;

pub fn is_bmson(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("bmson"))
}

pub fn load(path: &Path, lead_in_ms: f64) -> Result<Chart> {
    load_bytes(path, &crate::chart_file::read(path)?, lead_in_ms)
}

pub(crate) fn load_bytes(path: &Path, bytes: &[u8], lead_in_ms: f64) -> Result<Chart> {
    let data = parse(path, bytes)?;
    let timeline = Timeline::new(&data, lead_in_ms)?;
    let meta = metadata::convert(path, &data.info)?;
    let mode = data.info.mode_hint.as_deref().unwrap_or("beat-7k");
    let lanes = if meta.lane_count == 14 {
        Lane::sides(7, Some(7), false, false)
    } else {
        Lane::ordinary(meta.lane_count)
    };
    let mut chart = Chart {
        asset_dir: path.parent().unwrap_or(Path::new(".")).to_path_buf(),
        title: meta.title,
        subtitle: meta.subtitle,
        artist: meta.artist,
        subartist: meta.subartist,
        genre: meta.genre,
        bpm: meta.bpm,
        gauge_recovery_multiplier: data.info.total.unwrap_or(100.0).abs() / 100.0,
        stagefile: data.info.eyecatch_image.clone().unwrap_or_default(),
        banner: meta.banner,
        playlevel: meta.playlevel,
        difficulty: meta.difficulty,
        def_exrank: data
            .info
            .judge_rank
            .filter(|&rank| JudgmentWindows::valid_percent(rank)),
        lane_count: meta.lane_count,
        keys: Lane::bindings(&lanes),
        lanes,
        bga_extensions: Extensions {
            overlay_transparency: OverlayTransparency::ImageAlpha,
            ..Extensions::default()
        },
        ..Chart::default()
    };
    let dir = path.parent().unwrap_or(Path::new("."));
    let assets = AssetIndex::new(dir);
    let mut playable = Vec::new();
    for channel in &data.sound_channels {
        add_channel(&mut chart, &mut playable, channel, &timeline, &assets, mode)?;
    }
    layers::merge(&mut chart, playable, &timeline)?;
    for image in &data.bga.bga_header {
        if let Some(path) = asset_path(&assets, &image.name, &[]) {
            chart.bmp_paths.insert(image.id, path);
        } else {
            chart.bmp_paths.remove(&image.id);
        }
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
    let last_event_ms = chart
        .notes
        .iter()
        .map(|n| n.end_ms.unwrap_or(n.time_ms))
        .chain(chart.bgm.iter().map(|e| e.time_ms))
        .chain(chart.bga.iter().map(|e| e.time_ms))
        .fold(lead_in_ms, f64::max);
    chart.trailing_stop_completion_ms = timeline.trailing_stop_end(last_event_ms);
    chart.duration_ms =
        (last_event_ms + 2500.0).max(chart.trailing_stop_completion_ms.unwrap_or(0.0));
    ensure!(chart.duration_ms.is_finite(), "Chart duration overflow");
    Ok(chart)
}

pub fn read_meta(path: &Path) -> Result<ChartMeta> {
    let (mut meta, preview) = parse_meta(path, &crate::chart_file::read(path)?)?;
    meta.preview = crate::bms::resolve_preview(path, preview.as_deref());
    Ok(meta)
}

pub(crate) fn parse_meta(path: &Path, bytes: &[u8]) -> Result<(ChartMeta, Option<String>)> {
    #[cfg(test)]
    crate::select::count_metadata_parse();
    let data = parse(path, bytes)?;
    Timeline::new(&data, 0.0)?;
    let meta = metadata::convert(path, &data.info)?;
    Ok((meta, data.info.preview_music))
}

fn parse(path: &Path, bytes: &[u8]) -> Result<Bmson> {
    let data: Bmson = serde_json::from_slice(bytes)
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
    playable: &mut Vec<layers::PlayableLayer>,
    channel: &SoundChannel,
    timeline: &Timeline,
    assets: &AssetIndex,
    mode: &str,
) -> Result<()> {
    #[cfg(test)]
    tests::CHANNEL_BUILDS.with(|count| count.set(count.get() + 1));
    let (path, fallbacks) = audio_paths(assets, &channel.name);
    let mut notes: Vec<_> = channel.notes.iter().collect();
    notes.sort_by_key(|n| n.y);
    let mut groups = notes
        .chunk_by(|a, b| a.y == b.y)
        .map(|group| (group, timeline.time(group[0].y.into())))
        .peekable();
    let mut restart = groups.peek().map_or(0.0, |&(_, time)| time);
    while let Some((group, time)) = groups.next() {
        if group.iter().any(|n| !n.c) {
            restart = time;
        }
        let id = u32::try_from(chart.sample_slices.len()).context("Too many sound slices")?;
        if let Some(path) = &path {
            chart.wav_paths.insert(id, Arc::clone(path));
            if !fallbacks.is_empty() {
                chart.wav_fallbacks.insert(id, Arc::clone(&fallbacks));
            }
        }
        chart.sample_slices.insert(
            id,
            SampleSlice {
                start_ms: time - restart,
                duration_ms: groups.peek().map(|&(_, next_time)| next_time - time),
            },
        );
        for note in group {
            if let Some(lane) = lane(note.x.unwrap_or(0), chart.lane_count, mode) {
                playable.push(layers::PlayableLayer {
                    pulse: note.y,
                    length: note.l,
                    lane,
                    keysound: id,
                });
            } else {
                chart.bgm.push(BgmEvent {
                    time_ms: time,
                    keysound: id,
                });
            }
        }
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

fn audio_paths(assets: &AssetIndex, name: &str) -> (Option<Arc<Path>>, Arc<[PathBuf]>) {
    #[cfg(test)]
    tests::ASSET_RESOLUTIONS.with(|count| count.set(count.get() + 1));
    let mut candidates = assets.resolve_candidates(name, &["wav", "ogg", "mp3"]);
    let primary = candidates.next();
    let extension = primary.as_deref().and_then(Path::extension);
    let fallbacks = candidates
        .filter(|path| {
            !extension
                .zip(path.extension())
                .is_some_and(|(primary, candidate)| primary.eq_ignore_ascii_case(candidate))
        })
        .collect();
    (primary.map(Arc::from), fallbacks)
}

fn asset_path(assets: &AssetIndex, name: &str, extensions: &[&str]) -> Option<PathBuf> {
    #[cfg(test)]
    tests::ASSET_RESOLUTIONS.with(|count| count.set(count.get() + 1));
    assets.resolve_path(name, extensions)
}

#[cfg(test)]
mod tests;
