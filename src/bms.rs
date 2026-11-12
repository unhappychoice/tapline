mod bga_extensions;
use crate::chart::{
    keys_for, BgaEvent, BgaLayer, BgmEvent, Chart, ExRankEvent, InvisibleNote, Mine, Note,
    ScrollEvent, VolumeEvent, VolumeTarget,
};
use anyhow::{ensure, Context, Result};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct ChartMeta {
    pub path: PathBuf,
    /// Reuse this seed when loading the selected chart.
    pub random_seed: u64,
    pub title: String,
    pub subtitle: String,
    pub artist: String,
    pub subartist: String,
    pub genre: String,
    pub banner: String,
    pub comment: String,
    pub bpm: f64,
    pub playlevel: Option<u8>,
    pub difficulty: Option<u8>,
    pub lane_count: usize,
}

pub fn read_meta(path: &Path) -> Result<ChartMeta> {
    read_meta_with_seed(path, random_seed())
}

/// Read metadata using an explicit seed instead of the environment override.
pub fn read_meta_with_seed(path: &Path, seed: u64) -> Result<ChartMeta> {
    let text = read_text(path)?;
    let text = resolve_control_flow(&text, seed);
    let mut pass = HeaderPass::for_text(&text);
    let mut raw = Vec::new();
    for line in header_lines(&text) {
        if let Some(parsed) = parse_channel_line(line) {
            raw.push(parsed);
        } else {
            pass.absorb(line)?;
        }
    }
    let (_, acc, p2_acc) = collect_chart_sides(&raw, &pass, 0.0, false, is_pms(path))?;
    let mode = detect_mode(path, pass.player, &raw, &acc, &p2_acc);
    Ok(ChartMeta {
        path: path.to_path_buf(),
        random_seed: seed,
        title: pass.title,
        subtitle: pass.subtitle,
        artist: pass.artist,
        subartist: pass.subartist,
        genre: pass.genre,
        banner: pass.banner,
        comment: pass.comment,
        bpm: pass.bpm,
        playlevel: pass.playlevel,
        difficulty: pass.difficulty,
        lane_count: mode.playable_lane_count(&acc, &p2_acc),
    })
}

pub fn load(path: &Path, lead_in_ms: f64) -> Result<Chart> {
    load_with_seed(path, lead_in_ms, random_seed())
}

/// Load the same control-flow branches used by metadata with this seed.
pub fn load_with_seed(path: &Path, lead_in_ms: f64, seed: u64) -> Result<Chart> {
    let text = read_text(path)?;
    let text = resolve_control_flow(&text, seed);
    let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
    let mut pass = HeaderPass::for_text(&text);
    let mut raw: Vec<(u32, String, String)> = Vec::new();
    for line in header_lines(&text) {
        match parse_channel_line(line) {
            Some(parsed) => raw.push(parsed),
            None => pass.absorb(line)?,
        }
    }
    build_chart(pass, raw, dir, path, lead_in_ms)
}

fn is_pms(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("pms"))
}

#[derive(Default)]
struct ChartAcc {
    lanes: LaneSet,
    notes: Vec<Note>,
    invisible_notes: Vec<InvisibleNote>,
    mines: Vec<Mine>,
    bgm: Vec<BgmEvent>,
}

#[derive(Copy, Clone)]
struct LaneMap {
    normal: fn(&str) -> Option<usize>,
    invisible: fn(&str) -> Option<usize>,
    long: fn(&str) -> Option<usize>,
    mine: fn(&str) -> Option<usize>,
}

const P1_LANES: LaneMap = LaneMap {
    normal: channel_to_lane,
    invisible: |ch| {
        ch.strip_prefix('3')
            .and_then(|suffix| channel_to_lane(&format!("1{suffix}")))
    },
    long: long_channel_to_lane,
    mine: mine_channel_to_lane,
};

const P2_LANES: LaneMap = LaneMap {
    normal: p2_channel_to_lane,
    invisible: |ch| {
        ch.strip_prefix('4')
            .and_then(|suffix| p2_channel_to_lane(&format!("2{suffix}")))
    },
    long: p2_long_channel_to_lane,
    mine: p2_mine_channel_to_lane,
};

const PMS_LANES: LaneMap = LaneMap {
    normal: |ch| pms_channel_to_lane(ch, "1", "2"),
    invisible: |ch| pms_channel_to_lane(ch, "3", "4"),
    long: |ch| pms_channel_to_lane(ch, "5", "6"),
    mine: |ch| pms_channel_to_lane(ch, "D", "E"),
};

fn pms_channel_to_lane(ch: &str, left: &str, right: &str) -> Option<usize> {
    match (ch.strip_prefix(left), ch.strip_prefix(right)) {
        (Some("1"), _) => Some(0),
        (Some("2"), _) => Some(1),
        (Some("3"), _) => Some(2),
        (Some("4"), _) => Some(3),
        (Some("5"), _) => Some(4),
        (_, Some("2")) => Some(5),
        (_, Some("3")) => Some(6),
        (_, Some("4")) => Some(7),
        (_, Some("5")) => Some(8),
        _ => None,
    }
}

fn build_chart(
    mut pass: HeaderPass,
    raw: Vec<(u32, String, String)>,
    dir: PathBuf,
    path: &Path,
    lead_in_ms: f64,
) -> Result<Chart> {
    let assets = AssetIndex::new(&dir);
    let wav_paths = resolve_wavs(&assets, &pass.wav_defs);
    let max_measure = raw
        .iter()
        .map(|(m, _, _)| *m)
        .chain(pass.stp_events.iter().map(|event| event.measure))
        .max()
        .unwrap_or(0);
    let (timeline, acc, p2_acc) = collect_chart_sides(&raw, &pass, lead_in_ms, true, is_pms(path))?;
    let mut bga = collect_bga_events(&raw, &timeline, pass.base);
    let exrank_events = collect_exrank_events(&raw, &timeline, &pass.exrank_defs, pass.base);
    let bmp_paths = resolve_bmp_paths(&assets, &pass.bmp_defs);
    let mode = detect_mode(path, pass.player, &raw, &acc, &p2_acc);
    let lane_count = mode.playable_lane_count(&acc, &p2_acc);
    let p1_lane_count = mode.side_lane_count(&acc);
    let p2_lane_count = mode.side_lane_count(&p2_acc);
    pass.bga_extensions
        .switches
        .values_mut()
        .for_each(|switch| {
            switch.lane = bga_switch_lane(
                &switch.channel,
                p1_lane_count,
                mode.double_play.then_some(p2_lane_count),
                is_pms(path),
            );
        });
    let scroll = collect_scroll_events(&raw, &timeline, "SC", &pass.scroll_defs, pass.base);
    let speed = collect_scroll_events(&raw, &timeline, "SP", &pass.speed_defs, pass.base);
    let mut notes = acc.notes;
    let mut invisible_notes = acc.invisible_notes;
    let mut mines = acc.mines;
    let mut bgm = acc.bgm;
    let mut p2_notes = p2_acc.notes;
    let mut p2_invisible_notes = p2_acc.invisible_notes;
    let mut p2_mines = p2_acc.mines;
    if let Some(lnobj) = pass.lnobj {
        promote_lnobj(&mut notes, lnobj);
        promote_lnobj(&mut p2_notes, lnobj);
    }
    notes.retain(|n| n.lane < p1_lane_count || n.lane == 7);
    mines.retain(|n| n.lane < p1_lane_count || n.lane == 7);
    p2_notes.retain(|n| n.lane < p2_lane_count || n.lane == 7);
    p2_mines.retain(|n| n.lane < p2_lane_count || n.lane == 7);
    invisible_notes.retain(|n| n.lane < p1_lane_count || n.lane == 7);
    p2_invisible_notes.retain(|n| n.lane < p2_lane_count || n.lane == 7);
    compact_invisible_scratch(&mut invisible_notes, p1_lane_count);
    compact_invisible_scratch(&mut p2_invisible_notes, p2_lane_count);
    // Scratch is collected as lane 7, then compacted after the five keys in 6K.
    compact_scratch(&mut notes, &mut mines, p1_lane_count);
    compact_scratch(&mut p2_notes, &mut p2_mines, p2_lane_count);
    if lane_count > p1_lane_count {
        invisible_notes.extend(p2_invisible_notes.drain(..).map(|mut n| {
            n.lane += p1_lane_count;
            n
        }));
        notes.extend(p2_notes.drain(..).map(|mut n| {
            n.lane += p1_lane_count;
            n
        }));
        mines.extend(p2_mines.drain(..).map(|mut m| {
            m.lane += p1_lane_count;
            m
        }));
    }
    invisible_notes.sort_by(|a, b| a.time_ms.total_cmp(&b.time_ms));
    p2_invisible_notes.sort_by(|a, b| a.time_ms.total_cmp(&b.time_ms));
    notes.sort_by(|a, b| a.time_ms.partial_cmp(&b.time_ms).unwrap());
    mines.sort_by(|a, b| a.time_ms.partial_cmp(&b.time_ms).unwrap());
    p2_notes.sort_by(|a, b| a.time_ms.partial_cmp(&b.time_ms).unwrap());
    p2_mines.sort_by(|a, b| a.time_ms.partial_cmp(&b.time_ms).unwrap());
    bgm.sort_by(|a, b| a.time_ms.partial_cmp(&b.time_ms).unwrap());
    bga.sort_by(|a, b| a.time_ms.partial_cmp(&b.time_ms).unwrap());
    let duration_ms = timeline.end_time(max_measure) + 2500.0;
    ensure!(duration_ms.is_finite(), "BMS duration must be finite");
    Ok(Chart {
        title: pass.title,
        subtitle: pass.subtitle,
        artist: pass.artist,
        subartist: pass.subartist,
        genre: pass.genre,
        stagefile: pass.stagefile,
        banner: pass.banner,
        backbmp: pass.backbmp,
        comment: pass.comment,
        asset_dir: dir.clone(),
        maker: pass.maker,
        bpm: pass.bpm,
        playlevel: pass.playlevel,
        difficulty: pass.difficulty,
        rank: pass.rank,
        def_exrank: pass.def_exrank,
        exrank_defs: pass.exrank_defs,
        exrank_events,
        total: pass.total,
        vol_wav: pass.vol_wav,
        volume_events: collect_volume_events(&raw, &timeline),
        notes,
        mines,
        p2_notes,
        p2_mines,
        invisible_notes,
        p2_invisible_notes,
        bgm,
        bga,
        bga_extensions: bga_extensions::collect(pass.bga_extensions, &raw, &timeline, pass.base),
        bmp_paths,
        scroll,
        speed,
        duration_ms,
        lane_count,
        keys: keys_for(lane_count),
        wav_paths,
        sample_settings: pass.sample_settings,
        sample_slices: HashMap::new(),
        sound_layers: HashMap::new(),
    })
}

fn collect_chart_sides(
    raw: &[(u32, String, String)],
    pass: &HeaderPass,
    lead_in_ms: f64,
    collect_bgm: bool,
    pms: bool,
) -> Result<(Timeline, ChartAcc, ChartAcc)> {
    let max_measure = raw
        .iter()
        .map(|(m, _, _)| *m)
        .chain(pass.stp_events.iter().map(|event| event.measure))
        .max()
        .unwrap_or(0);
    let scales = collect_measure_scales(raw);
    let events = collect_timing_events(
        raw,
        &pass.bpm_defs,
        &pass.stop_defs,
        &pass.stp_events,
        pass.base,
    );
    let timeline = Timeline::build(lead_in_ms, pass.bpm, &scales, &events, max_measure)?;
    let lanes = if pms { PMS_LANES } else { P1_LANES };
    let acc = accumulate_side(raw, &timeline, lanes, collect_bgm, pass.lntype, pass.base);
    let p2_acc = if pms {
        ChartAcc::default()
    } else {
        accumulate_side(raw, &timeline, P2_LANES, false, pass.lntype, pass.base)
    };
    Ok((timeline, acc, p2_acc))
}

#[derive(Debug, PartialEq, Eq)]
struct BmsMode {
    keys_per_side: usize,
    double_play: bool,
    scratch: bool,
    reserve_seven: bool,
    declared_sides: bool,
}

impl BmsMode {
    fn side_lane_count(&self, acc: &ChartAcc) -> usize {
        match self.keys_per_side {
            9 => 9,
            _ if self.reserve_seven => 7 + usize::from(acc.lanes.0.contains(&7)),
            _ if self.declared_sides => acc.lanes.count().max(5),
            _ => acc.lanes.count(),
        }
    }

    fn playable_lane_count(&self, acc: &ChartAcc, p2_acc: &ChartAcc) -> usize {
        let p1 = self.side_lane_count(acc);
        let p2 = self.side_lane_count(p2_acc);
        if self.double_play {
            p1 + p2
        } else {
            p1
        }
    }
}

fn detect_mode(
    path: &Path,
    player: Option<u8>,
    raw: &[(u32, String, String)],
    acc: &ChartAcc,
    p2_acc: &ChartAcc,
) -> BmsMode {
    let extension = path.extension().and_then(|s| s.to_str()).unwrap_or("");
    if extension.eq_ignore_ascii_case("pms") {
        return BmsMode {
            keys_per_side: 9,
            double_play: false,
            scratch: false,
            reserve_seven: false,
            declared_sides: false,
        };
    }
    let extended = extension.eq_ignore_ascii_case("bme")
        || acc.lanes.0.iter().any(|lane| matches!(lane, 5 | 6))
        || (matches!(player, Some(2 | 3))
            && p2_acc.lanes.0.iter().any(|lane| matches!(lane, 5 | 6)));
    let double_play =
        matches!(player, Some(2 | 3)) || !p2_acc.notes.is_empty() || !p2_acc.mines.is_empty();
    let scratch = raw.iter().any(|(_, ch, data)| {
        matches!(ch.as_str(), "16" | "26" | "56" | "66" | "D6" | "E6")
            && parse_slots(data, 36).iter().any(|slot| *slot != 0)
    });
    BmsMode {
        keys_per_side: if extended { 7 } else { 5 },
        double_play,
        scratch,
        reserve_seven: extension.eq_ignore_ascii_case("bme"),
        declared_sides: matches!(player, Some(2 | 3)),
    }
}

fn compact_invisible_scratch(notes: &mut [InvisibleNote], lane_count: usize) {
    if lane_count == 6 {
        notes
            .iter_mut()
            .filter(|n| n.lane == 7)
            .for_each(|n| n.lane = 5);
    }
}

fn compact_scratch(notes: &mut [Note], mines: &mut [Mine], lane_count: usize) {
    if lane_count == 6 {
        notes
            .iter_mut()
            .filter(|n| n.lane == 7)
            .for_each(|n| n.lane = 5);
        mines
            .iter_mut()
            .filter(|m| m.lane == 7)
            .for_each(|m| m.lane = 5);
    }
}

fn p2_channel_to_lane(ch: &str) -> Option<usize> {
    match ch {
        "21" => Some(0),
        "22" => Some(1),
        "23" => Some(2),
        "24" => Some(3),
        "25" => Some(4),
        "26" => Some(7),
        "27" => None, // Free zone / pedal is not a playable lane.
        "28" => Some(5),
        "29" => Some(6),
        _ => None,
    }
}

fn p2_long_channel_to_lane(ch: &str) -> Option<usize> {
    match ch {
        "61" => Some(0),
        "62" => Some(1),
        "63" => Some(2),
        "64" => Some(3),
        "65" => Some(4),
        "66" => Some(7),
        "67" => None, // Free zone / pedal is not a playable lane.
        "68" => Some(5),
        "69" => Some(6),
        _ => None,
    }
}

fn p2_mine_channel_to_lane(ch: &str) -> Option<usize> {
    match ch {
        "E1" => Some(0),
        "E2" => Some(1),
        "E3" => Some(2),
        "E4" => Some(3),
        "E5" => Some(4),
        "E6" => Some(7),
        "E7" => None, // Free zone / pedal is not a playable lane.
        "E8" => Some(5),
        "E9" => Some(6),
        _ => None,
    }
}

fn accumulate_side(
    raw: &[(u32, String, String)],
    timeline: &Timeline,
    lanes: LaneMap,
    collect_bgm: bool,
    lntype: u8,
    base: u32,
) -> ChartAcc {
    let raw = merge_note_lines(raw, lanes, base);
    let raw = raw.as_slice();
    let mut acc = ChartAcc::default();
    for (measure, ch, data) in raw {
        if let Some(lane) = (lanes.normal)(ch) {
            push_normal_slots(&mut acc, lane, *measure, data, timeline, base);
        } else if let Some(lane) = (lanes.invisible)(ch) {
            let slots = parse_slots(data, base);
            slots
                .iter()
                .enumerate()
                .filter(|(_, slot)| **slot != 0)
                .for_each(|(i, slot)| {
                    acc.lanes.observe_lane(lane);
                    acc.invisible_notes.push(InvisibleNote {
                        time_ms: timeline.time_at(*measure, i as f64 / slots.len() as f64),
                        lane,
                        keysound: *slot,
                    });
                });
        } else if collect_bgm && ch == "01" {
            push_bgm_slots(&mut acc, *measure, data, timeline, base);
        }
    }
    if lntype == 2 {
        materialize_continuous_long_notes(&mut acc, raw, timeline, lanes.long, base);
    } else {
        materialize_long_notes_with(&mut acc, raw, timeline, lanes.long, base);
    }
    materialize_mines_with(&mut acc, raw, timeline, lanes.mine);
    acc
}

fn merge_note_lines(
    raw: &[(u32, String, String)],
    lanes: LaneMap,
    base: u32,
) -> Vec<(u32, String, String)> {
    let mut seen = HashSet::new();
    let mut merged = raw.to_vec();
    for (measure, ch, data) in merged.iter_mut().rev() {
        let slots = if (lanes.mine)(ch).is_some() {
            parse_slots(data, 36)
        } else if (lanes.normal)(ch).is_some()
            || (lanes.long)(ch).is_some()
            || (lanes.invisible)(ch).is_some()
        {
            parse_slots(data, base)
        } else {
            continue;
        };
        let n = slots.len();
        for (i, _) in slots.iter().enumerate().filter(|(_, slot)| **slot != 0) {
            let divisor = greatest_common_divisor(i, n);
            let position = (*measure, ch.clone(), i / divisor, n / divisor);
            if !seen.insert(position) {
                data.replace_range(i * 2..i * 2 + 2, "00");
            }
        }
    }
    merged
}

