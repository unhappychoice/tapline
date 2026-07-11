use std::path::PathBuf;

#[derive(Debug, Clone, Copy, Default)]
pub struct Note {
    pub time_ms: f64,
    pub lane: usize,
    pub hit: bool,
    pub keysound: Option<u32>,
    /// For long notes, the end time of the hold in ms. `None` for taps.
    pub end_ms: Option<f64>,
    /// Timestamp of the frame that started holding this long note. `None` for
    /// taps and for LNs that haven't been pressed yet.
    pub held_since: Option<f64>,
}

#[derive(Debug, Clone, Copy)]
pub struct BgmEvent {
    pub time_ms: f64,
    pub keysound: u32,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Mine {
    pub time_ms: f64,
    pub lane: usize,
    /// Damage value from the BMS chart (0–255).
    pub damage: u32,
    /// Set once the player triggers the mine so it isn't punished twice.
    pub exploded: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BgaLayer {
    /// Channel 04 — the base background image.
    Base,
    /// Channel 06 — the "POOR" (miss) background image.
    Poor,
    /// Channel 07 — the overlay image that composites on top of the base.
    Overlay,
}

#[derive(Debug, Clone, Copy)]
pub struct BgaEvent {
    pub time_ms: f64,
    pub layer: BgaLayer,
    /// #BMPxx id whose image file should be swapped in at `time_ms`.
    pub bmp_id: u32,
}

pub struct Chart {
    pub title: String,
    pub subtitle: String,
    pub artist: String,
    pub subartist: String,
    pub genre: String,
    pub stagefile: String,
    pub banner: String,
    pub maker: String,
    pub bpm: f64,
    pub playlevel: Option<u8>,
    pub difficulty: Option<u8>,
    pub rank: Option<u8>,
    pub total: Option<f64>,
    pub vol_wav: Option<u8>,
    pub notes: Vec<Note>,
    pub mines: Vec<Mine>,
    pub p2_notes: Vec<Note>,
    pub p2_mines: Vec<Mine>,
    pub bgm: Vec<BgmEvent>,
    pub bga: Vec<BgaEvent>,
    pub bmp_paths: std::collections::HashMap<u32, PathBuf>,
    pub duration_ms: f64,
    pub lane_count: usize,
    pub keys: Vec<Vec<char>>,
    pub wav_paths: std::collections::HashMap<u32, PathBuf>,
}

impl Default for Chart {
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
            notes: Vec::new(),
            mines: Vec::new(),
            p2_notes: Vec::new(),
            p2_mines: Vec::new(),
            bgm: Vec::new(),
            bga: Vec::new(),
            bmp_paths: std::collections::HashMap::new(),
            duration_ms: 0.0,
            lane_count: 4,
            keys: keys_for(4),
            wav_paths: std::collections::HashMap::new(),
        }
    }
}

pub fn difficulty_label(d: Option<u8>) -> &'static str {
    match d {
        Some(1) => "BEGINNER",
        Some(2) => "NORMAL",
        Some(3) => "HYPER",
        Some(4) => "ANOTHER",
        Some(5) => "INSANE",
        _ => "",
    }
}

