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