fn greatest_common_divisor(a: usize, b: usize) -> usize {
    if b == 0 {
        a
    } else {
        greatest_common_divisor(b, a % b)
    }
}

fn push_normal_slots(
    acc: &mut ChartAcc,
    lane: usize,
    measure: u32,
    data: &str,
    timeline: &Timeline,
    base: u32,
) {
    let slots = parse_slots(data, base);
    let n = slots.len();
    if n == 0 {
        return;
    }
    for (i, slot) in slots.iter().enumerate() {
        if *slot == 0 {
            continue;
        }
        let t = timeline.time_at(measure, i as f64 / n as f64);
        acc.lanes.observe_lane(lane);
        acc.notes.push(Note {
            time_ms: t,
            lane,
            hit: false,
            keysound: Some(*slot),
            end_ms: None,
            held_since: None,
        });
    }
}

fn push_bgm_slots(acc: &mut ChartAcc, measure: u32, data: &str, timeline: &Timeline, base: u32) {
    let slots = parse_slots(data, base);
    let n = slots.len();
    if n == 0 {
        return;
    }
    for (i, slot) in slots.iter().enumerate() {
        if *slot == 0 {
            continue;
        }
        let t = timeline.time_at(measure, i as f64 / n as f64);
        acc.bgm.push(BgmEvent {
            time_ms: t,
            keysound: *slot,
        });
    }
}

fn materialize_long_notes_with(
    acc: &mut ChartAcc,
    raw: &[(u32, String, String)],
    timeline: &Timeline,
    lane_of: fn(&str) -> Option<usize>,
    base: u32,
) {
    let mut per_lane: HashMap<usize, Vec<(f64, u32)>> = HashMap::new();
    for (measure, ch, data) in raw {
        let Some(lane) = lane_of(ch) else {
            continue;
        };
        let slots = parse_slots(data, base);
        let n = slots.len();
        if n == 0 {
            continue;
        }
        for (i, slot) in slots.iter().enumerate() {
            if *slot == 0 {
                continue;
            }
            let t = timeline.time_at(*measure, i as f64 / n as f64);
            per_lane.entry(lane).or_default().push((t, *slot));
        }
    }
    for (lane, mut events) in per_lane {
        events.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        for pair in events.chunks(2) {
            if pair.len() != 2 {
                continue;
            }
            let (start_ms, ks) = pair[0];
            let (end_ms, _) = pair[1];
            acc.lanes.observe_lane(lane);
            acc.notes.push(Note {
                time_ms: start_ms,
                lane,
                hit: false,
                keysound: Some(ks),
                end_ms: Some(end_ms),
                held_since: None,
            });
        }
    }
}

fn materialize_continuous_long_notes(
    acc: &mut ChartAcc,
    raw: &[(u32, String, String)],
    timeline: &Timeline,
    lane_of: fn(&str) -> Option<usize>,
    base: u32,
) {
    let mut per_lane: HashMap<usize, Vec<(f64, f64, u32)>> = HashMap::new();
    for (measure, ch, data) in raw {
        let Some(lane) = lane_of(ch) else {
            continue;
        };
        let slots = parse_slots(data, base);
        let n = slots.len() as f64;
        per_lane.entry(lane).or_default().extend(
            slots
                .iter()
                .enumerate()
                .filter(|(_, slot)| **slot != 0)
                .map(|(i, slot)| {
                    let start = *measure as f64 + i as f64 / n;
                    let end = *measure as f64 + (i + 1) as f64 / n;
                    (start, end, *slot)
                }),
        );
    }
    for (lane, mut spans) in per_lane {
        spans.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        let mut merged: Vec<(f64, f64, u32)> = Vec::new();
        for (start, end, keysound) in spans {
            if let Some(previous) = merged.last_mut().filter(|span| start <= span.1) {
                previous.1 = previous.1.max(end);
            } else {
                merged.push((start, end, keysound));
            }
        }
        for (start, end, keysound) in merged {
            acc.lanes.observe_lane(lane);
            acc.notes.push(Note {
                time_ms: timeline.time_at(start.floor() as u32, start.fract()),
                lane,
                hit: false,
                keysound: Some(keysound),
                end_ms: Some(timeline.time_at(end.floor() as u32, end.fract())),
                held_since: None,
            });
        }
    }
}

fn materialize_mines_with(
    acc: &mut ChartAcc,
    raw: &[(u32, String, String)],
    timeline: &Timeline,
    lane_of: fn(&str) -> Option<usize>,
) {
    for (measure, ch, data) in raw {
        let Some(lane) = lane_of(ch) else {
            continue;
        };
        let slots = parse_slots(data, 36);
        let n = slots.len();
        if n == 0 {
            continue;
        }
        for (i, slot) in slots.iter().enumerate() {
            if *slot == 0 {
                continue;
            }
            let t = timeline.time_at(*measure, i as f64 / n as f64);
            acc.lanes.observe_lane(lane);
            acc.mines.push(Mine {
                time_ms: t,
                lane,
                damage: *slot,
                exploded: false,
            });
        }
    }
}

fn bga_switch_lane(
    channel: &str,
    p1_lanes: usize,
    p2_lanes: Option<usize>,
    pms: bool,
) -> Option<usize> {
    if pms {
        return (PMS_LANES.normal)(channel);
    }
    let (lane, side_lanes, offset) = channel_to_lane(channel)
        .map(|lane| (lane, p1_lanes, 0))
        .or_else(|| Some((p2_channel_to_lane(channel)?, p2_lanes?, p1_lanes)))?;
    let scratch = matches!(side_lanes, 6 | 8);
    match lane {
        7 if scratch => Some(offset + side_lanes - 1),
        lane if lane < side_lanes - usize::from(scratch) => Some(offset + lane),
        _ => None,
    }
}

fn positive_percent(value: &str) -> Option<f64> {
    value
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite() && *v > 0.0)
}

fn collect_exrank_events(
    raw: &[(u32, String, String)],
    timeline: &Timeline,
    defs: &HashMap<u32, f64>,
    base: u32,
) -> Vec<ExRankEvent> {
    let mut events: Vec<_> = raw
        .iter()
        .filter(|(_, ch, _)| ch == "A0")
        .flat_map(|(measure, _, data)| {
            let slots = parse_slots(data, base);
            let count = slots.len();
            slots.into_iter().enumerate().filter_map(move |(i, id)| {
                defs.get(&id).map(|percent| ExRankEvent {
                    time_ms: timeline.time_at(*measure, i as f64 / count as f64),
                    percent: *percent,
                })
            })
        })
        .collect();
    events.sort_by(|a, b| a.time_ms.total_cmp(&b.time_ms));
    events
}

fn bga_channel_to_layer(ch: &str) -> Option<BgaLayer> {
    match ch {
        "04" => Some(BgaLayer::Base),
        "06" => Some(BgaLayer::Poor),
        "07" => Some(BgaLayer::Overlay),
        "0A" => Some(BgaLayer::Overlay2),
        _ => None,
    }
}

fn collect_scroll_events(
    raw: &[(u32, String, String)],
    timeline: &Timeline,
    channel: &str,
    definitions: &HashMap<u32, f64>,
    base: u32,
) -> Vec<ScrollEvent> {
    let mut events: Vec<_> = raw
        .iter()
        .filter(|(_, ch, _)| ch == channel)
        .flat_map(|(measure, _, data)| {
            let slots = parse_slots(data, base);
            let count = slots.len();
            slots.into_iter().enumerate().filter_map(move |(i, id)| {
                definitions
                    .get(&id)
                    .filter(|_| id != 0)
                    .map(|&multiplier| ScrollEvent {
                        time_ms: timeline.time_at(*measure, i as f64 / count as f64),
                        multiplier,
                    })
            })
        })
        .collect();
    events.sort_by(|a, b| a.time_ms.total_cmp(&b.time_ms));
    events
}

fn collect_volume_events(raw: &[(u32, String, String)], timeline: &Timeline) -> Vec<VolumeEvent> {
    let mut events = Vec::new();
    for (measure, channel, data) in raw {
        let target = match channel.as_str() {
            "97" => VolumeTarget::Bgm,
            "98" => VolumeTarget::Key,
            _ => continue,
        };
        let slots = parse_slots_hex(data);
        events.extend(
            slots
                .iter()
                .enumerate()
                .filter(|(_, v)| **v != 0)
                .map(|(i, &volume)| VolumeEvent {
                    time_ms: timeline.time_at(*measure, i as f64 / slots.len() as f64),
                    target,
                    volume: volume as u8,
                }),
        );
    }
    events.sort_by(|a, b| a.time_ms.total_cmp(&b.time_ms));
    events
}

fn collect_bga_events(
    raw: &[(u32, String, String)],
    timeline: &Timeline,
    base: u32,
) -> Vec<BgaEvent> {
    let mut out: Vec<BgaEvent> = Vec::new();
    for (measure, ch, data) in raw {
        let Some(layer) = bga_channel_to_layer(ch) else {
            continue;
        };
        let slots = parse_slots(data, base);
        let n = slots.len();
        if n == 0 {
            continue;
        }
        for (i, slot) in slots.iter().enumerate() {
            if *slot == 0 {
                continue;
            }
            let t = timeline.time_at(*measure, i as f64 / n as f64);
            out.push(BgaEvent {
                time_ms: t,
                layer,
                bmp_id: *slot,
            });
        }
    }
    out
}

fn resolve_bmp_paths(assets: &AssetIndex, defs: &HashMap<u32, String>) -> HashMap<u32, PathBuf> {
    assets.resolve(defs, &["png", "bmp", "jpg", "jpeg", "gif", "mp4", "webm"])
}

fn mine_channel_to_lane(ch: &str) -> Option<usize> {
    match ch {
        "D1" => Some(0),
        "D2" => Some(1),
        "D3" => Some(2),
        "D4" => Some(3),
        "D5" => Some(4),
        "D6" => Some(7),
        "D7" => None, // Free zone / pedal is not a playable lane.
        "D8" => Some(5),
        "D9" => Some(6),
        _ => None,
    }
}

fn long_channel_to_lane(ch: &str) -> Option<usize> {
    match ch {
        "51" => Some(0),
        "52" => Some(1),
        "53" => Some(2),
        "54" => Some(3),
        "55" => Some(4),
        "56" => Some(7),
        "57" => None, // Free zone / pedal is not a playable lane.
        "58" => Some(5),
        "59" => Some(6),
        _ => None,
    }
}

fn promote_lnobj(notes: &mut Vec<Note>, lnobj: u32) {
    let mut per_lane: HashMap<usize, Vec<usize>> = HashMap::new();
    let mut indices: Vec<usize> = (0..notes.len()).collect();
    indices.sort_by(|&a, &b| notes[a].time_ms.partial_cmp(&notes[b].time_ms).unwrap());
    for i in indices {
        per_lane.entry(notes[i].lane).or_default().push(i);
    }
    let mut drop_set: std::collections::HashSet<usize> = std::collections::HashSet::new();
    for lane_indices in per_lane.values() {
        let mut prev: Option<usize> = None;
        for &i in lane_indices {
            if notes[i].keysound == Some(lnobj) {
                if let Some(p) = prev {
                    notes[p].end_ms = Some(notes[i].time_ms);
                    drop_set.insert(i);
                }
                prev = None;
            } else {
                prev = Some(i);
            }
        }
    }
    let mut idx = 0;
    notes.retain(|_| {
        let keep = !drop_set.contains(&idx);
        idx += 1;
        keep
    });
}

fn collect_measure_scales(raw: &[(u32, String, String)]) -> HashMap<u32, f64> {
    let mut out: HashMap<u32, f64> = HashMap::new();
    for (measure, ch, data) in raw {
        if ch != "02" {
            continue;
        }
        if let Ok(v) = data.trim().parse::<f64>() {
            // A sane BMS measure scales the standard 4/4 bar by roughly
            // 0.001..=100. Non-standard extensions abuse channel 02 to encode
            // BPM references as fat integers; those blow up the timeline into
            // hours of silence. Drop anything that clearly isn't a scale.
            if (0.001..=100.0).contains(&v) {
                out.insert(*measure, v);
            }
        }
    }
    out
}

#[derive(Debug, Clone, Copy)]
enum TimingEventKind {
    Bpm(f64),
    Stop(f64),
    StopMs(f64),
}

#[derive(Debug, Clone, Copy)]
struct TimingEvent {
    measure: u32,
    frac: f64,
    kind: TimingEventKind,
}

impl TimingEvent {
    fn sort_bucket(&self) -> u8 {
        match self.kind {
            TimingEventKind::Bpm(_) => 0,
            TimingEventKind::Stop(_) | TimingEventKind::StopMs(_) => 1,
        }
    }
}

