use crate::chart::{keys_for, BgmEvent, Chart, Note};
use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct ChartMeta {
    pub path: PathBuf,
    pub title: String,
    pub subtitle: String,
    pub artist: String,
    pub subartist: String,
    pub genre: String,
    pub bpm: f64,
    pub playlevel: Option<u8>,
    pub difficulty: Option<u8>,
    pub lane_count: usize,
}

pub fn read_meta(path: &Path) -> Result<ChartMeta> {
    let text = read_text(path)?;
    let mut pass = HeaderPass::default();
    let mut lanes = LaneSet::default();
    for line in header_lines(&text) {
        if let Some((_, ch, _)) = parse_channel_line(line) {
            lanes.observe(&ch);
        } else {
            pass.absorb(line);
        }
    }
    Ok(ChartMeta {
        path: path.to_path_buf(),
        title: pass.title,
        subtitle: pass.subtitle,
        artist: pass.artist,
        subartist: pass.subartist,
        genre: pass.genre,
        bpm: pass.bpm,
        playlevel: pass.playlevel,
        difficulty: pass.difficulty,
        lane_count: lanes.count(),
    })
}

pub fn load(path: &Path, lead_in_ms: f64) -> Result<Chart> {
    let text = read_text(path)?;
    let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
    let mut pass = HeaderPass::default();
    let mut raw: Vec<(u32, String, String)> = Vec::new();
    for line in header_lines(&text) {
        match parse_channel_line(line) {
            Some(parsed) => raw.push(parsed),
            None => pass.absorb(line),
        }
    }
    build_chart(pass, raw, dir, lead_in_ms)
}

#[derive(Default)]
struct ChartAcc {
    lanes: LaneSet,
    notes: Vec<Note>,
    bgm: Vec<BgmEvent>,
}

fn build_chart(
    pass: HeaderPass,
    raw: Vec<(u32, String, String)>,
    dir: PathBuf,
    lead_in_ms: f64,
) -> Result<Chart> {
    let wav_paths = resolve_wavs(&dir, &pass.wav_defs);
    let max_measure = raw.iter().map(|(m, _, _)| *m).max().unwrap_or(0);
    let scales = collect_measure_scales(&raw);
    let events = collect_timing_events(&raw, &pass.bpm_defs, &pass.stop_defs);
    let timeline = Timeline::build(lead_in_ms, pass.bpm, &scales, &events, max_measure);
    let mut acc = ChartAcc::default();
    for (measure, ch, data) in &raw {
        if matches!(ch.as_str(), "02" | "03" | "08" | "09")
            || long_channel_to_lane(ch).is_some()
        {
            continue;
        }
        materialize_channel(&mut acc, *measure, ch, data, &timeline);
    }
    materialize_long_notes(&mut acc, &raw, &timeline);
    let lane_count = acc.lanes.count();
    let mut notes = acc.notes;
    let mut bgm = acc.bgm;
    if let Some(lnobj) = pass.lnobj {
        promote_lnobj(&mut notes, lnobj);
    }
    notes.retain(|n| n.lane < lane_count);
    notes.sort_by(|a, b| a.time_ms.partial_cmp(&b.time_ms).unwrap());
    bgm.sort_by(|a, b| a.time_ms.partial_cmp(&b.time_ms).unwrap());
    let duration_ms = timeline.end_time(max_measure) + 2500.0;
    Ok(Chart {
        title: pass.title,
        subtitle: pass.subtitle,
        artist: pass.artist,
        subartist: pass.subartist,
        genre: pass.genre,
        stagefile: pass.stagefile,
        banner: pass.banner,
        maker: pass.maker,
        bpm: pass.bpm,
        playlevel: pass.playlevel,
        difficulty: pass.difficulty,
        rank: pass.rank,
        total: pass.total,
        vol_wav: pass.vol_wav,
        notes,
        bgm,
        duration_ms,
        lane_count,
        keys: keys_for(lane_count),
        wav_paths,
    })
}

