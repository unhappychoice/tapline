use std::path::PathBuf;

#[derive(Debug, Clone, Copy)]
pub struct Note {
    pub time_ms: f64,
    pub lane: usize,
    pub hit: bool,
    pub keysound: Option<u32>,
}

#[derive(Debug, Clone, Copy)]
pub struct BgmEvent {
    pub time_ms: f64,
    pub keysound: u32,
    pub played: bool,
}

pub struct Chart {
    pub title: String,
    pub artist: String,
    pub bpm: f64,
    pub notes: Vec<Note>,
    pub bgm: Vec<BgmEvent>,
    pub duration_ms: f64,
    pub lane_count: usize,
    pub keys: Vec<char>,
    pub wav_paths: std::collections::HashMap<u32, PathBuf>,
}

const KEYS_4: [char; 4] = ['D', 'F', 'J', 'K'];
const KEYS_5: [char; 5] = ['D', 'F', 'G', 'J', 'K'];
const KEYS_7: [char; 7] = ['S', 'D', 'F', ' ', 'J', 'K', 'L'];

pub fn keys_for(lane_count: usize) -> Vec<char> {
    match lane_count {
        5 => KEYS_5.to_vec(),
        7 => KEYS_7.to_vec(),
        _ => KEYS_4.to_vec(),
    }
}

pub fn built_in(bpm: f64, lead_in_ms: f64) -> Chart {
    let beat = 60_000.0 / bpm;
    let mut notes: Vec<Note> = Vec::new();

    let section = |offset: f64, pattern: &[(f64, usize)], notes: &mut Vec<Note>| {
        for (frac, lane) in pattern {
            notes.push(Note {
                time_ms: offset + frac * beat,
                lane: *lane,
                hit: false,
                keysound: None,
            });
        }
    };

    let mut t = lead_in_ms;
    let sec = 16.0 * beat;

    let easy: Vec<(f64, usize)> = (0..16)
        .map(|i| (i as f64, match i % 4 { 0 => 0, 1 => 2, 2 => 1, _ => 3 }))
        .collect();
    section(t, &easy, &mut notes);
    t += sec;

    let mid: Vec<(f64, usize)> = (0..32)
        .map(|i| (i as f64 * 0.5, match i % 4 { 0 => 0, 1 => 3, 2 => 1, _ => 2 }))
        .collect();
    section(t, &mid, &mut notes);
    t += sec;

    let syncopated: Vec<(f64, usize)> = (0..16).flat_map(|i| {
        let base = i as f64;
        let lane = match i % 4 { 0 => 0, 1 => 1, 2 => 2, _ => 3 };
        vec![(base, lane), (base + 0.5, (lane + 2) % 4), (base + 0.75, (lane + 1) % 4)]
    }).collect();
    section(t, &syncopated, &mut notes);
    t += sec;

    let finale: Vec<(f64, usize)> = (0..32).flat_map(|i| {
        let base = i as f64 * 0.5;
        vec![(base, i % 4), (base + 0.25, (i + 2) % 4)]
    }).collect();
    section(t, &finale, &mut notes);
    t += sec;

    for lane in 0..4 {
        notes.push(Note { time_ms: t, lane, hit: false, keysound: None });
    }

    notes.sort_by(|a, b| a.time_ms.partial_cmp(&b.time_ms).unwrap());
    let duration_ms = t + 2000.0;
    Chart {
        title: "Built-in Practice".into(),
        artist: "tapline".into(),
        bpm,
        notes,
        bgm: Vec::new(),
        duration_ms,
        lane_count: 4,
        keys: keys_for(4),
        wav_paths: Default::default(),
    }
}
