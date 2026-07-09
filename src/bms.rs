use crate::chart::{keys_for, BgmEvent, Chart, Note};
use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct ChartMeta {
    pub path: PathBuf,
    pub title: String,
    pub artist: String,
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
        artist: pass.artist,
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
    let measure_ms = 4.0 * 60_000.0 / pass.bpm;
    let mut acc = ChartAcc::default();
    let mut max_measure: u32 = 0;
    for (measure, ch, data) in &raw {
        max_measure = max_measure.max(*measure);
        materialize_channel(&mut acc, *measure, ch, data, measure_ms, lead_in_ms);
    }
    let lane_count = acc.lanes.count();
    let mut notes = acc.notes;
    let mut bgm = acc.bgm;
    notes.retain(|n| n.lane < lane_count);
    notes.sort_by(|a, b| a.time_ms.partial_cmp(&b.time_ms).unwrap());
    bgm.sort_by(|a, b| a.time_ms.partial_cmp(&b.time_ms).unwrap());
    let duration_ms = (max_measure + 1) as f64 * measure_ms + lead_in_ms + 2500.0;
    Ok(Chart {
        title: pass.title,
        artist: pass.artist,
        bpm: pass.bpm,
        playlevel: pass.playlevel,
        difficulty: pass.difficulty,
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
    measure_ms: f64,
    lead_in_ms: f64,
) {
    let slots = parse_slots(data);
    if slots.is_empty() {
        return;
    }
    let base = measure as f64 * measure_ms + lead_in_ms;
    let step = measure_ms / slots.len() as f64;
    for (i, slot) in slots.iter().enumerate() {
        if *slot == 0 {
            continue;
        }
        let t = base + i as f64 * step;
        if let Some(lane) = channel_to_lane(ch) {
            acc.lanes.observe(ch);
            acc.notes.push(Note {
                time_ms: t,
                lane,
                hit: false,
                keysound: Some(*slot),
            });
        } else if ch == "01" {
            acc.bgm.push(BgmEvent {
                time_ms: t,
                keysound: *slot,
            });
        }
    }
}

struct HeaderPass {
    title: String,
    artist: String,
    bpm: f64,
    playlevel: Option<u8>,
    difficulty: Option<u8>,
    wav_defs: HashMap<u32, String>,
}

impl Default for HeaderPass {
    fn default() -> Self {
        Self {
            title: String::new(),
            artist: String::new(),
            bpm: 130.0,
            playlevel: None,
            difficulty: None,
            wav_defs: HashMap::new(),
        }
    }
}

impl HeaderPass {
    fn absorb(&mut self, line: &str) {
        let (cmd, val) = split_cmd_value(line);
        let up = cmd.to_ascii_uppercase();
        match up.as_str() {
            "TITLE" => self.title = val.to_string(),
            "ARTIST" => self.artist = val.to_string(),
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
            _ => {
                if let Some(rest) = up.strip_prefix("WAV") {
                    if let Ok(id) = u32::from_str_radix(rest, 36) {
                        self.wav_defs.insert(id, val.trim().to_string());
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