fn materialize_channel(
    acc: &mut ChartAcc,
    measure: u32,
    ch: &str,
    data: &str,
    timeline: &Timeline,
) {
    let slots = parse_slots(data);
    if slots.is_empty() {
        return;
    }
    let n = slots.len() as f64;
    for (i, slot) in slots.iter().enumerate() {
        if *slot == 0 {
            continue;
        }
        let t = timeline.time_at(measure, i as f64 / n);
        if let Some(lane) = channel_to_lane(ch) {
            acc.lanes.observe(ch);
            acc.notes.push(Note {
                time_ms: t,
                lane,
                hit: false,
                keysound: Some(*slot),
                end_ms: None,
            });
        } else if ch == "01" {
            acc.bgm.push(BgmEvent {
                time_ms: t,
                keysound: *slot,
            });
        }
    }
}

fn long_channel_to_lane(ch: &str) -> Option<usize> {
    match ch {
        "51" => Some(0),
        "52" => Some(1),
        "53" => Some(2),
        "54" => Some(3),
        "55" => Some(4),
        "58" => Some(5),
        "59" => Some(6),
        _ => None,
    }
}

fn materialize_long_notes(
    acc: &mut ChartAcc,
    raw: &[(u32, String, String)],
    timeline: &Timeline,
) {
    let mut per_lane: HashMap<usize, Vec<(f64, u32)>> = HashMap::new();
    for (measure, ch, data) in raw {
        let Some(lane) = long_channel_to_lane(ch) else {
            continue;
        };
        let slots = parse_slots(data);
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
            });
        }
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
            if v > 0.0 {
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
            TimingEventKind::Stop(_) => 1,
        }
    }
}

