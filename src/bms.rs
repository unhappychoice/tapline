use crate::chart::{keys_for, BgmEvent, Chart, Note};
use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub fn load(path: &Path, lead_in_ms: f64) -> Result<Chart> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot read {}", path.display()))?;
    let text = decode_text(&bytes);
    let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();

    let mut title = String::new();
    let mut artist = String::new();
    let mut bpm: f64 = 130.0;
    let mut wav_defs: HashMap<u32, String> = HashMap::new();
    let mut raw: Vec<(u32, String, String)> = Vec::new();

    for line in text.lines() {
        let line = line.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
        if line.is_empty() || !line.starts_with('#') { continue; }
        let body = &line[1..];

        if let Some(parsed) = parse_channel_line(body) {
            raw.push(parsed);
            continue;
        }
        let (cmd, val) = split_cmd_value(body);
        let up = cmd.to_ascii_uppercase();
        match up.as_str() {
            "TITLE"  => title = val.to_string(),
            "ARTIST" => artist = val.to_string(),
            "BPM"    => { if let Ok(v) = val.trim().parse() { bpm = v; } }
            _ => {
                if let Some(rest) = up.strip_prefix("WAV") {
                    if let Ok(id) = u32::from_str_radix(rest, 36) {
                        wav_defs.insert(id, val.trim().to_string());
                    }
                }
            }
        }
    }

    let wav_paths = resolve_wavs(&dir, &wav_defs);
    let measure_ms = 4.0 * 60_000.0 / bpm;

    let mut used_lanes: HashSet<usize> = HashSet::new();
    let mut notes: Vec<Note> = Vec::new();
    let mut bgm: Vec<BgmEvent> = Vec::new();
    let mut max_measure: u32 = 0;

    for (measure, ch, data) in &raw {
        max_measure = max_measure.max(*measure);
        let slots = parse_slots(data);
        let n = slots.len();
        if n == 0 { continue; }
        let base = *measure as f64 * measure_ms + lead_in_ms;
        let step = measure_ms / n as f64;
        for (i, slot) in slots.iter().enumerate() {
            if *slot == 0 { continue; }
            let t = base + i as f64 * step;
            if let Some(lane) = channel_to_lane(ch) {
                used_lanes.insert(lane);
                notes.push(Note { time_ms: t, lane, hit: false, keysound: Some(*slot) });
            } else if ch == "01" {
                bgm.push(BgmEvent { time_ms: t, keysound: *slot, played: false });
            }
        }
    }

    let lane_count = determine_lane_count(&used_lanes);
    notes.retain(|n| n.lane < lane_count);
    notes.sort_by(|a, b| a.time_ms.partial_cmp(&b.time_ms).unwrap());
    bgm.sort_by(|a, b| a.time_ms.partial_cmp(&b.time_ms).unwrap());

    let duration_ms = (max_measure + 1) as f64 * measure_ms + lead_in_ms + 2500.0;

    Ok(Chart {
        title, artist, bpm, notes, bgm, duration_ms,
        lane_count,
        keys: keys_for(lane_count),
        wav_paths,
    })
}

fn decode_text(bytes: &[u8]) -> String {
    if let Ok(s) = std::str::from_utf8(bytes) { return s.to_string(); }
    encoding_rs::SHIFT_JIS.decode(bytes).0.into_owned()
}

fn parse_channel_line(body: &str) -> Option<(u32, String, String)> {
    if body.len() < 7 || !body.is_ascii() { return None; }
    let colon = body.find(':')?;
    if colon != 5 { return None; }
    let measure = body.get(0..3)?.parse::<u32>().ok()?;
    let channel = body.get(3..5)?.to_ascii_uppercase();
    if !channel.chars().all(|c| c.is_ascii_alphanumeric()) { return None; }
    let data = body.get(colon + 1..)?.trim().to_string();
    Some((measure, channel, data))
}

fn split_cmd_value(body: &str) -> (&str, &str) {
    match body.find(char::is_whitespace) {
        Some(i) => (&body[..i], body[i..].trim_start()),
        None    => (body, ""),
    }
}

fn parse_slots(data: &str) -> Vec<u32> {
    let bytes = data.as_bytes();
    let take = bytes.len() - (bytes.len() % 2);
    (0..take).step_by(2).map(|i| {
        std::str::from_utf8(&bytes[i..i+2]).ok()
            .and_then(|s| u32::from_str_radix(s, 36).ok())
            .unwrap_or(0)
    }).collect()
}

fn channel_to_lane(ch: &str) -> Option<usize> {
    match ch {
        "11" => Some(0), "12" => Some(1), "13" => Some(2),
        "14" => Some(3), "15" => Some(4),
        "18" => Some(5), "19" => Some(6),
        _ => None,
    }
}

fn determine_lane_count(used: &HashSet<usize>) -> usize {
    let max = used.iter().copied().max().unwrap_or(3);
    if max >= 5 { 7 } else if max >= 3 { 5 } else { 4 }
}

fn resolve_wavs(dir: &Path, defs: &HashMap<u32, String>) -> HashMap<u32, PathBuf> {
    let mut out = HashMap::new();
    for (id, name) in defs {
        let direct = dir.join(name);
        if direct.exists() { out.insert(*id, direct); continue; }
        let stem = Path::new(name).file_stem().and_then(|s| s.to_str()).unwrap_or(name);
        for ext in ["wav", "ogg", "mp3", "flac"] {
            let p = dir.join(format!("{stem}.{ext}"));
            if p.exists() { out.insert(*id, p); break; }
        }
    }
    out
}