fn collect_timing_events(
    raw: &[(u32, String, String)],
    bpm_defs: &HashMap<u32, f64>,
    stop_defs: &HashMap<u32, f64>,
    stp_events: &[TimingEvent],
    base: u32,
) -> Vec<TimingEvent> {
    type TimingResolver<'a> = Box<dyn Fn(u32) -> Option<TimingEventKind> + 'a>;
    let mut out = stp_events.to_vec();
    for (measure, ch, data) in raw {
        let (slots, resolve): (Vec<u32>, TimingResolver) = match ch.as_str() {
            "03" => (
                parse_slots_hex(data),
                Box::new(|v| Some(TimingEventKind::Bpm(v as f64))),
            ),
            "08" => (
                parse_slots(data, base),
                Box::new(|v| bpm_defs.get(&v).copied().map(TimingEventKind::Bpm)),
            ),
            "09" => (
                parse_slots(data, base),
                Box::new(|v| stop_defs.get(&v).copied().map(TimingEventKind::Stop)),
            ),
            _ => continue,
        };
        let n = slots.len();
        if n == 0 {
            continue;
        }
        for (i, slot) in slots.iter().enumerate() {
            if *slot == 0 {
                continue;
            }
            if let Some(kind) = resolve(*slot) {
                if kind_is_positive(&kind) {
                    out.push(TimingEvent {
                        measure: *measure,
                        frac: i as f64 / n as f64,
                        kind,
                    });
                }
            }
        }
    }
    out.sort_by(|a, b| {
        a.measure
            .cmp(&b.measure)
            .then_with(|| {
                a.frac
                    .partial_cmp(&b.frac)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| a.sort_bucket().cmp(&b.sort_bucket()))
    });
    out
}

fn parse_stp(value: &str) -> Option<TimingEvent> {
    let mut fields = value.split_whitespace();
    let (measure, position) = fields.next()?.split_once('.')?;
    let valid_part = |part: &str| part.len() == 3 && part.bytes().all(|b| b.is_ascii_digit());
    if !valid_part(measure) || !valid_part(position) {
        return None;
    }
    let ms = fields.next()?.parse::<f64>().ok()?;
    (fields.next().is_none() && ms.is_finite() && ms > 0.0).then_some(TimingEvent {
        measure: measure.parse().ok()?,
        frac: position.parse::<u32>().ok()? as f64 / 1000.0,
        kind: TimingEventKind::StopMs(ms),
    })
}

fn kind_is_positive(kind: &TimingEventKind) -> bool {
    match *kind {
        TimingEventKind::Bpm(v) | TimingEventKind::Stop(v) | TimingEventKind::StopMs(v) => v > 0.0,
    }
}

fn parse_slots_hex(data: &str) -> Vec<u32> {
    let bytes = data.as_bytes();
    let take = bytes.len() - (bytes.len() % 2);
    (0..take)
        .step_by(2)
        .map(|i| {
            std::str::from_utf8(&bytes[i..i + 2])
                .ok()
                .and_then(|s| u32::from_str_radix(s, 16).ok())
                .unwrap_or(0)
        })
        .collect()
}

#[derive(Debug, Clone, Copy)]
struct Segment {
    frac_start: f64,
    ms_at_start: f64,
    bpm: f64,
}

struct MeasureSlice {
    beats: f64,
    segments: Vec<Segment>,
}

struct Timeline {
    measures: Vec<MeasureSlice>,
    tail_ms: f64,
    tail_bpm: f64,
    default_beats: f64,
}

impl Timeline {
    fn build(
        lead_in_ms: f64,
        default_bpm: f64,
        scales: &HashMap<u32, f64>,
        events: &[TimingEvent],
        max_measure: u32,
    ) -> Result<Self> {
        ensure!(lead_in_ms.is_finite(), "BMS lead-in must be finite");
        let n = (max_measure as usize).saturating_add(2);
        let mut measures: Vec<MeasureSlice> = Vec::with_capacity(n);
        let mut current_ms = lead_in_ms;
        let mut current_bpm = default_bpm;
        let mut ev_idx = 0;
        for m in 0..n {
            let beats = 4.0 * scales.get(&(m as u32)).copied().unwrap_or(1.0);
            let mut segments: Vec<Segment> = vec![Segment {
                frac_start: 0.0,
                ms_at_start: current_ms,
                bpm: current_bpm,
            }];
            while ev_idx < events.len() && events[ev_idx].measure as usize == m {
                let ev = events[ev_idx];
                ev_idx += 1;
                let frac = ev.frac.clamp(0.0, 1.0);
                if frac >= 1.0 {
                    continue;
                }
                let last = *segments.last().unwrap();
                let ms_here =
                    last.ms_at_start + (frac - last.frac_start) * beats * 60_000.0 / last.bpm;
                match ev.kind {
                    TimingEventKind::Bpm(new_bpm) => {
                        if (frac - last.frac_start).abs() < f64::EPSILON {
                            segments.last_mut().unwrap().bpm = new_bpm;
                        } else {
                            segments.push(Segment {
                                frac_start: frac,
                                ms_at_start: ms_here,
                                bpm: new_bpm,
                            });
                        }
                        current_bpm = new_bpm;
                    }
                    TimingEventKind::Stop(_) | TimingEventKind::StopMs(_) => {
                        let pause_ms = match ev.kind {
                            TimingEventKind::Stop(ticks) => ticks / 48.0 * 60_000.0 / last.bpm,
                            TimingEventKind::StopMs(ms) => ms,
                            TimingEventKind::Bpm(_) => unreachable!(),
                        };
                        segments.push(Segment {
                            frac_start: frac,
                            ms_at_start: ms_here + pause_ms,
                            bpm: last.bpm,
                        });
                    }
                }
            }
            let last = *segments.last().unwrap();
            let end_ms = last.ms_at_start + (1.0 - last.frac_start) * beats * 60_000.0 / last.bpm;
            ensure!(
                end_ms.is_finite(),
                "BMS timeline must be finite at measure {m}"
            );
            measures.push(MeasureSlice { beats, segments });
            current_ms = end_ms;
        }
        Ok(Self {
            measures,
            tail_ms: current_ms,
            tail_bpm: current_bpm,
            default_beats: 4.0,
        })
    }

    fn time_at(&self, measure: u32, frac: f64) -> f64 {
        let m = measure as usize;
        if let Some(slice) = self.measures.get(m) {
            let segs = &slice.segments;
            let idx = segs
                .partition_point(|s| s.frac_start <= frac)
                .saturating_sub(1);
            let seg = segs[idx];
            seg.ms_at_start + (frac - seg.frac_start) * slice.beats * 60_000.0 / seg.bpm
        } else {
            let extra = m - self.measures.len();
            let step = self.default_beats * 60_000.0 / self.tail_bpm;
            self.tail_ms + extra as f64 * step + frac * step
        }
    }

    fn end_time(&self, max_measure: u32) -> f64 {
        self.time_at(max_measure, 1.0)
    }
}

struct HeaderPass {
    player: Option<u8>,
    base: u32,
    title: String,
    subtitle: String,
    artist: String,
    subartist: String,
    genre: String,
    stagefile: String,
    banner: String,
    backbmp: String,
    comment: String,
    maker: String,
    bpm: f64,
    playlevel: Option<u8>,
    difficulty: Option<u8>,
    rank: Option<u8>,
    def_exrank: Option<f64>,
    exrank_defs: HashMap<u32, f64>,
    total: Option<f64>,
    vol_wav: Option<u8>,
    sample_settings: HashMap<u32, crate::sample_settings::SampleSettings>,
    wav_defs: HashMap<u32, String>,
    bpm_defs: HashMap<u32, f64>,
    stop_defs: HashMap<u32, f64>,
    scroll_defs: HashMap<u32, f64>,
    speed_defs: HashMap<u32, f64>,
    stp_events: Vec<TimingEvent>,
    bmp_defs: HashMap<u32, String>,
    lnobj: Option<u32>,
    lntype: u8,
    bga_extensions: crate::bga::Extensions,
}

impl Default for HeaderPass {
    fn default() -> Self {
        Self {
            player: None,
            base: 36,
            title: String::new(),
            subtitle: String::new(),
            artist: String::new(),
            subartist: String::new(),
            genre: String::new(),
            stagefile: String::new(),
            banner: String::new(),
            backbmp: String::new(),
            comment: String::new(),
            maker: String::new(),
            bpm: 130.0,
            playlevel: None,
            difficulty: None,
            rank: None,
            def_exrank: None,
            exrank_defs: HashMap::new(),
            total: None,
            vol_wav: None,
            sample_settings: HashMap::new(),
            wav_defs: HashMap::new(),
            bpm_defs: HashMap::new(),
            stop_defs: HashMap::new(),
            scroll_defs: HashMap::new(),
            speed_defs: HashMap::new(),
            stp_events: Vec::new(),
            bmp_defs: HashMap::new(),
            lnobj: None,
            lntype: 1,
            bga_extensions: Default::default(),
        }
    }
}

impl HeaderPass {
    fn absorb_wavcmd(&mut self, value: &str) {
        let parts: Vec<_> = value.split_whitespace().collect();
        if let [command, id, value] = parts.as_slice() {
            if let (Some(id), Ok(value)) = (parse_id(id, self.base), value.parse::<u32>()) {
                let settings = self.sample_settings.entry(id).or_default();
                match *command {
                    "00" if value <= 127 => settings.pitch = Some(value as u8),
                    "01" => settings.volume = Some(value),
                    "02" => settings.length = Some(value),
                    _ => {}
                }
            }
        }
    }

    fn for_text(text: &str) -> Self {
        let base = header_lines(text)
            .filter_map(|line| {
                let (cmd, val) = split_cmd_value(line);
                cmd.eq_ignore_ascii_case("BASE")
                    .then(|| val.trim().parse::<u32>().ok())
                    .flatten()
                    .filter(|base| matches!(base, 36 | 62))
            })
            .last()
            .unwrap_or(36);
        Self {
            base,
            ..Self::default()
        }
    }

    fn absorb(&mut self, line: &str) -> Result<()> {
        let (cmd, val) = split_cmd_value(line);
        let up = cmd.to_ascii_uppercase();
        if bga_extensions::absorb(&mut self.bga_extensions, cmd, val, self.base) {
            return Ok(());
        }
        match up.as_str() {
            "PLAYER" => {
                if let Ok(value @ 1..=3) = val.trim().parse::<u8>() {
                    self.player = Some(value);
                }
            }
            "STP" => self.stp_events.extend(parse_stp(val)),
            "TITLE" => self.title = val.to_string(),
            "SUBTITLE" => self.subtitle = val.to_string(),
            "ARTIST" => self.artist = val.to_string(),
            "SUBARTIST" => self.subartist = val.to_string(),
            "GENRE" => self.genre = val.to_string(),
            "STAGEFILE" => self.stagefile = val.trim().to_string(),
            "BANNER" => self.banner = val.trim().to_string(),
            "BACKBMP" => self.backbmp = val.trim().to_string(),
            "COMMENT" => self.comment = crate::terminal_text::sanitize(val.trim()),
            "MAKER" => self.maker = val.to_string(),
            "BPM" => {
                self.bpm = parse_bpm(val)?;
            }
            "PLAYLEVEL" => {
                if let Ok(v) = val.trim().parse() {
                    self.playlevel = Some(v);
                }
            }
            "DIFFICULTY" => {
                if let Ok(v) = val.trim().parse() {
                    self.difficulty = Some(v);
                }
            }
            "DEFEXRANK" => {
                if let Some(v) = positive_percent(val) {
                    self.def_exrank = Some(v);
                }
            }
            "RANK" => {
                if let Ok(v) = val.trim().parse::<u8>() {
                    if v <= 4 {
                        self.rank = Some(v);
                    }
                }
            }
            "TOTAL" => {
                if let Ok(v) = val.trim().parse::<f64>() {
                    if v > 0.0 {
                        self.total = Some(v);
                    }
                }
            }
            "VOLWAV" => {
                if let Ok(v) = val.trim().parse::<u8>() {
                    if v <= 100 {
                        self.vol_wav = Some(v);
                    }
                }
            }
            "WAVCMD" => self.absorb_wavcmd(val),
            "LNOBJ" => {
                if let Some(id) = parse_id(val.trim(), self.base) {
                    if id > 0 {
                        self.lnobj = Some(id);
                    }
                }
            }
            "LNTYPE" => {
                if let Ok(v) = val.trim().parse::<u8>() {
                    if (1..=2).contains(&v) {
                        self.lntype = v;
                    }
                }
            }
            _ => {
                if let Some((rest, definitions)) = up
                    .strip_prefix("SCROLL")
                    .and_then(|_| cmd.get(6..))
                    .map(|rest| (rest, &mut self.scroll_defs))
                    .or_else(|| {
                        up.strip_prefix("SPEED")
                            .and_then(|_| cmd.get(5..))
                            .map(|rest| (rest, &mut self.speed_defs))
                    })
                {
                    if rest.len() == 2 {
                        if let (Some(id), Ok(value)) =
                            (parse_id(rest, self.base), val.trim().parse::<f64>())
                        {
                            if id != 0 && value.is_finite() {
                                definitions.insert(id, value);
                            }
                        }
                    }
                } else if let Some(rest) = up.strip_prefix("EXWAV").and_then(|_| cmd.get(5..)) {
                    if let (Some(id), Some((settings, path))) = (
                        parse_id(rest, self.base),
                        crate::sample_settings::SampleSettings::parse_exwav(val),
                    ) {
                        let current = self.sample_settings.entry(id).or_default();
                        current.pan = settings.pan;
                        current.attenuation = settings.attenuation;
                        current.frequency = settings.frequency;
                        self.wav_defs.insert(id, path.to_string());
                    }
                } else if let Some(rest) = up.strip_prefix("EXRANK").and_then(|_| cmd.get(6..)) {
                    if rest.len() == 2 {
                        if let (Some(id), Some(v)) =
                            (parse_id(rest, self.base), positive_percent(val))
                        {
                            if id > 0 {
                                self.exrank_defs.insert(id, v);
                            }
                        }
                    }
                } else if let Some(rest) = up.strip_prefix("WAV").and_then(|_| cmd.get(3..)) {
                    if let Some(id) = parse_id(rest, self.base) {
                        self.wav_defs.insert(id, val.trim().to_string());
                        if let Some(settings) = self.sample_settings.get_mut(&id) {
                            settings.pan = 0;
                            settings.attenuation = 0;
                            settings.frequency = None;
                        }
                    }
                } else if let Some(rest) = up
                    .strip_prefix("BPM")
                    .and_then(|_| cmd.get(3..))
                    .or_else(|| up.strip_prefix("EXBPM").and_then(|_| cmd.get(5..)))
                {
                    if !rest.is_empty() {
                        if let Some(id) = parse_id(rest, self.base) {
                            self.bpm_defs.insert(id, parse_bpm(val)?);
                        }
                    }
                } else if let Some(rest) = up.strip_prefix("STOP").and_then(|_| cmd.get(4..)) {
                    if !rest.is_empty() {
                        if let (Some(id), Ok(v)) =
                            (parse_id(rest, self.base), val.trim().parse::<f64>())
                        {
                            if v > 0.0 {
                                self.stop_defs.insert(id, v);
                            }
                        }
                    }
                } else if let Some(rest) = up.strip_prefix("BMP").and_then(|_| cmd.get(3..)) {
                    if !rest.is_empty() {
                        if let Some(id) = parse_id(rest, self.base) {
                            self.bmp_defs.insert(id, val.trim().to_string());
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

fn parse_bpm(value: &str) -> Result<f64> {
    let bpm: f64 = value.trim().parse().context("Invalid BMS BPM")?;
    ensure!(
        bpm.is_finite() && bpm > 0.0,
        "BMS BPM must be finite and greater than zero"
    );
    Ok(bpm)
}

#[derive(Default)]
struct LaneSet(HashSet<usize>);

impl LaneSet {
    fn observe_lane(&mut self, lane: usize) {
        self.0.insert(lane);
    }
    fn count(&self) -> usize {
        let scratch = self.0.contains(&7);
        let max = self
            .0
            .iter()
            .copied()
            .filter(|lane| *lane != 7)
            .max()
            .unwrap_or(3);
        if scratch {
            return if max >= 5 { 8 } else { 6 };
        }
        if max >= 5 {
            7
        } else if max >= 3 {
            5
        } else {
            4
        }
    }
}

fn random_seed() -> u64 {
    std::env::var("TAPLINE_RANDOM_SEED")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or_else(|| {
            use std::hash::{BuildHasher, Hasher};
            std::collections::hash_map::RandomState::new()
                .build_hasher()
                .finish()
        })
}

fn resolve_control_flow(text: &str, seed: u64) -> String {
    #[derive(Clone, Copy)]
    struct IfLevel {
        matched_yet: bool,
        included: bool,
    }
    #[derive(Default)]
    struct SwitchLevel {
        pick: Option<u32>,
        included: bool,
        skipped: bool,
        default: bool,
    }
    let mut switch_stack: Vec<SwitchLevel> = Vec::new();
    let mut rng = seed.wrapping_add(0x9E3779B97F4A7C15);
    let mut random_stack: Vec<u32> = Vec::new();
    let mut if_stack: Vec<IfLevel> = Vec::new();
    let is_active = |ifs: &[IfLevel], switches: &[SwitchLevel]| {
        ifs.iter().all(|l| l.included) && switches.iter().all(|l| l.included)
    };
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        let trimmed = line.trim().trim_start_matches('\u{feff}');
        if let Some(body) = trimmed.strip_prefix('#') {
            let (head, rest) = split_first_word(body);
            let up = head.to_ascii_uppercase();
            match up.as_str() {
                "SWITCH" | "SETSWITCH" => {
                    let pick = rest.trim().parse::<u32>().ok().filter(|&n| n > 0);
                    let pick = pick.filter(|_| is_active(&if_stack, &switch_stack));
                    switch_stack.push(SwitchLevel {
                        pick: pick.map(|n| {
                            if up == "SWITCH" {
                                xorshift_pick(&mut rng, n)
                            } else {
                                n
                            }
                        }),
                        ..SwitchLevel::default()
                    });
                    continue;
                }
                "CASE" | "DEF" | "SKIP" => {
                    if if_stack.iter().all(|l| l.included) {
                        if let Some(top) = switch_stack.last_mut() {
                            if !top.skipped && !top.default {
                                match up.as_str() {
                                    "CASE" => {
                                        top.included |= top.pick.is_some_and(|p| {
                                            rest.trim().parse::<u32>().ok() == Some(p)
                                        });
                                    }
                                    "DEF" => {
                                        top.included = top.pick.is_some();
                                        top.default = true;
                                    }
                                    _ if top.included => {
                                        top.included = false;
                                        top.skipped = true;
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                    continue;
                }
                "ENDSW" => {
                    switch_stack.pop();
                    continue;
                }
                "RANDOM" => {
                    if is_active(&if_stack, &switch_stack) {
                        if let Ok(n) = rest.trim().parse::<u32>() {
                            if n > 0 {
                                random_stack.push(xorshift_pick(&mut rng, n));
                            }
                        }
                    }
                    continue;
                }
                "SETRANDOM" => {
                    if is_active(&if_stack, &switch_stack) {
                        if let Ok(n) = rest.trim().parse::<u32>() {
                            random_stack.push(n);
                        }
                    }
                    continue;
                }
                "ENDRANDOM" => {
                    if is_active(&if_stack, &switch_stack) {
                        random_stack.pop();
                    }
                    continue;
                }
                "IF" => {
                    let ok = random_stack
                        .last()
                        .is_some_and(|&p| rest.trim().parse::<u32>().ok() == Some(p));
                    if_stack.push(IfLevel {
                        matched_yet: ok,
                        included: ok,
                    });
                    continue;
                }
                "ELSEIF" => {
                    if let Some(top) = if_stack.last_mut() {
                        if top.matched_yet {
                            top.included = false;
                        } else {
                            let ok = random_stack
                                .last()
                                .is_some_and(|&p| rest.trim().parse::<u32>().ok() == Some(p));
                            top.included = ok;
                            top.matched_yet = ok;
                        }
                    }
                    continue;
                }
                "ELSE" => {
                    if let Some(top) = if_stack.last_mut() {
                        top.included = !top.matched_yet;
                        top.matched_yet = true;
                    }
                    continue;
                }
                "ENDIF" | "END" => {
                    if_stack.pop();
                    continue;
                }
                _ => {}
            }
        }
        if is_active(&if_stack, &switch_stack) {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

fn xorshift_pick(state: &mut u64, n: u32) -> u32 {
    let mut x = *state;
    if x == 0 {
        x = 0xdeadbeef;
    }
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    (x % n as u64) as u32 + 1
}

fn split_first_word(s: &str) -> (&str, &str) {
    match s.find(char::is_whitespace) {
        Some(i) => (&s[..i], &s[i..]),
        None => (s, ""),
    }
}

fn read_text(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot read {}", path.display()))?;
    Ok(decode_text(&bytes))
}

fn header_lines(text: &str) -> impl Iterator<Item = &str> {
    text.lines().filter_map(|line| {
        let line = line.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
        line.strip_prefix('#').filter(|s| !s.is_empty())
    })
}

fn decode_text(bytes: &[u8]) -> String {
    if let Ok(s) = std::str::from_utf8(bytes) {
        return s.to_string();
    }
    encoding_rs::SHIFT_JIS.decode(bytes).0.into_owned()
}

fn parse_channel_line(body: &str) -> Option<(u32, String, String)> {
    if body.len() < 7 || !body.is_ascii() {
        return None;
    }
    let colon = body.find(':')?;
    if colon != 5 {
        return None;
    }
    let measure = body.get(0..3)?.parse::<u32>().ok()?;
    let channel = body.get(3..5)?.to_ascii_uppercase();
    if !channel.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    let data = body.get(colon + 1..)?.trim().to_string();
    Some((measure, channel, data))
}

fn split_cmd_value(body: &str) -> (&str, &str) {
    match body.find(char::is_whitespace) {
        Some(i) => (&body[..i], body[i..].trim_start()),
        None => (body, ""),
    }
}

fn parse_id(value: &str, base: u32) -> Option<u32> {
    if base == 36 {
        return u32::from_str_radix(value, 36).ok();
    }
    if value.is_empty() {
        return None;
    }
    value.bytes().try_fold(0u32, |id, byte| {
        let digit = match byte {
            b'0'..=b'9' => byte - b'0',
            b'A'..=b'Z' => byte - b'A' + 10,
            b'a'..=b'z' => byte - b'a' + 36,
            _ => return None,
        };
        id.checked_mul(base)?.checked_add(u32::from(digit))
    })
}

fn parse_slots(data: &str, base: u32) -> Vec<u32> {
    let bytes = data.as_bytes();
    let take = bytes.len() - (bytes.len() % 2);
    (0..take)
        .step_by(2)
        .map(|i| {
            std::str::from_utf8(&bytes[i..i + 2])
                .ok()
                .and_then(|s| parse_id(s, base))
                .unwrap_or(0)
        })
        .collect()
}

fn channel_to_lane(ch: &str) -> Option<usize> {
    match ch {
        "11" => Some(0),
        "12" => Some(1),
        "13" => Some(2),
        "14" => Some(3),
        "15" => Some(4),
        "16" => Some(7),
        "17" => None, // Free zone / pedal is not a playable lane.
        "18" => Some(5),
        "19" => Some(6),
        _ => None,
    }
}

struct AssetIndex {
    exact: HashMap<PathBuf, PathBuf>,
    folded: HashMap<String, PathBuf>,
}

impl AssetIndex {
    fn new(dir: &Path) -> Self {
        let mut assets = Self {
            exact: HashMap::new(),
            folded: HashMap::new(),
        };
        assets.scan(dir, Path::new(""));
        assets
    }

    fn resolve(&self, defs: &HashMap<u32, String>, extensions: &[&str]) -> HashMap<u32, PathBuf> {
        defs.iter()
            .filter_map(|(id, name)| {
                let normalized = name.replace('\\', "/");
                let path: PathBuf = Path::new(&normalized)
                    .components()
                    .filter(|component| *component != std::path::Component::CurDir)
                    .collect();
                self.find(&path)
                    .or_else(|| {
                        extensions
                            .iter()
                            .find_map(|ext| self.find(&path.with_extension(ext)))
                    })
                    .map(|path| (*id, path.clone()))
            })
            .collect()
    }

    fn find(&self, path: &Path) -> Option<&PathBuf> {
        self.exact
            .get(path)
            .or_else(|| self.folded.get(&path.to_string_lossy().to_lowercase()))
    }

    fn scan(&mut self, dir: &Path, relative: &Path) {
        let Ok(entries) = std::fs::read_dir(dir.join(relative)) else {
            return;
        };
        let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = relative.join(entry.file_name());
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                self.scan(dir, &path);
            } else if entry.path().is_file() {
                self.folded
                    .entry(path.to_string_lossy().to_lowercase())
                    .or_insert_with(|| entry.path());
                self.exact.insert(path, entry.path());
            }
        }
    }
}

fn resolve_wavs(assets: &AssetIndex, defs: &HashMap<u32, String>) -> HashMap<u32, PathBuf> {
    assets.resolve(defs, &["wav", "ogg", "mp3", "flac"])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tempdir() -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let dir = std::env::temp_dir().join(format!("tapline-bms-test-{}-{}", pid, n));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn scroll_and_speed_preserve_judgment_and_audio_timing_with_stops() {
        let dir = tempdir();
        let path = dir.join("scroll.bms");
        let source =
            "#BPM 120\n#DEFEXRANK 75\n#EXRANK01 50\n#001A0:01\n#STP 000.500 250\n#STOP01 48\n#00109:01\n#00111:0101\n#00251:0101\n#00101:01\n#001D2:01\n";
        std::fs::write(&path, source).unwrap();
        let original = load(&path, 300.0).unwrap();
        std::fs::write(
            &path,
            format!("{source}#SCROLLzz 0.5\n#SPEED01 2\n#001SC:zz\n#002SP:01\n"),
        )
        .unwrap();
        let chart = load(&path, 300.0).unwrap();
        assert_eq!(chart.def_exrank, Some(75.0));
        assert_eq!(chart.exrank_events.len(), 1);
        assert_eq!(chart.judgment_scale(chart.notes[0].time_ms), 0.5);
        assert_eq!(chart.scroll.len(), 1);
        assert_eq!(chart.scroll[0].time_ms, 3050.0);
        assert_eq!(chart.scroll[0].multiplier, 0.5);
        assert_eq!(chart.speed[0].multiplier, 2.0);
        assert_eq!(chart.duration_ms, original.duration_ms);
        assert_eq!(chart.bgm[0].time_ms, original.bgm[0].time_ms);
        assert_eq!(chart.mines[0].time_ms, original.mines[0].time_ms);
        for (note, expected) in chart.notes.iter().zip(&original.notes) {
            assert_eq!(note.time_ms, expected.time_ms);
            assert_eq!(note.end_ms, expected.end_ms);
        }
        let time = chart.notes[0].time_ms;
        let mut game = crate::game::Game::new(chart);
        game.hit(0, time);
        assert_eq!(game.perfect, 1);
    }

    #[test]
    fn scroll_and_speed_preserve_sparse_bme_double_play_layout() {
        let dir = tempdir();
        let path = dir.join("scroll.bme");
        std::fs::write(
            &path,
            "#PLAYER 3\n#BPM 120\n#SCROLL01 0.5\n#SPEED01 2\n#000SC:01\n#000SP:01\n#00011:01\n#00021:01\n",
        )
        .unwrap();
        let chart = load(&path, 300.0).unwrap();
        assert_eq!(chart.lane_count, 14);
        assert_eq!(
            chart.notes.iter().map(|note| note.lane).collect::<Vec<_>>(),
            vec![0, 7]
        );
        assert_eq!(chart.scroll[0].multiplier, 0.5);
        assert_eq!(chart.speed[0].multiplier, 2.0);
    }

    #[test]
    fn scroll_and_speed_preserve_base62_ids_in_pms_charts() {
        let dir = tempdir();
        let path = dir.join("scroll.pms");
        std::fs::write(
            &path,
            "#BASE 62\n#BPM 120\n#scroll0A 0.5\n#SCROLL0a -1\n#speed0A 2\n#SPEED0a 0\n#000SC:0A0a\n#000SP:0A0a\n#00045:0a\n#00011:01\n",
        )
        .unwrap();
        let chart = load(&path, 300.0).unwrap();
        assert_eq!(chart.lane_count, 9);
        assert_eq!(chart.invisible_notes[0].keysound, 36);
        assert_eq!(chart.scroll.len(), 2);
        assert_eq!(chart.speed.len(), 2);
        assert_eq!(chart.scroll[0].time_ms, 300.0);
        assert_eq!(chart.scroll[1].time_ms, 1300.0);
        assert_eq!(chart.scroll[0].multiplier, 0.5);
        assert_eq!(chart.scroll[1].multiplier, -1.0);
        assert_eq!(chart.speed[0].multiplier, 2.0);
        assert_eq!(chart.speed[1].multiplier, 0.0);
    }

    #[test]
    fn scroll_definitions_allow_zero_and_reverse_but_reject_nonfinite_values() {
        let mut pass = HeaderPass::default();
        [
            "SCROLL01 0",
            "SCROLL02 -1",
            "SCROLL03 NaN",
            "SPEED01 inf",
            "SPEED02 0.5",
            "SCROLL00 2",
            "SCROLL001 3",
        ]
        .iter()
        .for_each(|line| pass.absorb(line).unwrap());
        assert_eq!(pass.scroll_defs.len(), 2);
        assert_eq!(pass.scroll_defs[&1], 0.0);
        assert_eq!(pass.scroll_defs[&2], -1.0);
        assert_eq!(pass.speed_defs.len(), 1);
    }

    #[test]
    fn extended_wav_parses_flags_in_order_and_keeps_filename_spaces() {
        let mut pass = HeaderPass::default();
        pass.absorb("exwav0E vFp -2000 22050 -10000 sound file.wav")
            .unwrap();
        pass.absorb("WAVCMD 00 0E 72").unwrap();
        pass.absorb("WAVCMD 01 0E 50").unwrap();
        pass.absorb("WAVCMD 02 0E 200").unwrap();
        let settings = pass.sample_settings[&14].clone();
        assert_eq!(pass.wav_defs[&14], "sound file.wav");
        assert_eq!(settings.pan, -10000);
        assert_eq!(settings.attenuation, -2000);
        assert_eq!(settings.frequency, Some(22050));
        assert_eq!(settings.pitch, Some(72));
        assert_eq!(settings.volume, Some(50));
        assert_eq!(settings.length, Some(200));
        pass.absorb("EXWAV0E p 10001 invalid.wav").unwrap();
        pass.absorb("WAVCMD 00 0E 128").unwrap();
        assert_eq!(pass.sample_settings[&14], settings);
    }

    #[test]
    fn load_extended_wav_combines_base62_settings_volume_and_exrank_events() {
        let dir = tempdir();
        let path = dir.join("extended-base62.bms");
        ["upper.wav", "lower.wav"].iter().for_each(|name| {
            std::fs::write(dir.join(name), b"").unwrap();
        });
        std::fs::write(
            &path,
            "#BPM 120\n#DEFEXRANK 75\n#EXRANK0a 50\n#000A0:0a\n#SCROLL0A 0.5\n#SPEED0a 2\n#000SC:0A\n#000SP:0a\n#EXWAV0A p -10000 upper.wav\n#exwav0a v -2000 lower.wav\n#WAVCMD 00 0A 72\n#WAVCMD 01 0a 50\n#WAVCMD 02 0a 200\n#00011:0A0a\n#00097:80\n#00098:40\n#BASE 62\n",
        )
        .unwrap();
        let chart = load(&path, 0.0).unwrap();
        assert_eq!(chart.scroll[0].multiplier, 0.5);
        assert_eq!(chart.speed[0].multiplier, 2.0);
        assert_eq!(chart.wav_paths[&10], dir.join("upper.wav"));
        assert_eq!(chart.wav_paths[&36], dir.join("lower.wav"));
        assert_eq!(chart.sample_settings[&10].pan, -10000);
        assert_eq!(chart.sample_settings[&10].pitch, Some(72));
        assert_eq!(chart.sample_settings[&10].volume, None);
        assert_eq!(chart.sample_settings[&36].attenuation, -2000);
        assert_eq!(chart.sample_settings[&36].pitch, None);
        assert_eq!(chart.sample_settings[&36].volume, Some(50));
        assert_eq!(chart.sample_settings[&36].length, Some(200));
        assert_eq!(chart.notes[0].keysound, Some(10));
        assert_eq!(chart.notes[1].keysound, Some(36));
        assert_eq!(chart.volume_events.len(), 2);
        assert_eq!(chart.exrank_defs[&36], 50.0);
        assert_eq!(chart.judgment_scale(0.0), 0.5);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn parse_slots_distinguishes_base62_letters_and_keeps_base36_case_insensitive() {
        assert_eq!(parse_slots("0A0aZZzz10", 62), vec![10, 36, 2205, 3843, 62]);
        assert_eq!(parse_slots("0A0aZZzz10", 36), vec![10, 10, 1295, 1295, 36]);
        assert_eq!(parse_slots("!!é0?", 62), vec![0, 0, 0]);
        assert_eq!(parse_id("", 62), None);
        assert_eq!(parse_id("zzzzzzzz", 62), None);
    }

    #[test]
    fn header_prescan_uses_the_last_supported_base_in_active_control_flow() {
        let text = resolve_control_flow(
            "#BASE 62\n#SETRANDOM 1\n#IF 2\n#BASE 36\n#ENDIF\n#BASE 16\n",
            0,
        );
        assert_eq!(HeaderPass::for_text(&text).base, 62);
        assert_eq!(HeaderPass::for_text("#BASE 62\n#BASE 36").base, 36);
        assert_eq!(HeaderPass::for_text("#BASE invalid").base, 36);
    }

    #[test]
    fn load_prescans_base62_and_preserves_object_ids_across_channels() {
        let dir = tempdir();
        let path = dir.join("base62.bms");
        ["upper.wav", "lower.wav", "upper.png", "lower.png"]
            .iter()
            .for_each(|name| std::fs::write(dir.join(name), b"").unwrap());
        std::fs::write(
            &path,
            "#BPM 60
#wav0A upper.wav
#WaV0a lower.wav
#BMP0A upper.png
#bmp0a lower.png
#BPM0A 120
#bpm0a 240
#STOP0A 48
#stop0a 96
#LNOBJ 0a
#00011:0A0a
#00021:0a
#00052:0a0A
#00062:0a0A
#00001:0A0a
#00004:0A0a
#00008:0A0a
#00009:0A0a
#00003:0a
#000D1:0a
#bAsE 62
",
        )
        .unwrap();
        let chart = load(&path, 0.0).unwrap();
        assert_eq!(chart.wav_paths[&10], dir.join("upper.wav"));
        assert_eq!(chart.wav_paths[&36], dir.join("lower.wav"));
        assert_eq!(chart.bmp_paths[&10], dir.join("upper.png"));
        assert_eq!(chart.bmp_paths[&36], dir.join("lower.png"));
        assert_eq!(
            chart.bgm.iter().map(|e| e.keysound).collect::<Vec<_>>(),
            vec![10, 36]
        );
        assert_eq!(
            chart.bga.iter().map(|e| e.bmp_id).collect::<Vec<_>>(),
            vec![10, 36]
        );
        assert!(chart
            .notes
            .iter()
            .any(|n| n.keysound == Some(10) && n.end_ms.is_some()));
        assert!(chart
            .notes
            .iter()
            .any(|n| n.keysound == Some(36) && n.end_ms.is_some()));
        assert!(chart.p2_notes.iter().all(|n| n.keysound == Some(36)));
        assert_eq!(chart.mines[0].damage, 10);
        assert_eq!(read_meta(&path).unwrap().lane_count, chart.lane_count);
    }

    #[test]
    fn base62_timing_references_are_case_sensitive_and_direct_bpm_stays_hexadecimal() {
        let text = "#BPM0A 120\n#bpm0a 240\n#STOP0A 48\n#stop0a 96\n#BASE 62";
        let mut pass = HeaderPass::for_text(text);
        header_lines(text).for_each(|line| pass.absorb(line).unwrap());
        assert_eq!(pass.bpm_defs[&10], 120.0);
        assert_eq!(pass.bpm_defs[&36], 240.0);
        assert_eq!(pass.stop_defs[&10], 48.0);
        assert_eq!(pass.stop_defs[&36], 96.0);
        let raw = vec![
            (0, "08".into(), "0A0a".into()),
            (0, "09".into(), "0A0a".into()),
        ];
        let events = collect_timing_events(
            &raw,
            &pass.bpm_defs,
            &pass.stop_defs,
            &pass.stp_events,
            pass.base,
        );
        assert!(matches!(events[0].kind, TimingEventKind::Bpm(120.0)));
        assert!(matches!(events[2].kind, TimingEventKind::Bpm(240.0)));
        assert!(matches!(events[1].kind, TimingEventKind::Stop(48.0)));
        assert!(matches!(events[3].kind, TimingEventKind::Stop(96.0)));
        let hex = collect_timing_events(
            &[(0, "03".into(), "0a".into())],
            &pass.bpm_defs,
            &pass.stop_defs,
            &pass.stp_events,
            pass.base,
        );
        assert!(matches!(hex[0].kind, TimingEventKind::Bpm(10.0)));
    }

    #[test]
    fn load_base62_keeps_uppercase_and_lowercase_notes_separate() {
        let dir = tempdir();
        let path = dir.join("notes.bms");
        std::fs::write(&path, "#00011:0A0a\n#BASE 62").unwrap();
        let chart = load(&path, 0.0).unwrap();
        assert_eq!(
            chart.notes.iter().map(|n| n.keysound).collect::<Vec<_>>(),
            vec![Some(10), Some(36)]
        );
    }

    #[test]
    fn load_keeps_default_and_explicit_base36_ids_case_insensitive() {
        let dir = tempdir();
        let path = dir.join("base36.bms");
        std::fs::write(dir.join("last.wav"), b"").unwrap();
        for base in ["", "#BASE 36"] {
            std::fs::write(
                &path,
                format!("#WAV0A first.wav\n#WAV0a last.wav\n#00011:0A0a\n{base}"),
            )
            .unwrap();
            let chart = load(&path, 0.0).unwrap();
            assert_eq!(chart.wav_paths.len(), 1);
            assert_eq!(chart.wav_paths[&10], dir.join("last.wav"));
            assert_eq!(
                chart.notes.iter().map(|n| n.keysound).collect::<Vec<_>>(),
                vec![Some(10), Some(10)]
            );
        }
    }

    #[test]
    fn pms_maps_all_buttons_for_visible_long_lnobj_and_mine_channels() {
        let dir = tempdir();
        let families = [
            ("11 12 13 14 15 22 23 24 25", "01", false),
            ("51 52 53 54 55 62 63 64 65", "0101", true),
            ("11 12 13 14 15 22 23 24 25", "01ZZ", true),
            ("D1 D2 D3 D4 D5 E2 E3 E4 E5", "64", false),
        ];
        for extension in ["pms", "PMS"] {
            let path = dir.join(format!("song.{extension}"));
            for (channels, slots, long) in families {
                let lines = channels
                    .split_whitespace()
                    .map(|channel| format!("#001{channel}:{slots}\n"))
                    .collect::<String>();
                std::fs::write(&path, format!("#BPM 60\n#LNOBJ ZZ\n#00101:01\n{lines}")).unwrap();
                let chart = load(&path, 500.0).unwrap();
                assert_eq!(chart.lane_count, 9);
                assert_eq!(read_meta(&path).unwrap().lane_count, 9);
                assert_eq!(chart.keys, keys_for(9));
                assert!(chart.p2_notes.is_empty());
                assert!(chart.p2_mines.is_empty());
                assert_eq!(chart.bgm.len(), 1);
                let mut lanes = if channels.starts_with('D') {
                    assert!(chart.notes.is_empty());
                    assert!(chart.mines.iter().all(|mine| mine.damage == 220));
                    chart.mines.iter().map(|mine| mine.lane).collect::<Vec<_>>()
                } else {
                    assert!(chart.notes.iter().all(|note| note.keysound == Some(1)));
                    assert!(chart.notes.iter().all(|note| note.time_ms == 4500.0));
                    assert!(chart
                        .notes
                        .iter()
                        .all(|note| note.end_ms == long.then_some(6500.0)));
                    chart.notes.iter().map(|note| note.lane).collect::<Vec<_>>()
                };
                lanes.sort_unstable();
                assert_eq!(lanes, (0..9).collect::<Vec<_>>());
                let mut game = crate::game::Game::new(chart);
                (0..9).for_each(|lane| {
                    game.hit(lane, 4500.0);
                    if long {
                        game.release(lane, 6500.0);
                    }
                });
                if channels.starts_with('D') {
                    assert_eq!(game.miss, 0);
                    assert!(game.chart.mines.iter().all(|mine| mine.exploded));
                } else {
                    assert_eq!(game.perfect, if long { 18 } else { 9 });
                    assert!(game.chart.notes.iter().all(|note| note.hit));
                }
            }
        }
    }

    #[test]
    fn sparse_pms_and_bme_reserve_their_declared_lanes() {
        let dir = tempdir();
        let cases = [
            ("#00111:01", 4, 1, 0),
            ("#00115:01", 5, 1, 0),
            ("#00119:01", 7, 1, 0),
            ("#00119:01\n#00129:01", 14, 2, 0),
            ("#00115:01\n#00125:01", 10, 2, 0),
            ("#00116:01", 6, 1, 0),
            ("#00119:01\n#00116:01", 8, 2, 0),
            ("#00116:01\n#00126:01", 12, 2, 0),
            ("#00119:01\n#00129:01\n#00116:01\n#00126:01", 16, 4, 0),
        ];
        for extension in ["bms", "bme"] {
            for (channels, count, notes, p2) in cases {
                let path = dir.join(format!("song.{extension}"));
                std::fs::write(&path, format!("#BPM 120\n{channels}\n")).unwrap();
                let chart = load(&path, 0.0).unwrap();
                let expected = match (extension, count) {
                    ("bme", 4 | 5) => 7,
                    ("bme", 6) => 8,
                    ("bme", 10) => 14,
                    ("bme", 12) => 16,
                    _ => count,
                };
                assert_eq!(chart.lane_count, expected);
                assert_eq!(read_meta(&path).unwrap().lane_count, expected);
                assert_eq!(chart.notes.len(), notes);
                assert_eq!(chart.p2_notes.len(), p2);
            }
        }
        let path = dir.join("sparse.pms");
        ["11", "24", "25"].iter().for_each(|channel| {
            std::fs::write(&path, format!("#BPM 120\n#001{channel}:01\n")).unwrap();
            let chart = load(&path, 0.0).unwrap();
            assert_eq!(chart.lane_count, 9);
            assert_eq!(read_meta(&path).unwrap().lane_count, 9);
            let expected_lane = match *channel {
                "24" => 7,
                "25" => 8,
                _ => 0,
            };
            assert_eq!(chart.notes[0].lane, expected_lane);
        });
    }

    #[test]
    fn base62_preserves_pms_invisible_ids_and_exbpm_aliases() {
        let dir = tempdir();
        let path = dir.join("base62.pms");
        std::fs::write(
            &path,
            "#BASE 62\n#BPM 60\n#EXBPM0A 120\n#exbpm0a 240\n#00008:0A0a\n#00011:0A0a\n#00045:0A0a\n#00045:000A\n",
        )
        .unwrap();
        let chart = load_with_seed(&path, 0.0, 42).unwrap();
        assert_eq!(read_meta_with_seed(&path, 42).unwrap().lane_count, 9);
        assert_eq!(chart.lane_count, 9);
        assert_eq!(chart.notes[0].keysound, Some(10));
        assert_eq!(chart.notes[1].keysound, Some(36));
        assert_eq!(chart.notes[1].time_ms, 1000.0);
        assert_eq!(chart.invisible_notes.len(), 2);
        assert!(chart.invisible_notes.iter().all(|note| note.lane == 8));
        assert!(chart.invisible_notes.iter().all(|note| note.keysound == 10));
        assert!(chart.p2_invisible_notes.is_empty());
        let text = "#BASE 62\n#EXBPM0A 120\n#exbpm0a 240";
        let mut pass = HeaderPass::for_text(text);
        header_lines(text).for_each(|line| pass.absorb(line).unwrap());
        assert_eq!(pass.bpm_defs[&10], 120.0);
        assert_eq!(pass.bpm_defs[&36], 240.0);
    }

    #[test]
    fn parse_slots_reads_base36_pairs() {
        assert_eq!(parse_slots("00010203", 36), vec![0, 1, 2, 3]);
    }

    #[test]
    fn parse_slots_handles_base36_letters() {
        assert_eq!(parse_slots("0AZZ10", 36), vec![10, 1295, 36]);
    }

    #[test]
    fn parse_slots_drops_a_trailing_odd_char() {
        assert_eq!(parse_slots("010", 36), vec![1]);
    }

    #[test]
    fn parse_slots_treats_bad_hex_as_zero() {
        assert_eq!(parse_slots("!!!!", 36), vec![0, 0]);
    }

    #[test]
    fn channel_to_lane_covers_p1_visible_channels() {
        assert_eq!(channel_to_lane("11"), Some(0));
        assert_eq!(channel_to_lane("12"), Some(1));
        assert_eq!(channel_to_lane("13"), Some(2));
        assert_eq!(channel_to_lane("14"), Some(3));
        assert_eq!(channel_to_lane("15"), Some(4));
        assert_eq!(channel_to_lane("18"), Some(5));
        assert_eq!(channel_to_lane("19"), Some(6));
    }

    #[test]
    fn channel_to_lane_ignores_bgm_and_unknown_channels() {
        assert_eq!(channel_to_lane("01"), None);
        assert_eq!(channel_to_lane("16"), Some(7));
        assert_eq!(channel_to_lane("21"), None);
        assert_eq!(channel_to_lane("D1"), None);
    }

    #[test]
    fn parse_channel_line_extracts_measure_channel_data() {
        let parsed = parse_channel_line("00111:0100").unwrap();
        assert_eq!(parsed.0, 1);
        assert_eq!(parsed.1, "11");
        assert_eq!(parsed.2, "0100");
    }

    #[test]
    fn parse_channel_line_uppercases_hex_channels() {
        let parsed = parse_channel_line("001d1:0100").unwrap();
        assert_eq!(parsed.1, "D1");
    }

    #[test]
    fn parse_channel_line_rejects_short_or_missing_colon() {
        assert!(parse_channel_line("TITLE Foo").is_none());
        assert!(parse_channel_line("0011").is_none());
        assert!(parse_channel_line("001110100").is_none());
    }

    #[test]
    fn parse_channel_line_rejects_non_numeric_measure() {
        assert!(parse_channel_line("abc11:0100").is_none());
    }

    #[test]
    fn split_cmd_value_splits_at_first_whitespace() {
        assert_eq!(split_cmd_value("TITLE Foo Bar"), ("TITLE", "Foo Bar"));
        assert_eq!(split_cmd_value("BPM   180"), ("BPM", "180"));
    }

    #[test]
    fn split_cmd_value_returns_empty_value_when_no_whitespace() {
        assert_eq!(split_cmd_value("SOLO"), ("SOLO", ""));
    }

    #[test]
    fn header_lines_strips_hash_and_trims_bom_and_whitespace() {
        let text = "\u{feff}#TITLE Foo\n\n  #BPM 130  \nnot a header\n#\n";
        let collected: Vec<_> = header_lines(text).collect();
        assert_eq!(collected, vec!["TITLE Foo", "BPM 130"]);
    }

    #[test]
    fn decode_text_prefers_utf8() {
        assert_eq!(decode_text("hello".as_bytes()), "hello");
    }

    #[test]
    fn decode_text_falls_back_to_shift_jis_for_non_utf8() {
        // "あ" in Shift-JIS is 0x82 0xA0.
        let decoded = decode_text(&[0x82, 0xA0]);
        assert_eq!(decoded, "あ");
    }

    #[test]
    fn header_pass_absorbs_the_standard_headers() {
        let mut p = HeaderPass::default();
        p.absorb("TITLE Foo Song").unwrap();
        p.absorb("ARTIST Bar").unwrap();
        p.absorb("BPM 180").unwrap();
        p.absorb("PLAYLEVEL 7").unwrap();
        p.absorb("DIFFICULTY 3").unwrap();
        p.absorb("WAV01 kick.wav").unwrap();
        p.absorb("WAVZZ hihat.ogg").unwrap();
        assert_eq!(p.title, "Foo Song");
        assert_eq!(p.artist, "Bar");
        assert_eq!(p.bpm, 180.0);
        assert_eq!(p.playlevel, Some(7));
        assert_eq!(p.difficulty, Some(3));
        assert_eq!(p.wav_defs.get(&1).unwrap(), "kick.wav");
        assert_eq!(p.wav_defs.get(&1295).unwrap(), "hihat.ogg");
    }

    #[test]
    fn header_pass_absorbs_optional_metadata_headers() {
        let mut p = HeaderPass::default();
        p.absorb("SUBTITLE ~an interlude~").unwrap();
        p.absorb("SUBARTIST arr. someone").unwrap();
        p.absorb("GENRE electronic").unwrap();
        p.absorb("STAGEFILE  cover.png ").unwrap();
        p.absorb("BANNER  banner.png ").unwrap();
        p.absorb("MAKER unhappychoice").unwrap();
        assert_eq!(p.subtitle, "~an interlude~");
        assert_eq!(p.subartist, "arr. someone");
        assert_eq!(p.genre, "electronic");
        assert_eq!(p.stagefile, "cover.png");
        assert_eq!(p.banner, "banner.png");
        assert_eq!(p.maker, "unhappychoice");
    }

    #[test]
    fn read_meta_surfaces_genre_subtitle_and_subartist() {
        let dir = tempdir();
        let path = dir.join("song.bms");
        std::fs::write(
            &path,
            "\
#TITLE Testing
#SUBTITLE ~beta~
#ARTIST alice
#SUBARTIST bob
#GENRE electronic
#BPM 130
#00111:0100
",
        )
        .unwrap();
        let m = read_meta(&path).unwrap();
        assert_eq!(m.subtitle, "~beta~");
        assert_eq!(m.subartist, "bob");
        assert_eq!(m.genre, "electronic");
    }

    #[test]
    fn header_pass_absorbs_rank_total_and_volwav() {
        let mut p = HeaderPass::default();
        p.absorb("RANK 2").unwrap();
        p.absorb("TOTAL 260").unwrap();
        p.absorb("VOLWAV 80").unwrap();
        assert_eq!(p.rank, Some(2));
        assert_eq!(p.total, Some(260.0));
        assert_eq!(p.vol_wav, Some(80));
    }

    #[test]
    fn header_pass_rejects_out_of_range_rank_total_volwav() {
        let mut p = HeaderPass::default();
        p.absorb("RANK 9").unwrap();
        p.absorb("TOTAL 0").unwrap();
        p.absorb("TOTAL -5").unwrap();
        p.absorb("VOLWAV 250").unwrap();
        p.absorb("VOLWAV nonsense").unwrap();
        assert_eq!(p.rank, None, "RANK > 4 is not a valid difficulty tier");
        assert_eq!(p.total, None, "TOTAL must be positive");
        assert_eq!(p.vol_wav, None, "VOLWAV is a 0..=100 percent");
    }

    #[test]
    fn timeline_defaults_every_measure_to_base_length() {
        // 240 BPM → 4 beats / (240/60) = 1 second per measure.
        let t = Timeline::build(0.0, 240.0, &HashMap::new(), &[], 3).unwrap();
        assert_eq!(t.time_at(0, 0.0), 0.0);
        assert_eq!(t.time_at(1, 0.0), 1000.0);
        assert_eq!(t.time_at(2, 0.0), 2000.0);
        assert_eq!(t.end_time(2), 3000.0);
    }

    #[test]
    fn timeline_scales_a_single_measure_and_shifts_the_rest() {
        let scales: HashMap<u32, f64> = [(1u32, 0.5)].into();
        let t = Timeline::build(100.0, 240.0, &scales, &[], 3).unwrap();
        assert_eq!(t.time_at(0, 0.0), 100.0);
        assert_eq!(t.time_at(1, 0.0), 1100.0);
        assert!((t.time_at(1, 1.0) - 1600.0).abs() < 1e-6);
        assert_eq!(t.time_at(2, 0.0), 1600.0);
    }

    #[test]
    fn timeline_extrapolates_past_the_max_measure() {
        let t = Timeline::build(0.0, 240.0, &HashMap::new(), &[], 1).unwrap();
        assert!((t.time_at(5, 0.0) - 5000.0).abs() < 1e-6);
    }

    #[test]
    fn timeline_applies_a_mid_measure_bpm_change() {
        // 120 BPM → 2 seconds per measure. Halfway through measure 0 the BPM
        // doubles to 240 → the second half plays in 500ms instead of 1000ms,
        // and measure 1 starts at 1500ms not 2000ms.
        let evs = vec![TimingEvent {
            measure: 0,
            frac: 0.5,
            kind: TimingEventKind::Bpm(240.0),
        }];
        let t = Timeline::build(0.0, 120.0, &HashMap::new(), &evs, 1).unwrap();
        assert!((t.time_at(0, 0.5) - 1000.0).abs() < 1e-6);
        assert!((t.time_at(1, 0.0) - 1500.0).abs() < 1e-6);
    }

    #[test]
    fn timeline_bpm_change_persists_into_later_measures() {
        // Change BPM at start of measure 1, ensure measure 2 uses the new tempo.
        let evs = vec![TimingEvent {
            measure: 1,
            frac: 0.0,
            kind: TimingEventKind::Bpm(240.0),
        }];
        let t = Timeline::build(0.0, 120.0, &HashMap::new(), &evs, 3).unwrap();
        assert!((t.time_at(1, 0.0) - 2000.0).abs() < 1e-6);
        assert!((t.time_at(2, 0.0) - 3000.0).abs() < 1e-6);
        assert!((t.time_at(3, 0.0) - 4000.0).abs() < 1e-6);
    }

    #[test]
    fn timeline_stop_delays_everything_after_the_stop_frac() {
        // 120 BPM → 2 seconds per measure. STOP of 48 ticks = 1 beat at 120 BPM = 500ms.
        // Halfway through measure 0 we pause 500ms. Then the second half of the
        // measure still plays at 120 BPM = 1000ms. Measure 1 starts at 2500ms.
        let evs = vec![TimingEvent {
            measure: 0,
            frac: 0.5,
            kind: TimingEventKind::Stop(48.0),
        }];
        let t = Timeline::build(0.0, 120.0, &HashMap::new(), &evs, 1).unwrap();
        assert!((t.time_at(0, 0.5) - 1500.0).abs() < 1e-6);
        assert!((t.time_at(1, 0.0) - 2500.0).abs() < 1e-6);
    }

    #[test]
    fn parse_slots_hex_reads_two_digit_hex_bpm_slots() {
        // 0x78 = 120, 0x64 = 100. Pairs: 00 78 64 00.
        assert_eq!(parse_slots_hex("00786400"), vec![0, 120, 100, 0]);
    }

    #[test]
    fn collect_timing_events_reads_bpm_and_stop_channels() {
        let raw = vec![
            (1u32, "03".to_string(), "0078".to_string()),
            (2u32, "08".to_string(), "0001".to_string()),
            (3u32, "09".to_string(), "0001".to_string()),
        ];
        let bpm_defs: HashMap<u32, f64> = [(1u32, 175.5)].into();
        let stop_defs: HashMap<u32, f64> = [(1u32, 96.0)].into();
        let evs = collect_timing_events(&raw, &bpm_defs, &stop_defs, &[], 36);
        assert_eq!(evs.len(), 3);
        assert!(matches!(evs[0].kind, TimingEventKind::Bpm(v) if (v - 120.0).abs() < 1e-6));
        assert!(matches!(evs[1].kind, TimingEventKind::Bpm(v) if (v - 175.5).abs() < 1e-6));
        assert!(matches!(evs[2].kind, TimingEventKind::Stop(v) if (v - 96.0).abs() < 1e-6));
    }

    #[test]
    fn collect_timing_events_ignores_references_without_definitions() {
        let raw = vec![
            (1u32, "08".to_string(), "0099".to_string()),
            (2u32, "09".to_string(), "0099".to_string()),
        ];
        let empty: HashMap<u32, f64> = HashMap::new();
        assert!(collect_timing_events(&raw, &empty, &empty, &[], 36).is_empty());
    }

    #[test]
    fn header_pass_absorbs_stop_definitions() {
        let mut p = HeaderPass::default();
        p.absorb("STOP01 96").unwrap();
        p.absorb("STOP02 -1").unwrap();
        p.absorb("STOPZZ 24").unwrap();
        assert_eq!(p.stop_defs.get(&1), Some(&96.0));
        assert!(!p.stop_defs.contains_key(&2));
        assert_eq!(p.stop_defs.get(&1295), Some(&24.0));
    }

    #[test]
    fn p2_channel_to_lane_mirrors_the_p1_map_on_2x_prefix() {
        assert_eq!(p2_channel_to_lane("21"), Some(0));
        assert_eq!(p2_channel_to_lane("25"), Some(4));
        assert_eq!(p2_channel_to_lane("28"), Some(5));
        assert_eq!(p2_channel_to_lane("29"), Some(6));
        assert_eq!(p2_channel_to_lane("26"), Some(7));
        assert_eq!(p2_channel_to_lane("11"), None);
    }

    #[test]
    fn p2_long_and_mine_channels_map_to_the_same_lanes_as_p2_normal() {
        assert_eq!(p2_long_channel_to_lane("61"), Some(0));
        assert_eq!(p2_long_channel_to_lane("69"), Some(6));
        assert_eq!(p2_mine_channel_to_lane("E1"), Some(0));
        assert_eq!(p2_mine_channel_to_lane("E9"), Some(6));
    }

    #[test]
    fn load_merges_p2_notes_mines_and_long_notes() {
        let dir = tempdir();
        let path = dir.join("song.bms");
        std::fs::write(
            &path,
            "\
#TITLE DP
#BPM 60
#00111:0100
#00121:0100
#00161:01000100
#001E1:0064
",
        )
        .unwrap();
        let chart = load(&path, 0.0).unwrap();
        assert_eq!(chart.lane_count, 8);
        assert_eq!(chart.notes.len(), 3);
        assert!(chart
            .notes
            .iter()
            .any(|n| n.lane == 4 && n.end_ms.is_some()));
        assert!(chart.p2_notes.is_empty());
        assert!(chart.p2_mines.is_empty());
        assert_eq!(chart.mines[0].lane, 4);
        assert_eq!(chart.mines[0].damage, 220);
    }

    #[test]
    fn load_folds_canonical_7k_double_play_into_a_single_14_lane_chart() {
        let dir = tempdir();
        let path = dir.join("song.bms");
        // Force 7-lane inference on both sides by touching the scratch/high
        // lanes: P1 channels 15/18/19 and P2 channels 25/28/29.
        std::fs::write(
            &path,
            "\
#TITLE DP-14K
#BPM 60
#00111:0100
#00115:0100
#00118:0100
#00119:0100
#00121:0100
#00125:0100
#00128:0100
#00129:0100
#00161:01000100
#001E9:0064
",
        )
        .unwrap();
        let chart = load(&path, 0.0).unwrap();
        assert_eq!(read_meta(&path).unwrap().lane_count, chart.lane_count);
        assert_eq!(chart.lane_count, 14, "P1 7K + P2 7K → single 14K chart");
        assert!(
            chart.p2_notes.is_empty(),
            "P2 pool is drained into the main pool"
        );
        // The lane-0 P2 note should be relocated to lane 7 (P1 lane_count offset).
        assert!(
            chart.notes.iter().any(|n| n.lane == 7),
            "P2 lane 0 should live at merged lane 7"
        );
        assert!(chart.notes.iter().any(|n| n.lane == 13));
        assert!(chart
            .notes
            .iter()
            .any(|n| n.lane == 7 && n.end_ms.is_some()));
        assert_eq!(chart.mines[0].lane, 13);
        assert!(chart.p2_mines.is_empty());
    }

    #[test]
    fn bga_switches_follow_materialized_lanes() {
        let dir = tempdir();
        for (extension, header, channels) in [
            ("bms", "", "11"),
            ("bms", "", "15"),
            ("bms", "", "16"),
            ("bme", "", "19"),
            ("bme", "", "16"),
            ("bms", "", "11 25"),
            ("bms", "", "11 29"),
            ("bms", "", "19 21"),
            ("bms", "#PLAYER 3", "15 25"),
            ("bms", "#PLAYER 3", "15 29"),
            ("bms", "#PLAYER 3", "19 25"),
            ("bms", "#PLAYER 3", "16 25"),
            ("bms", "#PLAYER 3", "15 26"),
            ("bms", "#PLAYER 3", "16 29"),
            ("bms", "#PLAYER 3", "19 26"),
            ("bms", "#PLAYER 3", "16 26"),
            ("bme", "#PLAYER 3", "19 29"),
            ("bme", "#PLAYER 3", "16 26"),
            ("pms", "", "11 25"),
        ] {
            let path = dir.join(format!("switch.{extension}"));
            let notes = channels
                .split_whitespace()
                .map(|channel| format!("#001{channel}:01\n"))
                .collect::<String>();
            for channel in [
                "11", "15", "16", "17", "18", "19", "21", "25", "26", "27", "28", "29",
            ] {
                let text = format!(
                    "#BPM 120\n{header}\n{notes}#SWBGA01 100:0:{channel}:1:255,0,0,0 01\n#000A5:01\n"
                );
                std::fs::write(&path, text).unwrap();
                let chart = load(&path, 0.0).unwrap();
                let expected = match channel {
                    "17" | "27" => None,
                    _ if channels.split_whitespace().any(|ch| ch == channel) => {
                        let index = channels
                            .split_whitespace()
                            .position(|ch| ch == channel)
                            .unwrap();
                        Some(chart.notes[index].lane)
                    }
                    _ => continue,
                };
                assert_eq!(
                    chart.bga_extensions.switches[&1].lane, expected,
                    "{extension} {header} {channels}: {channel}"
                );
                let mut playback = crate::bga::SwitchPlayback::default();
                for lane in 0..chart.lane_count {
                    playback.active = None;
                    playback.press(&chart, lane, 0.0);
                    assert_eq!(
                        playback.active.map(|(_, lane, _)| lane),
                        expected.filter(|expected| *expected == lane)
                    );
                }
            }
        }
    }

    #[test]
    fn double_play_merges_both_sides_and_matches_metadata() {
        let dir = tempdir();
        let path = dir.join("song.bms");
        let cases = [
            ("", "#00115:01", "#00125:01", 5, 5),
            ("#PLAYER 3", "#00115:01", "#00125:01", 5, 5),
            ("", "#00119:01", "#00125:01", 7, 5),
            ("", "#00115:01", "#00129:01", 5, 7),
            ("#PLAYER 3", "#00115:01", "", 5, 5),
            ("#PLAYER 2", "#00111:01", "", 5, 5),
            ("#PLAYER 3", "#00119:01", "#00125:01", 7, 5),
            ("#PLAYER 3", "#00115:01", "#00129:01", 5, 7),
        ];
        for (header, p1, p2, left, right) in cases {
            let text = format!("#BPM 120\n{header}\n#00111:01\n{p1}\n{p2}\n");
            std::fs::write(&path, text).unwrap();
            let chart = load(&path, 0.0).unwrap();
            assert_eq!(chart.lane_count, left + right);
            assert_eq!(read_meta(&path).unwrap().lane_count, chart.lane_count);
            assert_eq!(chart.keys.len(), chart.lane_count);
            assert!(chart.p2_notes.is_empty());
            assert!(chart.p2_mines.is_empty());
            if !p2.is_empty() {
                assert!(chart.notes.iter().any(|n| n.lane == left + right - 1));
            }
        }
    }

    #[test]
    fn double_play_preserves_bga_extensions() {
        let dir = tempdir();
        let path = dir.join("song.bms");
        std::fs::write(
            &path,
            "#BPM 120\n#PLAYER 3\n#POORBGA 1\n#ARGB01 255,20,40,60\n#00115:01\n#00125:01\n#0010A:01\n",
        )
        .unwrap();
        let chart = load(&path, 0.0).unwrap();
        assert_eq!(chart.lane_count, 10);
        assert_eq!(chart.bga_extensions.poor_mode, 1);
        assert_eq!(chart.bga_extensions.argb[&1], [255, 20, 40, 60]);
        assert_eq!(chart.bga[0].layer, BgaLayer::Overlay2);
    }

    #[test]
    fn ten_key_chart_makes_every_lane_playable() {
        let dir = tempdir();
        let path = dir.join("song.bms");
        let channels = [11, 12, 13, 14, 15, 21, 22, 23, 24, 25];
        let text = channels
            .iter()
            .map(|ch| format!("#001{ch}:01\n"))
            .collect::<String>();
        std::fs::write(&path, format!("#BPM 120\n{text}")).unwrap();
        let chart = load(&path, 0.0).unwrap();
        assert_eq!(chart.lane_count, 10);
        let mut lanes = chart.notes.iter().map(|n| n.lane).collect::<Vec<_>>();
        lanes.sort_unstable();
        assert_eq!(lanes, (0..10).collect::<Vec<_>>());
        let mut game = crate::game::Game::new(chart);
        for key in "sdfklzxcnm".chars() {
            let lane = crate::runtime::lane_for_key(key, &game.chart.keys).unwrap();
            game.hit(lane, 2000.0);
        }
        assert_eq!(game.combo, 10);
    }

    #[test]
    fn metadata_lane_count_matches_materialized_notes() {
        let dir = tempdir();
        let path = dir.join("song.bms");
        let cases = [
            ("long note", "#00111:01\n#00159:0101", 7),
            ("empty channel", "#00111:01\n#00119:00", 4),
            ("unpaired long note", "#00111:01\n#00159:01", 4),
            ("split long pair", "#00159:01\n#00259:01", 7),
            ("long note DP", "#00159:0101\n#00169:0101", 14),
            ("empty P2", "#00119:01\n#00129:00", 7),
            ("asymmetric DP", "#00111:01\n#00129:01", 11),
            ("mine", "#00111:01\n#001D9:01", 7),
            ("P2 mine only", "#00119:01\n#001E9:01", 14),
            ("invalid slot", "#00111:01\n#00119:!!", 4),
            ("invisible P2 channel", "#00111:01\n#00101:01\n#00149:01", 4),
            ("LNOBJ DP", "#LNOBJ ZZ\n#00119:01ZZ\n#00129:01ZZ", 14),
        ];
        for (name, channels, expected) in cases {
            std::fs::write(&path, format!("#BPM 120\n{channels}\n")).unwrap();
            let meta = read_meta(&path).unwrap();
            let chart = load(&path, 0.0).unwrap();
            assert_eq!(chart.lane_count, expected, "{name}");
            assert_eq!(meta.lane_count, chart.lane_count, "{name}");
        }
    }

    #[test]
    fn load_leaves_p2_pools_empty_for_single_play_charts() {
        let dir = tempdir();
        let path = dir.join("song.bms");
        std::fs::write(
            &path,
            "\
#TITLE SP
#BPM 60
#00111:0100
",
        )
        .unwrap();
        let chart = load(&path, 0.0).unwrap();
        assert!(chart.p2_notes.is_empty());
        assert!(chart.p2_mines.is_empty());
    }

    #[test]
    fn bga_channel_to_layer_covers_the_documented_channels() {
        assert_eq!(bga_channel_to_layer("04"), Some(BgaLayer::Base));
        assert_eq!(bga_channel_to_layer("06"), Some(BgaLayer::Poor));
        assert_eq!(bga_channel_to_layer("07"), Some(BgaLayer::Overlay));
        assert_eq!(bga_channel_to_layer("05"), None);
        assert_eq!(bga_channel_to_layer("11"), None);
    }

    #[test]
    fn header_pass_absorbs_bmp_definitions() {
        let mut p = HeaderPass::default();
        p.absorb("BMP01 cover.png").unwrap();
        p.absorb("BMPZZ  scene.mp4  ").unwrap();
        assert_eq!(p.bmp_defs.get(&1).unwrap(), "cover.png");
        assert_eq!(p.bmp_defs.get(&1295).unwrap(), "scene.mp4");
    }

    #[test]
    fn volume_events_use_hex_skip_rests_and_follow_the_timeline() {
        let dir = tempdir();
        let path = dir.join("volume.bms");
        std::fs::write(&path, "#BPM 120\n#STOP01 192\n#00109:01\n#00103:003C\n#00102:0.5\n#00298:ff\n#00197:004000GG\n#00198:0080\n").unwrap();
        let chart = load(&path, 100.0).unwrap();
        assert_eq!(chart.volume_events.len(), 3);
        let events = &chart.volume_events;
        assert_eq!(events[0].target, VolumeTarget::Bgm);
        assert_eq!(events[0].volume, 64);
        assert_eq!(events[0].time_ms, 4350.0);
        assert_eq!(events[1].target, VolumeTarget::Key);
        assert_eq!(events[1].volume, 128);
        assert_eq!(events[1].time_ms, 4600.0);
        assert_eq!(events[2].volume, 255);
        assert_eq!(events[2].time_ms, 5600.0);
    }

    #[test]
    fn load_records_bga_events_from_channels_04_06_07() {
        let dir = tempdir();
        let path = dir.join("song.bms");
        std::fs::write(
            &path,
            "\
#TITLE Bga
#BPM 60
#BMP01 cover.png
#BMP02 poor.png
#BMP03 layer.png
#00104:01
#00106:02
#00107:03
#00111:0100
",
        )
        .unwrap();
        let chart = load(&path, 0.0).unwrap();
        assert_eq!(chart.bga.len(), 3);
        let layers: Vec<_> = chart.bga.iter().map(|e| e.layer).collect();
        assert!(layers.contains(&BgaLayer::Base));
        assert!(layers.contains(&BgaLayer::Poor));
        assert!(layers.contains(&BgaLayer::Overlay));
    }

    #[test]
    fn resolve_bmp_paths_falls_back_to_alternate_extensions() {
        let dir = tempdir();
        std::fs::write(dir.join("cover.jpg"), b"").unwrap();
        let defs: HashMap<u32, String> = [(1u32, "cover.png".to_string())].into();
        let out = resolve_bmp_paths(&AssetIndex::new(&dir), &defs);
        assert_eq!(out.get(&1).unwrap(), &dir.join("cover.jpg"));
    }

    #[test]
    fn resolve_control_flow_passes_through_when_no_directives() {
        let text = "#TITLE Plain\n#BPM 130\n";
        assert_eq!(resolve_control_flow(text, 0), "#TITLE Plain\n#BPM 130\n");
    }

    #[test]
    fn resolve_control_flow_keeps_the_matching_setrandom_branch() {
        let text = "\
#SETRANDOM 2
#IF 1
#TITLE Never
#ENDIF
#IF 2
#TITLE Chosen
#ENDIF
";
        let out = resolve_control_flow(text, 0);
        assert!(out.contains("#TITLE Chosen"));
        assert!(!out.contains("#TITLE Never"));
    }

    #[test]
    fn resolve_control_flow_else_fires_when_no_prior_branch_matched() {
        let text = "\
#SETRANDOM 5
#IF 1
#TITLE A
#ELSEIF 2
#TITLE B
#ELSE
#TITLE Default
#ENDIF
";
        let out = resolve_control_flow(text, 0);
        assert!(out.contains("#TITLE Default"));
        assert!(!out.contains("#TITLE A"));
        assert!(!out.contains("#TITLE B"));
    }

    #[test]
    fn resolve_control_flow_drops_if_blocks_when_random_never_ran() {
        let text = "\
#IF 1
#TITLE Should not be included
#ENDIF
#TITLE Always
";
        let out = resolve_control_flow(text, 0);
        assert!(!out.contains("Should not be included"));
        assert!(out.contains("#TITLE Always"));
    }

    #[test]
    fn resolve_control_flow_supports_nested_ifs() {
        let text = "\
#SETRANDOM 1
#IF 1
#SETRANDOM 3
#IF 3
#TITLE Inner
#ENDIF
#ENDIF
";
        let out = resolve_control_flow(text, 0);
        assert!(out.contains("#TITLE Inner"));
    }

    #[test]
    fn switch_selects_cases_with_skip_and_fallthrough() {
        let branches = "\
#CASE 1
#00111:01
#SKIP
#CASE 2
#00112:02
#CASE 3
#00113:03
#SKIP
#DEF
#00114:04
#ENDSW
#TITLE After
";
        for (pick, expected) in [
            (1, "#00111:01\n"),
            (2, "#00112:02\n#00113:03\n"),
            (3, "#00113:03\n"),
            (4, "#00114:04\n"),
        ] {
            let text = format!("#SETSWITCH {pick}\n{branches}");
            assert_eq!(
                resolve_control_flow(&text, 0),
                format!("{expected}#TITLE After\n")
            );
        }
    }

    #[test]
    fn load_switch_keeps_only_selected_notes_and_metadata() {
        let dir = tempdir();
        let path = dir.join("switch.bms");
        std::fs::write(
            &path,
            "\
#BPM 120
#SETSWITCH 2
#CASE 1
#TITLE Hidden
#00111:01
#SKIP
#CASE 2
#TITLE Chosen
#00112:02
#SKIP
#DEF
#TITLE Default
#00113:03
#ENDSW
",
        )
        .unwrap();
        let chart = load(&path, 0.0).unwrap();
        assert_eq!(chart.notes.len(), 1);
        assert_eq!(chart.notes[0].keysound, Some(2));
        assert_eq!(read_meta(&path).unwrap().title, "Chosen");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn switch_falls_through_default_until_endsw() {
        let text = "\
#SETSWITCH 2
#CASE 2
#TITLE Chosen
#DEF
#ARTIST Default
#SKIP
#CASE 3
#BPM 150
#ENDSW
";
        assert_eq!(
            resolve_control_flow(text, 0),
            "#TITLE Chosen\n#ARTIST Default\n#BPM 150\n"
        );
    }

    #[test]
    fn switch_random_choice_is_deterministic_and_in_range() {
        let text = "\
#SWITCH 3
#CASE 1
#TITLE A
#SKIP
#CASE 2
#TITLE B
#SKIP
#CASE 3
#TITLE C
#SKIP
#DEF
#TITLE Invalid
#ENDSW
";
        for seed in 0..32 {
            let out = resolve_control_flow(text, seed);
            assert_eq!(out, resolve_control_flow(text, seed));
            assert!(["#TITLE A\n", "#TITLE B\n", "#TITLE C\n"].contains(&out.as_str()));
        }
    }

    #[test]
    fn switches_nest_with_random_and_if_in_both_directions() {
        let text = "\
#RANDOM 1
#IF 1
#SETSWITCH 2
#CASE 1
#SETSWITCH 1
#CASE 1
#TITLE Hidden switch
#ENDSW
#SETRANDOM 9
#CASE 2
#IF 9
#TITLE Hidden random
#ELSE
#TITLE Chosen
#ENDIF
#IF 9
#SKIP
#ENDIF
#SETSWITCH 1
#CASE 1
#ARTIST Nested
#SKIP
#DEF
#ARTIST Hidden default
#ENDSW
#BPM 150
#SKIP
#CASE 3
#TITLE Hidden case
#ENDSW
#ELSE
#SETSWITCH 1
#CASE 1
#TITLE Hidden if
#ENDSW
#ENDIF
#TITLE After
";
        assert_eq!(
            resolve_control_flow(text, 0),
            "#TITLE Chosen\n#ARTIST Nested\n#BPM 150\n#TITLE After\n"
        );
    }

    #[test]
    fn switch_directives_are_case_insensitive_and_ignore_invalid_ranges() {
        let text = "\
#switch 0
#case 1
#TITLE Invalid
#def
#TITLE Invalid default
#endsw
#setswitch 1
#case 1
#TITLE Chosen
#skip
#def
#TITLE Hidden
#endsw
";
        assert_eq!(resolve_control_flow(text, 0), "#TITLE Chosen\n");
    }

    #[test]
    fn switch_branches_preserve_and_restore_random_scopes() {
        let text = "\
#SETRANDOM 2
#SETSWITCH 1
#CASE 2
#RANDOM 3
#ENDRANDOM
#CASE 1
#RANDOM 1
#IF 1
Inner
#ENDIF
#ENDRANDOM
#IF 2
Outer
#ENDIF
#SKIP
#CASE 3
#ENDRANDOM
#ENDSW
#IF 2
After
#ENDIF
#ENDRANDOM
";
        assert_eq!(resolve_control_flow(text, 0), "Inner\nOuter\nAfter\n");
    }

    #[test]
    fn resolve_control_flow_restores_outer_random_after_nested_scope() {
        let text = "\
#RANDOM 2
#IF 1
#RANDOM 3
#IF 1
Inner
#ELSEIF 2
Inner
#ELSE
Inner
#ENDIF
#ENDRANDOM
#ENDIF
#IF 2
Outer two
#ELSEIF 1
Outer one
#ENDIF
";
        (0u64..64).for_each(|seed| {
            let mut rng = seed.wrapping_add(0x9E3779B97F4A7C15);
            let expected = match xorshift_pick(&mut rng, 2) {
                1 => "Inner\nOuter one\n",
                _ => "Outer two\n",
            };
            assert_eq!(resolve_control_flow(text, seed), expected, "seed {seed}");
        });
    }

    #[test]
    fn resolve_control_flow_ignores_random_scopes_in_skipped_branches() {
        let text = "\
#RANDOM 1
#IF 2
#RANDOM 3
#SETRANDOM 2
#ENDRANDOM
#ENDRANDOM
#ENDIF
#IF 1
Outer
#ENDIF
#ENDRANDOM
";
        assert_eq!(resolve_control_flow(text, 0), "Outer\n");
    }

    #[test]
    fn resolve_control_flow_keeps_latest_pick_without_endrandom() {
        let text = "\
#RANDOM 1
#IF 1
#SETRANDOM 2
#ENDIF
#IF 2
Latest
#ENDIF
#RANDOM 1
#IF 1
Next
#ENDIF
";
        assert_eq!(resolve_control_flow(text, 0), "Latest\nNext\n");
    }

    #[test]
    fn resolve_control_flow_endrandom_restores_setrandom_and_clears_last_pick() {
        let text = "\
#SETRANDOM 2
#RANDOM 1
#IF 1
Inner
#ENDIF
#ENDRANDOM
#IF 2
Outer
#ENDIF
#ENDRANDOM
#ENDRANDOM
#IF 2
Unexpected
#ENDIF
Always
";
        assert_eq!(resolve_control_flow(text, 0), "Inner\nOuter\nAlways\n");
    }

    #[test]
    fn xorshift_pick_stays_in_range_and_is_deterministic_for_a_seed() {
        let mut a = 12345u64;
        let mut b = 12345u64;
        for _ in 0..50 {
            let x = xorshift_pick(&mut a, 10);
            let y = xorshift_pick(&mut b, 10);
            assert_eq!(x, y);
            assert!((1..=10).contains(&x));
        }
    }

    #[test]
    fn mine_channel_to_lane_mirrors_the_regular_visible_channel_map() {
        assert_eq!(mine_channel_to_lane("D1"), Some(0));
        assert_eq!(mine_channel_to_lane("D5"), Some(4));
        assert_eq!(mine_channel_to_lane("D8"), Some(5));
        assert_eq!(mine_channel_to_lane("D9"), Some(6));
        assert_eq!(mine_channel_to_lane("D6"), Some(7));
        assert_eq!(mine_channel_to_lane("11"), None);
    }

    #[test]
    fn load_extracts_mines_from_channel_d1() {
        let dir = tempdir();
        let path = dir.join("song.bms");
        // BPM 60 → 4000ms/measure. Slot 0 (00 = no mine), slot 1 (base-36 64 = 220
        // damage). Also a regular note in channel 11 so lane 0 counts.
        std::fs::write(
            &path,
            "\
#TITLE Miner
#BPM 60
#001D1:0064
#00111:0100
",
        )
        .unwrap();
        let chart = load(&path, 0.0).unwrap();
        assert_eq!(chart.mines.len(), 1);
        assert_eq!(chart.mines[0].lane, 0);
        assert_eq!(chart.mines[0].damage, 220);
        assert!((chart.mines[0].time_ms - 6000.0).abs() < 1e-6);
    }

    #[test]
    fn load_drops_mines_whose_lane_is_above_the_detected_lane_count() {
        // Only lane 0 is used by real notes, so lane_count clamps to 4.
        // A mine on channel D9 (lane 6) should be dropped.
        let dir = tempdir();
        let path = dir.join("song.bms");
        std::fs::write(
            &path,
            "\
#TITLE OobMine
#BPM 60
#00111:0100
#001D9:0064
",
        )
        .unwrap();
        let chart = load(&path, 0.0).unwrap();
        // lane_count is 4 → the mine on lane 6 is filtered.
        assert!(chart.mines.iter().all(|m| m.lane < chart.lane_count));
    }

    #[test]
    fn long_channel_to_lane_mirrors_the_regular_visible_channel_map() {
        assert_eq!(long_channel_to_lane("51"), Some(0));
        assert_eq!(long_channel_to_lane("55"), Some(4));
        assert_eq!(long_channel_to_lane("58"), Some(5));
        assert_eq!(long_channel_to_lane("59"), Some(6));
        assert_eq!(long_channel_to_lane("56"), Some(7));
        assert_eq!(long_channel_to_lane("11"), None);
    }

    #[test]
    fn header_pass_absorbs_lnobj_and_lntype() {
        let mut p = HeaderPass::default();
        p.absorb("LNOBJ ZZ").unwrap();
        p.absorb("LNTYPE 2").unwrap();
        assert_eq!(p.lnobj, Some(1295));
        assert_eq!(p.lntype, 2);
    }

    #[test]
    fn header_pass_clamps_lntype_to_supported_variants() {
        let mut p = HeaderPass::default();
        p.absorb("LNTYPE 9").unwrap();
        assert_eq!(p.lntype, 1, "unsupported LN types fall back to the default");
    }

    #[test]
    fn load_pairs_channel_51_slots_into_long_notes() {
        let dir = tempdir();
        let path = dir.join("song.bms");
        // BPM 60 → 4000ms/measure. On channel 51 (LN lane 0), slot 0 (start)
        // and slot 2 (end) of measure 1: LN from 4000ms to 6000ms.
        std::fs::write(
            &path,
            "\
#TITLE LNPair
#BPM 60
#00151:01000100
",
        )
        .unwrap();
        let chart = load(&path, 0.0).unwrap();
        assert_eq!(chart.notes.len(), 1);
        assert_eq!(chart.notes[0].lane, 0);
        assert!((chart.notes[0].time_ms - 4000.0).abs() < 1e-6);
        assert_eq!(chart.notes[0].end_ms, Some(6000.0));
    }

    #[test]
    fn load_promotes_lnobj_terminator_into_an_ln_end() {
        let dir = tempdir();
        let path = dir.join("song.bms");
        // BPM 60 → 4000ms/measure. Slot 0 uses WAV01 (LN start), slot 2 uses
        // WAV ZZ which matches #LNOBJ → LN terminator. Result: one LN.
        std::fs::write(
            &path,
            "\
#TITLE LNObj
#BPM 60
#LNOBJ ZZ
#00111:010000ZZ
",
        )
        .unwrap();
        let chart = load(&path, 0.0).unwrap();
        assert_eq!(chart.notes.len(), 1);
        assert!((chart.notes[0].time_ms - 4000.0).abs() < 1e-6);
        assert_eq!(chart.notes[0].end_ms, Some(7000.0));
    }

    #[test]
    fn load_applies_channel_09_stop_to_note_timing() {
        let dir = tempdir();
        let path = dir.join("song.bms");
        // BPM 120 → 2000ms/measure. STOP 96 = 2 beats = 1000ms at 120 BPM.
        // Note in measure 2 should be shifted by 1000ms compared to no-stop.
        std::fs::write(
            &path,
            "\
#TITLE Stopper
#BPM 120
#STOP01 96
#00109:01
#00211:0100
",
        )
        .unwrap();
        let chart = load(&path, 0.0).unwrap();
        assert_eq!(chart.notes.len(), 1);
        // No stop would put the note at 4000ms; with a 1000ms stop it lands at 5000ms.
        assert!((chart.notes[0].time_ms - 5000.0).abs() < 1e-6);
    }

    #[test]
    fn load_applies_stp_at_mid_measure() {
        let dir = tempdir();
        let path = dir.join("song.bms");
        std::fs::write(
            &path,
            "#BPM 120\n#STP 002.500 1000\n#00211:01010101\n#00311:01\n",
        )
        .unwrap();
        let chart = load(&path, 250.0).unwrap();
        let times: Vec<_> = chart.notes.iter().map(|note| note.time_ms).collect();
        assert_eq!(times, vec![4250.0, 4750.0, 6250.0, 6750.0, 7250.0]);
    }

    #[test]
    fn load_pms_combines_stp_with_extended_bpm() {
        let dir = tempdir();
        let path = dir.join("song.pms");
        std::fs::write(
            &path,
            "#BPM 120\n#EXBPM01 240\n#STP 002.500 1000\n#00208:0001\n#00222:0101\n#00325:01\n",
        )
        .unwrap();
        let chart = load(&path, 250.0).unwrap();
        assert_eq!(chart.lane_count, 9);
        assert!(chart.p2_notes.is_empty());
        let notes: Vec<_> = chart
            .notes
            .iter()
            .map(|note| (note.lane, note.time_ms))
            .collect();
        assert_eq!(notes, vec![(5, 4250.0), (5, 6250.0), (8, 6750.0)]);
        assert_eq!(chart.duration_ms, 10250.0);
    }

    #[test]
    fn stp_combines_with_bpm_changes_and_tick_stops() {
        let mut pass = HeaderPass::default();
        pass.absorb("BPM 120").unwrap();
        pass.absorb("STOP01 48").unwrap();
        pass.absorb("stp 002.500 1000").unwrap();
        pass.absorb("STP 002.500 250").unwrap();
        let raw = vec![
            (2, "03".into(), "00F0".into()),
            (2, "09".into(), "0001".into()),
        ];
        let (timeline, _, _) = collect_chart_sides(&raw, &pass, 0.0, false, false).unwrap();
        assert_eq!(timeline.time_at(2, 0.25), 4500.0);
        assert_eq!(timeline.time_at(2, 0.5), 6500.0);
        assert_eq!(timeline.time_at(3, 0.0), 7000.0);
    }

    #[test]
    fn stp_combines_with_case_sensitive_base62_timing_references() {
        let text =
            "#BASE 62\n#BPM 120\n#BPM0A 60\n#BPM0a 240\n#STOP0A 192\n#STOP0a 48\n#STP 002.500 1000";
        let mut pass = HeaderPass::for_text(text);
        header_lines(text).for_each(|line| pass.absorb(line).unwrap());
        let raw = vec![
            (2, "08".into(), "000a".into()),
            (2, "09".into(), "000a".into()),
        ];
        let (timeline, _, _) = collect_chart_sides(&raw, &pass, 0.0, false, false).unwrap();
        assert_eq!(timeline.time_at(2, 0.25), 4500.0);
        assert_eq!(timeline.time_at(2, 0.5), 6250.0);
        assert_eq!(timeline.time_at(3, 0.0), 6750.0);
    }

    #[test]
    fn load_includes_stp_after_last_note_in_duration() {
        let dir = tempdir();
        let path = dir.join("song.bms");
        std::fs::write(&path, "#BPM 120\n#00011:01\n#STP 002.500 1000\n").unwrap();
        let chart = load(&path, 0.0).unwrap();
        assert_eq!(chart.duration_ms, 9500.0);
    }

    #[test]
    fn stp_ignores_invalid_positions_and_durations() {
        [
            "",
            "002.500",
            "002.500 0",
            "002.500 -1000",
            "002.500 NaN",
            "002.500 inf",
            "002.500 1000 extra",
            "002.1000 1000",
            "002.abc 1000",
            "-02.500 1000",
            "002500 1000",
        ]
        .iter()
        .for_each(|value| assert!(parse_stp(value).is_none(), "{value}"));
        let event = parse_stp("002.000 12.5").unwrap();
        assert_eq!(event.measure, 2);
        assert_eq!(event.frac, 0.0);
        assert!(matches!(event.kind, TimingEventKind::StopMs(12.5)));
    }

    #[test]
    fn header_pass_absorbs_bpm_definitions() {
        let mut p = HeaderPass::default();
        p.absorb("BPM01 175.5").unwrap();
        p.absorb("BPMZZ 60").unwrap();
        assert!(p.absorb("BPM02 -20").is_err());
        assert_eq!(p.bpm_defs.get(&1), Some(&175.5));
        assert_eq!(p.bpm_defs.get(&1295), Some(&60.0));
        assert!(!p.bpm_defs.contains_key(&2), "non-positive BPM is rejected");
    }

    #[test]
    fn header_pass_absorbs_exbpm_definitions_as_bpm_aliases() {
        let mut p = HeaderPass::default();
        p.absorb("EXBPM01 180").unwrap();
        p.absorb("exbpmZZ 60").unwrap();
        assert!(p.absorb("EXBPM02 -20").is_err());
        assert!(p.absorb("EXBPM03 invalid").is_err());
        p.absorb("EXBPM 240").unwrap();
        assert_eq!(p.bpm_defs.get(&1), Some(&180.0));
        assert_eq!(p.bpm_defs.get(&1295), Some(&60.0));
        assert_eq!(p.bpm_defs.len(), 2);
        assert_eq!(p.bpm, 130.0);
        p.absorb("BPM01 150").unwrap();
        assert_eq!(p.bpm_defs.get(&1), Some(&150.0));
        p.absorb("EXBPM01 180").unwrap();
        assert_eq!(p.bpm_defs.get(&1), Some(&180.0));
    }

    #[test]
    fn load_honors_a_channel_08_exbpm_change_mid_song() {
        let dir = tempdir();
        let path = dir.join("song.bms");
        std::fs::write(
            &path,
            "#BPM 120\n#EXBPM01 180\n#00108:01\n#00111:01\n#00211:01\n",
        )
        .unwrap();
        let chart = load(&path, 0.0).unwrap();
        assert_eq!(chart.notes.len(), 2);
        assert!((chart.notes[0].time_ms - 2000.0).abs() < 1e-6);
        assert!((chart.notes[1].time_ms - (2000.0 + 4.0 * 60_000.0 / 180.0)).abs() < 1e-6);
    }

    #[test]
    fn load_honors_a_channel_08_bpm_change_mid_song() {
        let dir = tempdir();
        let path = dir.join("song.bms");
        // BPM starts at 120 (2000ms/measure). At the top of measure 1 we
        // switch to 240 via #BPM01. Note at measure 2 should therefore be at
        // 2000 (measure 0) + 1000 (measure 1 at 240) + 0 = 3000ms.
        std::fs::write(
            &path,
            "\
#TITLE BpmSwap
#BPM 120
#BPM01 240
#00108:01
#00211:0100
",
        )
        .unwrap();
        let chart = load(&path, 0.0).unwrap();
        assert_eq!(chart.notes.len(), 1);
        assert!((chart.notes[0].time_ms - 3000.0).abs() < 1e-6);
    }

    #[test]
    fn collect_measure_scales_reads_channel_02_floats_and_drops_others() {
        let raw = vec![
            (0u32, "02".to_string(), "0.5".to_string()),
            (1u32, "11".to_string(), "0100".to_string()),
            (2u32, "02".to_string(), "1.25".to_string()),
            (3u32, "02".to_string(), "not a number".to_string()),
            (4u32, "02".to_string(), "-1".to_string()),
            (5u32, "02".to_string(), "150000".to_string()),
        ];
        let out = collect_measure_scales(&raw);
        assert_eq!(out.get(&0), Some(&0.5));
        assert_eq!(out.get(&2), Some(&1.25));
        assert!(!out.contains_key(&1));
        assert!(!out.contains_key(&3), "unparseable data is skipped");
        assert!(!out.contains_key(&4), "non-positive scale is skipped");
        assert!(
            !out.contains_key(&5),
            "wildly out-of-range values (non-standard extensions) are dropped"
        );
    }

    #[test]
    fn load_applies_measure_length_change_to_note_timing() {
        let dir = tempdir();
        let path = dir.join("song.bms");
        // BPM 60 → base measure = 4000ms. Measure 1 is halved to 2000ms,
        // so the measure-2 note should land 2000ms after the measure-1 note
        // instead of the usual 4000ms.
        std::fs::write(
            &path,
            "\
#TITLE MeasureLen
#BPM 60
#00102:0.5
#00111:0100
#00211:0100
",
        )
        .unwrap();
        let chart = load(&path, 0.0).unwrap();
        assert_eq!(chart.notes.len(), 2);
        assert!((chart.notes[0].time_ms - 4000.0).abs() < 1e-6);
        assert!((chart.notes[1].time_ms - 6000.0).abs() < 1e-6);
    }

    #[test]
    fn load_populates_rank_total_and_volwav() {
        let dir = tempdir();
        let path = dir.join("song.bms");
        std::fs::write(
            &path,
            "\
#TITLE Testing
#BPM 130
#RANK 2
#TOTAL 260.5
#VOLWAV 75
#00111:0100
",
        )
        .unwrap();
        let chart = load(&path, 0.0).unwrap();
        assert_eq!(chart.rank, Some(2));
        assert_eq!(chart.total, Some(260.5));
        assert_eq!(chart.vol_wav, Some(75));
    }

    #[test]
    fn load_populates_optional_metadata_on_chart() {
        let dir = tempdir();
        let path = dir.join("song.bms");
        std::fs::write(
            &path,
            "\
#TITLE Testing
#SUBTITLE ~beta~
#ARTIST alice
#SUBARTIST bob
#GENRE electronic
#STAGEFILE cover.png
#BANNER banner.png
#BACKBMP background.bmp
#COMMENT Nice song!\x1b[2J\u{9b}\x07
#MAKER unhappychoice
#BPM 130
#00111:0100
",
        )
        .unwrap();
        let chart = load(&path, 0.0).unwrap();
        assert_eq!(chart.subtitle, "~beta~");
        assert_eq!(chart.subartist, "bob");
        assert_eq!(chart.genre, "electronic");
        assert_eq!(chart.stagefile, "cover.png");
        assert_eq!(chart.banner, "banner.png");
        assert_eq!(chart.backbmp, "background.bmp");
        assert_eq!(chart.comment, "Nice song![2J");
        assert_eq!(chart.asset_dir, dir);
        let meta = read_meta(&path).unwrap();
        assert_eq!(meta.banner, chart.banner);
        assert_eq!(meta.comment, chart.comment);
        assert_eq!(chart.maker, "unhappychoice");
    }

    #[test]
    fn header_pass_defaults_bpm_to_130() {
        let p = HeaderPass::default();
        assert_eq!(p.bpm, 130.0);
    }

    #[test]
    fn header_pass_rejects_malformed_bpm_and_ignores_other_malformed_numbers() {
        let mut p = HeaderPass::default();
        assert!(p.absorb("BPM  not-a-number").is_err());
        p.absorb("PLAYLEVEL huh").unwrap();
        assert_eq!(p.bpm, 130.0);
        assert_eq!(p.playlevel, None);
    }

    #[test]
    fn mode_detection_combines_extension_player_and_channels() {
        let cases = [
            ("song.bms", None, "15", 5, false, false),
            ("song.bme", Some(1), "11", 7, false, false),
            ("song.BME", Some(3), "11", 7, true, false),
            ("song.bml", Some(3), "19", 7, true, false),
            ("song.bms", Some(3), "11", 5, true, false),
            ("song.bme", Some(2), "26", 7, true, true),
            ("song.bml", None, "56", 5, false, true),
            ("song.pms", Some(3), "16", 9, false, false),
        ];
        for (name, player, channel, keys_per_side, double_play, scratch) in cases {
            let raw = vec![(1, channel.to_string(), "0101".to_string())];
            let (_, acc, p2_acc) = collect_chart_sides(
                &raw,
                &HeaderPass::default(),
                0.0,
                false,
                is_pms(Path::new(name)),
            )
            .unwrap();
            assert_eq!(
                detect_mode(Path::new(name), player, &raw, &acc, &p2_acc),
                BmsMode {
                    keys_per_side,
                    double_play,
                    scratch,
                    reserve_seven: name.to_ascii_lowercase().ends_with(".bme"),
                    declared_sides: !name.ends_with("pms") && matches!(player, Some(2 | 3)),
                },
                "{name}"
            );
        }
    }

    #[test]
    fn sparse_bme_uses_declared_sides_in_metadata_and_playback() {
        let dir = tempdir();
        let path = dir.join("sparse.bme");
        for (player, channels, expected) in [
            (1, "#00111:01", 7),
            (3, "#00111:01", 14),
            (3, "#00111:01\n#00121:01", 14),
            (2, "#00121:01", 14),
        ] {
            std::fs::write(&path, format!("#PLAYER {player}\n{channels}\n")).unwrap();
            let chart = load(&path, 0.0).unwrap();
            assert_eq!(chart.lane_count, expected);
            assert_eq!(read_meta(&path).unwrap().lane_count, expected);
            if channels.contains("21") {
                assert!(chart.notes.iter().any(|note| note.lane == 7));
            }
        }
    }

    #[test]
    fn declared_modes_preserve_scratch_layouts_and_millisecond_stops() {
        let dir = tempdir();
        for (name, player, channels, count, scratch_lanes) in [
            ("single.bme", 1, "#00116:01", 8, vec![7]),
            ("double.bme", 3, "#00116:01\n#00126:02", 16, vec![7, 15]),
            ("double.bms", 2, "#00116:01\n#00126:02", 12, vec![5, 11]),
            (
                "extended.bms",
                3,
                "#00116:01\n#00129:02\n#00126:03",
                14,
                vec![5, 13],
            ),
        ] {
            let path = dir.join(name);
            std::fs::write(
                &path,
                format!("#BPM 60\n#PLAYER {player}\n#STP 001.000 500\n{channels}\n"),
            )
            .unwrap();
            let chart = load(&path, 0.0).unwrap();
            assert_eq!(chart.lane_count, count, "{name}");
            assert_eq!(read_meta(&path).unwrap().lane_count, count, "{name}");
            assert!(chart.p2_notes.is_empty(), "{name}");
            scratch_lanes.iter().for_each(|lane| {
                assert!(
                    chart
                        .notes
                        .iter()
                        .any(|note| note.lane == *lane && (note.time_ms - 4500.0).abs() < 0.01),
                    "{name}: {lane}"
                );
            });
        }
    }

    #[test]
    fn sparse_dp_preserves_base62_invisible_notes_on_both_sides() {
        let dir = tempdir();
        let path = dir.join("sparse.bme");
        std::fs::write(
            &path,
            "#BASE 62\n#PLAYER 3\n#00111:01\n#00139:0a\n#00149:0A\n",
        )
        .unwrap();
        let chart = load_with_seed(&path, 0.0, 42).unwrap();
        assert_eq!(chart.lane_count, 14);
        assert_eq!(read_meta_with_seed(&path, 42).unwrap().lane_count, 14);
        assert_eq!(chart.invisible_notes.len(), 2);
        assert_eq!(chart.invisible_notes[0].lane, 6);
        assert_eq!(chart.invisible_notes[0].keysound, 36);
        assert_eq!(chart.invisible_notes[1].lane, 13);
        assert_eq!(chart.invisible_notes[1].keysound, 10);
        assert!(chart.p2_invisible_notes.is_empty());
    }

    #[test]
    fn header_pass_accepts_only_supported_player_values() {
        let mut pass = HeaderPass::default();
        assert_eq!(pass.player, None);
        for value in 1..=3 {
            pass.absorb(&format!("player {value}")).unwrap();
            assert_eq!(pass.player, Some(value));
        }
        for value in ["0", "4", "invalid"] {
            pass.absorb(&format!("PLAYER {value}")).unwrap();
            assert_eq!(pass.player, Some(3));
        }
    }

    #[test]
    fn lane_set_count_climbs_with_channel_max() {
        let mut s = LaneSet::default();
        s.observe_lane(0);
        s.observe_lane(1);
        // Only lanes 0..=1 seen so far → clamp down to the 4K keymap.
        assert_eq!(s.count(), 4);
        s.observe_lane(3);
        // Lane 3 pushes us out of pure 4K into the 5K keymap.
        assert_eq!(s.count(), 5);
        s.observe_lane(6);
        // Lane 6 is the seventh key column.
        assert_eq!(s.count(), 7);
    }

    #[test]
    fn load_resolves_windows_assets_and_preserves_exrank_events() {
        let dir = tempdir();
        std::fs::create_dir_all(dir.join("sub/nested")).unwrap();
        [
            "sub/kick.wav",
            "sub/nested/Snare.OGG",
            "Cover.PNG",
            "sub/Background.JPG",
        ]
        .iter()
        .for_each(|name| std::fs::write(dir.join(name), b"").unwrap());
        let chart_path = dir.join("chart.bms");
        std::fs::write(
            &chart_path,
            "#BPM 120\n#WAV01 SUB\\KICK.WAV\n#WAV02 SUB\\NESTED\\SNARE.WAV\n\
             #BMP01 cover.png\n#BMP02 SUB\\BACKGROUND.BMP\n\
             #DEFEXRANK 75\n#EXRANK01 50\n#001A0:01\n#00111:0102\n",
        )
        .unwrap();
        let chart = load(&chart_path, 0.0).unwrap();
        assert_eq!(chart.def_exrank, Some(75.0));
        assert_eq!(chart.exrank_defs.get(&1), Some(&50.0));
        assert_eq!(chart.exrank_events.len(), 1);
        assert_eq!(chart.exrank_events[0].time_ms, 2000.0);
        assert_eq!(chart.judgment_scale(1999.0), 0.75);
        assert_eq!(chart.judgment_scale(2000.0), 0.5);
        assert_eq!(chart.wav_paths.get(&1), Some(&dir.join("sub/kick.wav")));
        assert_eq!(
            chart.wav_paths.get(&2),
            Some(&dir.join("sub/nested/Snare.OGG"))
        );
        assert_eq!(chart.bmp_paths.get(&1), Some(&dir.join("Cover.PNG")));
        assert_eq!(
            chart.bmp_paths.get(&2),
            Some(&dir.join("sub/Background.JPG"))
        );
    }

    #[test]
    fn load_pms_resolves_windows_assets_and_preserves_ninth_lane() {
        let dir = tempdir();
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        std::fs::write(dir.join("assets/Kick.OGG"), b"").unwrap();
        std::fs::write(dir.join("assets/Cover.PNG"), b"").unwrap();
        let path = dir.join("chart.pms");
        std::fs::write(
            &path,
            "#BPM 120\n#WAV01 ASSETS\\KICK.WAV\n#BMP01 ASSETS\\COVER.BMP\n#00125:01\n",
        )
        .unwrap();
        let chart = load(&path, 0.0).unwrap();
        assert_eq!(chart.lane_count, 9);
        assert_eq!(chart.notes.len(), 1);
        assert_eq!(chart.notes[0].lane, 8);
        assert_eq!(chart.notes[0].keysound, Some(1));
        assert_eq!(chart.wav_paths.get(&1), Some(&dir.join("assets/Kick.OGG")));
        assert_eq!(chart.bmp_paths.get(&1), Some(&dir.join("assets/Cover.PNG")));
    }

    #[test]
    fn resolve_assets_normalizes_leading_current_directory_components() {
        let dir = tempdir();
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        [
            "kick.wav",
            "Cover.PNG",
            "sub/Snare.OGG",
            "sub/Background.JPG",
        ]
        .iter()
        .for_each(|name| std::fs::write(dir.join(name), b"").unwrap());
        let assets = AssetIndex::new(&dir);
        ["./", ".\\", "././", ".\\.\\"].iter().for_each(|prefix| {
            let wavs = [
                (1, format!("{prefix}kick.wav")),
                (2, format!("{prefix}SUB\\SNARE.WAV")),
            ]
            .into();
            let bmps = [
                (1, format!("{prefix}cover.png")),
                (2, format!("{prefix}SUB\\BACKGROUND.BMP")),
            ]
            .into();
            let wav_paths = resolve_wavs(&assets, &wavs);
            let bmp_paths = resolve_bmp_paths(&assets, &bmps);
            assert_eq!(wav_paths.get(&1), Some(&dir.join("kick.wav")));
            assert_eq!(wav_paths.get(&2), Some(&dir.join("sub/Snare.OGG")));
            assert_eq!(bmp_paths.get(&1), Some(&dir.join("Cover.PNG")));
            assert_eq!(bmp_paths.get(&2), Some(&dir.join("sub/Background.JPG")));
        });
    }

    #[test]
    fn resolve_assets_prefers_requested_extension_over_fallback() {
        let dir = tempdir();
        ["Kick.WAV", "kick.ogg", "Cover.BMP", "cover.png"]
            .iter()
            .for_each(|name| std::fs::write(dir.join(name), b"").unwrap());
        let assets = AssetIndex::new(&dir);
        let wavs = [(1, "kick.wav".to_string())].into();
        let bmps = [(1, "cover.bmp".to_string())].into();
        assert_eq!(
            resolve_wavs(&assets, &wavs).get(&1),
            Some(&dir.join("Kick.WAV"))
        );
        assert_eq!(
            resolve_bmp_paths(&assets, &bmps).get(&1),
            Some(&dir.join("Cover.BMP"))
        );
    }

    #[test]
    fn resolve_assets_prefers_exact_case_when_names_collide() {
        let dir = tempdir();
        std::fs::write(dir.join("Kick.wav"), b"").unwrap();
        if dir.join("kick.wav").exists() {
            return;
        }
        std::fs::write(dir.join("kick.wav"), b"").unwrap();
        let defs = [(1, "kick.wav".to_string()), (2, "KICK.WAV".to_string())].into();
        let out = resolve_wavs(&AssetIndex::new(&dir), &defs);
        assert_eq!(out.get(&1), Some(&dir.join("kick.wav")));
        assert_eq!(out.get(&2), Some(&dir.join("Kick.wav")));
    }

    #[test]
    fn resolve_bmp_paths_does_not_fall_back_to_chart_directory() {
        let dir = tempdir();
        std::fs::write(dir.join("cover.jpg"), b"").unwrap();
        let defs = [(1, "sub/cover.bmp".to_string())].into();
        assert!(resolve_bmp_paths(&AssetIndex::new(&dir), &defs).is_empty());
    }

    #[test]
    fn resolve_wavs_returns_direct_match_when_file_exists() {
        let dir = tempdir();
        std::fs::write(dir.join("kick.wav"), b"").unwrap();
        let defs: HashMap<u32, String> = [(1u32, "kick.wav".to_string())].into();
        let out = resolve_wavs(&AssetIndex::new(&dir), &defs);
        assert_eq!(out.get(&1).unwrap(), &dir.join("kick.wav"));
    }

    #[test]
    fn resolve_wavs_tries_alternate_extensions_when_direct_missing() {
        let dir = tempdir();
        std::fs::write(dir.join("kick.ogg"), b"").unwrap();
        let defs: HashMap<u32, String> = [(1u32, "kick.wav".to_string())].into();
        let out = resolve_wavs(&AssetIndex::new(&dir), &defs);
        assert_eq!(out.get(&1).unwrap(), &dir.join("kick.ogg"));
    }

    #[test]
    fn resolve_wavs_drops_entries_with_no_matching_file() {
        let dir = tempdir();
        let defs: HashMap<u32, String> = [(1u32, "missing.wav".to_string())].into();
        let out = resolve_wavs(&AssetIndex::new(&dir), &defs);
        assert!(out.is_empty());
    }

    #[test]
    fn resolve_wavs_preserves_subdirectories_when_trying_alternate_extensions() {
        let dir = tempdir();
        std::fs::create_dir_all(dir.join("audio")).unwrap();
        std::fs::write(dir.join("audio/kick.ogg"), b"").unwrap();
        std::fs::write(dir.join("kick.ogg"), b"").unwrap();
        let defs: HashMap<u32, String> = [(1u32, "audio/kick.wav".to_string())].into();
        let out = resolve_wavs(&AssetIndex::new(&dir), &defs);
        assert_eq!(out.get(&1).unwrap(), &dir.join("audio/kick.ogg"));
    }

    #[test]
    fn resolve_wavs_does_not_fall_back_to_chart_directory() {
        let dir = tempdir();
        std::fs::write(dir.join("kick.ogg"), b"").unwrap();
        let defs: HashMap<u32, String> = [(1u32, "audio/kick.wav".to_string())].into();
        let out = resolve_wavs(&AssetIndex::new(&dir), &defs);
        assert!(out.is_empty());
    }

    #[test]
    fn read_meta_extracts_headers_and_infers_lane_count() {
        let dir = tempdir();
        let path = dir.join("song.bms");
        std::fs::write(
            &path,
            "\
#TITLE Testing
#ARTIST unhappychoice
#BPM 148
#PLAYLEVEL 7
#DIFFICULTY 3
#WAV01 kick.wav
#00111:0100
#00119:0001
",
        )
        .unwrap();

        let m = read_meta(&path).unwrap();
        assert_eq!(m.title, "Testing");
        assert_eq!(m.artist, "unhappychoice");
        assert_eq!(m.bpm, 148.0);
        assert_eq!(m.playlevel, Some(7));
        assert_eq!(m.difficulty, Some(3));
        assert_eq!(m.lane_count, 7);
    }

    #[test]
    fn load_materialises_notes_bgm_and_sorts_by_time() {
        let dir = tempdir();
        let path = dir.join("song.bms");
        // BPM 60 → measure = 4000ms. Slot step for 4-slot data = 1000ms.
        std::fs::write(
            &path,
            "\
#TITLE Loader Test
#BPM 60
#WAV01 kick.wav
#00111:01000100
#00201:01000000
",
        )
        .unwrap();

        let chart = load(&path, 500.0).unwrap();
        assert_eq!(chart.title, "Loader Test");
        assert_eq!(chart.bpm, 60.0);
        assert_eq!(chart.lane_count, 4);
        assert_eq!(chart.notes.len(), 2);
        assert_eq!(chart.bgm.len(), 1);
        // Sorted ascending by time
        for pair in chart.notes.windows(2) {
            assert!(pair[0].time_ms <= pair[1].time_ms);
        }
        // First note is at measure 1, slot 0 → t = 4000 + 500 = 4500 ms.
        assert!((chart.notes[0].time_ms - 4500.0).abs() < 1e-6);
        assert!(chart.wav_paths.is_empty(), "no wav file bundled");
    }

    #[test]
    fn load_drops_notes_that_exceed_the_detected_lane_count() {
        let dir = tempdir();
        let path = dir.join("song.bms");
        // Only channel 11 is used, so lane_count should be 4.
        // Channel 15 references lane 4, which sits beyond the 4-lane keymap.
        std::fs::write(
            &path,
            "\
#TITLE Filter Test
#BPM 120
#00111:0100
#00115:0100
",
        )
        .unwrap();
        let chart = load(&path, 0.0).unwrap();
        assert_eq!(chart.lane_count, 5);
        assert!(chart.notes.iter().all(|n| n.lane < 5));
    }

    #[test]
    fn load_reports_context_for_missing_file() {
        let dir = tempdir();
        let missing = dir.join("nope.bms");
        let err = match load(&missing, 0.0) {
            Ok(_) => panic!("expected load to fail for a missing file"),
            Err(e) => e,
        };
        let msg = format!("{}", err);
        assert!(
            msg.contains("cannot read") && msg.contains("nope.bms"),
            "expected path context in error message, got: {}",
            msg
        );
    }
}