fn collect_timing_events(
    raw: &[(u32, String, String)],
    bpm_defs: &HashMap<u32, f64>,
    stop_defs: &HashMap<u32, f64>,
) -> Vec<TimingEvent> {
    let mut out: Vec<TimingEvent> = Vec::new();
    for (measure, ch, data) in raw {
        let (slots, resolve): (Vec<u32>, Box<dyn Fn(u32) -> Option<TimingEventKind>>) =
            match ch.as_str() {
                "03" => (
                    parse_slots_hex(data),
                    Box::new(|v| Some(TimingEventKind::Bpm(v as f64))),
                ),
                "08" => (
                    parse_slots(data),
                    Box::new(|v| bpm_defs.get(&v).copied().map(TimingEventKind::Bpm)),
                ),
                "09" => (
                    parse_slots(data),
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
            .then_with(|| a.frac.partial_cmp(&b.frac).unwrap_or(std::cmp::Ordering::Equal))
            .then_with(|| a.sort_bucket().cmp(&b.sort_bucket()))
    });
    out
}

fn kind_is_positive(kind: &TimingEventKind) -> bool {
    match *kind {
        TimingEventKind::Bpm(v) | TimingEventKind::Stop(v) => v > 0.0,
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
    ) -> Self {
        let n = (max_measure as usize).saturating_add(2);
        let mut measures: Vec<MeasureSlice> = Vec::with_capacity(n);
        let mut current_ms = lead_in_ms;
        let mut current_bpm = default_bpm.max(f64::MIN_POSITIVE);
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
                    TimingEventKind::Stop(ticks) => {
                        let pause_ms = ticks / 48.0 * 60_000.0 / last.bpm;
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
            measures.push(MeasureSlice { beats, segments });
            current_ms = end_ms;
        }
        Self {
            measures,
            tail_ms: current_ms,
            tail_bpm: current_bpm,
            default_beats: 4.0,
        }
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
    title: String,
    subtitle: String,
    artist: String,
    subartist: String,
    genre: String,
    stagefile: String,
    banner: String,
    maker: String,
    bpm: f64,
    playlevel: Option<u8>,
    difficulty: Option<u8>,
    rank: Option<u8>,
    total: Option<f64>,
    vol_wav: Option<u8>,
    wav_defs: HashMap<u32, String>,
    bpm_defs: HashMap<u32, f64>,
    stop_defs: HashMap<u32, f64>,
    lnobj: Option<u32>,
    lntype: u8,
}

impl Default for HeaderPass {
    fn default() -> Self {
        Self {
            title: String::new(),
            subtitle: String::new(),
            artist: String::new(),
            subartist: String::new(),
            genre: String::new(),
            stagefile: String::new(),
            banner: String::new(),
            maker: String::new(),
            bpm: 130.0,
            playlevel: None,
            difficulty: None,
            rank: None,
            total: None,
            vol_wav: None,
            wav_defs: HashMap::new(),
            bpm_defs: HashMap::new(),
            stop_defs: HashMap::new(),
            lnobj: None,
            lntype: 1,
        }
    }
}

impl HeaderPass {
    fn absorb(&mut self, line: &str) {
        let (cmd, val) = split_cmd_value(line);
        let up = cmd.to_ascii_uppercase();
        match up.as_str() {
            "TITLE" => self.title = val.to_string(),
            "SUBTITLE" => self.subtitle = val.to_string(),
            "ARTIST" => self.artist = val.to_string(),
            "SUBARTIST" => self.subartist = val.to_string(),
            "GENRE" => self.genre = val.to_string(),
            "STAGEFILE" => self.stagefile = val.trim().to_string(),
            "BANNER" => self.banner = val.trim().to_string(),
            "MAKER" => self.maker = val.to_string(),
            "BPM" => {
                if let Ok(v) = val.trim().parse() {
                    self.bpm = v;
                }
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
            "LNOBJ" => {
                if let Ok(id) = u32::from_str_radix(val.trim(), 36) {
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
                if let Some(rest) = up.strip_prefix("WAV") {
                    if let Ok(id) = u32::from_str_radix(rest, 36) {
                        self.wav_defs.insert(id, val.trim().to_string());
                    }
                } else if let Some(rest) = up.strip_prefix("BPM") {
                    if !rest.is_empty() {
                        if let (Ok(id), Ok(v)) =
                            (u32::from_str_radix(rest, 36), val.trim().parse::<f64>())
                        {
                            if v > 0.0 {
                                self.bpm_defs.insert(id, v);
                            }
                        }
                    }
                } else if let Some(rest) = up.strip_prefix("STOP") {
                    if !rest.is_empty() {
                        if let (Ok(id), Ok(v)) =
                            (u32::from_str_radix(rest, 36), val.trim().parse::<f64>())
                        {
                            if v > 0.0 {
                                self.stop_defs.insert(id, v);
                            }
                        }
                    }
                }
            }
        }
    }
}

#[derive(Default)]
struct LaneSet(HashSet<usize>);

impl LaneSet {
    fn observe(&mut self, ch: &str) {
        if let Some(lane) = channel_to_lane(ch) {
            self.0.insert(lane);
        }
    }
    fn observe_lane(&mut self, lane: usize) {
        self.0.insert(lane);
    }
    fn count(&self) -> usize {
        let max = self.0.iter().copied().max().unwrap_or(3);
        if max >= 5 {
            7
        } else if max >= 3 {
            5
        } else {
            4
        }
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

fn parse_slots(data: &str) -> Vec<u32> {
    let bytes = data.as_bytes();
    let take = bytes.len() - (bytes.len() % 2);
    (0..take)
        .step_by(2)
        .map(|i| {
            std::str::from_utf8(&bytes[i..i + 2])
                .ok()
                .and_then(|s| u32::from_str_radix(s, 36).ok())
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
        "18" => Some(5),
        "19" => Some(6),
        _ => None,
    }
}

fn resolve_wavs(dir: &Path, defs: &HashMap<u32, String>) -> HashMap<u32, PathBuf> {
    defs.iter()
        .filter_map(|(id, name)| {
            let direct = dir.join(name);
            if direct.exists() {
                return Some((*id, direct));
            }
            let stem = Path::new(name)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or(name);
            ["wav", "ogg", "mp3", "flac"]
                .iter()
                .map(|ext| dir.join(format!("{stem}.{ext}")))
                .find(|p| p.exists())
                .map(|p| (*id, p))
        })
        .collect()
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
    fn parse_slots_reads_base36_pairs() {
        assert_eq!(parse_slots("00010203"), vec![0, 1, 2, 3]);
    }

    #[test]
    fn parse_slots_handles_base36_letters() {
        assert_eq!(parse_slots("0AZZ10"), vec![10, 1295, 36]);
    }

    #[test]
    fn parse_slots_drops_a_trailing_odd_char() {
        assert_eq!(parse_slots("010"), vec![1]);
    }

    #[test]
    fn parse_slots_treats_bad_hex_as_zero() {
        assert_eq!(parse_slots("!!!!"), vec![0, 0]);
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
        assert_eq!(channel_to_lane("16"), None);
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
        p.absorb("TITLE Foo Song");
        p.absorb("ARTIST Bar");
        p.absorb("BPM 180");
        p.absorb("PLAYLEVEL 7");
        p.absorb("DIFFICULTY 3");
        p.absorb("WAV01 kick.wav");
        p.absorb("WAVZZ hihat.ogg");
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
        p.absorb("SUBTITLE ~an interlude~");
        p.absorb("SUBARTIST arr. someone");
        p.absorb("GENRE electronic");
        p.absorb("STAGEFILE  cover.png ");
        p.absorb("BANNER  banner.png ");
        p.absorb("MAKER unhappychoice");
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
        p.absorb("RANK 2");
        p.absorb("TOTAL 260");
        p.absorb("VOLWAV 80");
        assert_eq!(p.rank, Some(2));
        assert_eq!(p.total, Some(260.0));
        assert_eq!(p.vol_wav, Some(80));
    }

    #[test]
    fn header_pass_rejects_out_of_range_rank_total_volwav() {
        let mut p = HeaderPass::default();
        p.absorb("RANK 9");
        p.absorb("TOTAL 0");
        p.absorb("TOTAL -5");
        p.absorb("VOLWAV 250");
        p.absorb("VOLWAV nonsense");
        assert_eq!(p.rank, None, "RANK > 4 is not a valid difficulty tier");
        assert_eq!(p.total, None, "TOTAL must be positive");
        assert_eq!(p.vol_wav, None, "VOLWAV is a 0..=100 percent");
    }

    #[test]
    fn timeline_defaults_every_measure_to_base_length() {
        // 240 BPM → 4 beats / (240/60) = 1 second per measure.
        let t = Timeline::build(0.0, 240.0, &HashMap::new(), &[], 3);
        assert_eq!(t.time_at(0, 0.0), 0.0);
        assert_eq!(t.time_at(1, 0.0), 1000.0);
        assert_eq!(t.time_at(2, 0.0), 2000.0);
        assert_eq!(t.end_time(2), 3000.0);
    }

    #[test]
    fn timeline_scales_a_single_measure_and_shifts_the_rest() {
        let scales: HashMap<u32, f64> = [(1u32, 0.5)].into();
        let t = Timeline::build(100.0, 240.0, &scales, &[], 3);
        assert_eq!(t.time_at(0, 0.0), 100.0);
        assert_eq!(t.time_at(1, 0.0), 1100.0);
        assert!((t.time_at(1, 1.0) - 1600.0).abs() < 1e-6);
        assert_eq!(t.time_at(2, 0.0), 1600.0);
    }

    #[test]
    fn timeline_extrapolates_past_the_max_measure() {
        let t = Timeline::build(0.0, 240.0, &HashMap::new(), &[], 1);
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
        let t = Timeline::build(0.0, 120.0, &HashMap::new(), &evs, 1);
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
        let t = Timeline::build(0.0, 120.0, &HashMap::new(), &evs, 3);
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
        let t = Timeline::build(0.0, 120.0, &HashMap::new(), &evs, 1);
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
        let evs = collect_timing_events(&raw, &bpm_defs, &stop_defs);
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
        assert!(collect_timing_events(&raw, &empty, &empty).is_empty());
    }

    #[test]
    fn header_pass_absorbs_stop_definitions() {
        let mut p = HeaderPass::default();
        p.absorb("STOP01 96");
        p.absorb("STOP02 -1");
        p.absorb("STOPZZ 24");
        assert_eq!(p.stop_defs.get(&1), Some(&96.0));
        assert!(!p.stop_defs.contains_key(&2));
        assert_eq!(p.stop_defs.get(&1295), Some(&24.0));
    }

    #[test]
    fn long_channel_to_lane_mirrors_the_regular_visible_channel_map() {
        assert_eq!(long_channel_to_lane("51"), Some(0));
        assert_eq!(long_channel_to_lane("55"), Some(4));
        assert_eq!(long_channel_to_lane("58"), Some(5));
        assert_eq!(long_channel_to_lane("59"), Some(6));
        assert_eq!(long_channel_to_lane("56"), None);
        assert_eq!(long_channel_to_lane("11"), None);
    }

    #[test]
    fn header_pass_absorbs_lnobj_and_lntype() {
        let mut p = HeaderPass::default();
        p.absorb("LNOBJ ZZ");
        p.absorb("LNTYPE 2");
        assert_eq!(p.lnobj, Some(1295));
        assert_eq!(p.lntype, 2);
    }

    #[test]
    fn header_pass_clamps_lntype_to_supported_variants() {
        let mut p = HeaderPass::default();
        p.absorb("LNTYPE 9");
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
    fn header_pass_absorbs_bpm_definitions() {
        let mut p = HeaderPass::default();
        p.absorb("BPM01 175.5");
        p.absorb("BPMZZ 60");
        p.absorb("BPM02 -20");
        assert_eq!(p.bpm_defs.get(&1), Some(&175.5));
        assert_eq!(p.bpm_defs.get(&1295), Some(&60.0));
        assert!(!p.bpm_defs.contains_key(&2), "non-positive BPM is rejected");
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
        ];
        let out = collect_measure_scales(&raw);
        assert_eq!(out.get(&0), Some(&0.5));
        assert_eq!(out.get(&2), Some(&1.25));
        assert!(!out.contains_key(&1));
        assert!(!out.contains_key(&3), "unparseable data is skipped");
        assert!(!out.contains_key(&4), "non-positive scale is skipped");
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
        assert_eq!(chart.maker, "unhappychoice");
    }

    #[test]
    fn header_pass_defaults_bpm_to_130() {
        let p = HeaderPass::default();
        assert_eq!(p.bpm, 130.0);
    }

    #[test]
    fn header_pass_ignores_malformed_numeric_headers() {
        let mut p = HeaderPass::default();
        p.absorb("BPM  not-a-number");
        p.absorb("PLAYLEVEL huh");
        assert_eq!(p.bpm, 130.0);
        assert_eq!(p.playlevel, None);
    }

    #[test]
    fn lane_set_count_climbs_with_channel_max() {
        let mut s = LaneSet::default();
        s.observe("11");
        s.observe("12");
        // Only lanes 0..=1 seen so far → clamp down to the 4K keymap.
        assert_eq!(s.count(), 4);
        s.observe("14");
        // Lane 3 pushes us out of pure 4K into the 5K keymap.
        assert_eq!(s.count(), 5);
        s.observe("19");
        // Lane 6 is the 7K scratch column.
        assert_eq!(s.count(), 7);
    }

    #[test]
    fn lane_set_ignores_non_lane_channels() {
        let mut s = LaneSet::default();
        let before = s.count();
        s.observe("01");
        s.observe("21");
        assert_eq!(
            s.count(),
            before,
            "BGM / P2 channels must not change the inferred lane count"
        );
    }

    #[test]
    fn resolve_wavs_returns_direct_match_when_file_exists() {
        let dir = tempdir();
        std::fs::write(dir.join("kick.wav"), b"").unwrap();
        let defs: HashMap<u32, String> = [(1u32, "kick.wav".to_string())].into();
        let out = resolve_wavs(&dir, &defs);
        assert_eq!(out.get(&1).unwrap(), &dir.join("kick.wav"));
    }

    #[test]
    fn resolve_wavs_tries_alternate_extensions_when_direct_missing() {
        let dir = tempdir();
        std::fs::write(dir.join("kick.ogg"), b"").unwrap();
        let defs: HashMap<u32, String> = [(1u32, "kick.wav".to_string())].into();
        let out = resolve_wavs(&dir, &defs);
        assert_eq!(out.get(&1).unwrap(), &dir.join("kick.ogg"));
    }

    #[test]
    fn resolve_wavs_drops_entries_with_no_matching_file() {
        let dir = tempdir();
        let defs: HashMap<u32, String> = [(1u32, "missing.wav".to_string())].into();
        let out = resolve_wavs(&dir, &defs);
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