pub fn keys_for(lane_count: usize) -> Vec<Vec<char>> {
    let raw: &[&[char]] = match lane_count {
        5 => &[&['S'], &['D'], &['F', 'J'], &['K'], &['L']],
        7 => &[&['S'], &['D'], &['F'], &[' '], &['J'], &['K'], &['L']],
        // Double-play 14K: P1 hands on the home-row style layout,
        // P2 hands on the row below.
        14 => &[
            &['S'], &['D'], &['F'], &[' '], &['J'], &['K'], &['L'],
            &['Z'], &['X'], &['C'], &['V'], &['B'], &['N'], &['M'],
        ],
        _ => &[&['S'], &['D'], &['K'], &['L']],
    };
    raw.iter().map(|ks| ks.to_vec()).collect()
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
                end_ms: None, held_since: None,            });
        }
    };

    let mut t = lead_in_ms;
    let sec = 16.0 * beat;

    let easy: Vec<(f64, usize)> = (0..16)
        .map(|i| {
            (
                i as f64,
                match i % 4 {
                    0 => 0,
                    1 => 2,
                    2 => 1,
                    _ => 3,
                },
            )
        })
        .collect();
    section(t, &easy, &mut notes);
    t += sec;

    let mid: Vec<(f64, usize)> = (0..32)
        .map(|i| {
            (
                i as f64 * 0.5,
                match i % 4 {
                    0 => 0,
                    1 => 3,
                    2 => 1,
                    _ => 2,
                },
            )
        })
        .collect();
    section(t, &mid, &mut notes);
    t += sec;

    let syncopated: Vec<(f64, usize)> = (0..16)
        .flat_map(|i| {
            let base = i as f64;
            let lane = match i % 4 {
                0 => 0,
                1 => 1,
                2 => 2,
                _ => 3,
            };
            vec![
                (base, lane),
                (base + 0.5, (lane + 2) % 4),
                (base + 0.75, (lane + 1) % 4),
            ]
        })
        .collect();
    section(t, &syncopated, &mut notes);
    t += sec;

    let finale: Vec<(f64, usize)> = (0..32)
        .flat_map(|i| {
            let base = i as f64 * 0.5;
            vec![(base, i % 4), (base + 0.25, (i + 2) % 4)]
        })
        .collect();
    section(t, &finale, &mut notes);
    t += sec;

    for lane in 0..4 {
        notes.push(Note {
            time_ms: t,
            lane,
            hit: false,
            keysound: None,
            end_ms: None, held_since: None,        });
    }

    notes.sort_by(|a, b| a.time_ms.partial_cmp(&b.time_ms).unwrap());
    let duration_ms = t + 2000.0;
    Chart {
        title: "Built-in Practice".into(),
        artist: "tapline".into(),
        bpm,
        playlevel: Some(3),
        difficulty: Some(2),
        notes,
        duration_ms,
        ..Chart::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_for_4k_is_dj_max_outer_2_plus_2() {
        assert_eq!(
            keys_for(4),
            vec![vec!['S'], vec!['D'], vec!['K'], vec!['L']]
        );
    }

    #[test]
    fn keys_for_5k_center_lane_dual_binds_f_and_j() {
        let keys = keys_for(5);
        assert_eq!(keys.len(), 5);
        assert_eq!(keys[0], vec!['S']);
        assert_eq!(keys[1], vec!['D']);
        assert_eq!(keys[2], vec!['F', 'J']);
        assert_eq!(keys[3], vec!['K']);
        assert_eq!(keys[4], vec!['L']);
    }

    #[test]
    fn keys_for_7k_uses_spacebar_in_the_center() {
        let keys = keys_for(7);
        assert_eq!(keys.len(), 7);
        assert_eq!(keys[3], vec![' ']);
        assert_eq!(keys[0], vec!['S']);
        assert_eq!(keys[6], vec!['L']);
    }

    #[test]
    fn keys_for_unknown_lane_count_falls_back_to_4k() {
        assert_eq!(keys_for(0), keys_for(4));
        assert_eq!(keys_for(3), keys_for(4));
        assert_eq!(keys_for(6), keys_for(4));
        assert_eq!(keys_for(99), keys_for(4));
    }

    #[test]
    fn difficulty_label_covers_the_five_tiers() {
        assert_eq!(difficulty_label(Some(1)), "BEGINNER");
        assert_eq!(difficulty_label(Some(2)), "NORMAL");
        assert_eq!(difficulty_label(Some(3)), "HYPER");
        assert_eq!(difficulty_label(Some(4)), "ANOTHER");
        assert_eq!(difficulty_label(Some(5)), "INSANE");
    }

    #[test]
    fn difficulty_label_is_empty_for_missing_or_out_of_range() {
        assert_eq!(difficulty_label(None), "");
        assert_eq!(difficulty_label(Some(0)), "");
        assert_eq!(difficulty_label(Some(6)), "");
        assert_eq!(difficulty_label(Some(255)), "");
    }

    #[test]
    fn built_in_chart_is_4k_labelled_and_populated() {
        let c = built_in(120.0, 2000.0);
        assert_eq!(c.title, "Built-in Practice");
        assert_eq!(c.lane_count, 4);
        assert_eq!(c.keys.len(), 4);
        assert!(c.bgm.is_empty());
        assert!(c.wav_paths.is_empty());
        assert!(!c.notes.is_empty());
        assert!(c.notes.iter().all(|n| n.lane < 4));
    }

    #[test]
    fn built_in_notes_are_time_sorted() {
        let c = built_in(140.0, 0.0);
        for pair in c.notes.windows(2) {
            assert!(pair[0].time_ms <= pair[1].time_ms);
        }
    }

    #[test]
    fn built_in_bpm_scales_first_note_position() {
        let slow = built_in(60.0, 0.0);
        let fast = built_in(240.0, 0.0);
        let slow_last = slow.notes.iter().last().unwrap().time_ms;
        let fast_last = fast.notes.iter().last().unwrap().time_ms;
        assert!(
            slow_last > fast_last * 3.0,
            "expected slow (BPM 60) chart to be much longer than fast (BPM 240): {} vs {}",
            slow_last,
            fast_last
        );
    }

    #[test]
    fn built_in_lead_in_shifts_the_first_note() {
        let no_lead = built_in(120.0, 0.0);
        let with_lead = built_in(120.0, 1500.0);
        assert!(
            (with_lead.notes[0].time_ms - no_lead.notes[0].time_ms - 1500.0).abs() < 1e-6,
            "lead-in should offset every note by the given amount"
        );
    }
}
