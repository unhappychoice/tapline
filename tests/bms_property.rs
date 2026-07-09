use proptest::prelude::*;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use tapline::bms;

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn tempdir() -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir =
        std::env::temp_dir().join(format!("tapline-bms-property-{}-{}", std::process::id(), n));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_text(text: &str) -> PathBuf {
    let dir = tempdir();
    let path = dir.join("song.bms");
    std::fs::write(&path, text).unwrap();
    path
}

proptest! {
    #[test]
    fn read_meta_never_panics_on_arbitrary_ascii(text in "\\PC*") {
        let path = write_text(&text);
        // Must not panic; may or may not surface as Ok(...).
        let _ = bms::read_meta(&path);
    }

    #[test]
    fn load_never_panics_on_arbitrary_ascii(text in "\\PC*") {
        let path = write_text(&text);
        let _ = bms::load(&path, 0.0);
    }

    #[test]
    fn header_only_variants_report_the_declared_bpm(
        bpm in 30i32..=300i32
    ) {
        let text = format!("#TITLE Test\n#BPM {}\n", bpm);
        let path = write_text(&text);
        let m = bms::read_meta(&path).unwrap();
        prop_assert!((m.bpm - bpm as f64).abs() < 1e-6);
    }

    #[test]
    fn playlevel_and_difficulty_survive_the_round_trip(
        level in 1u8..=12u8,
        difficulty in 1u8..=5u8
    ) {
        let text = format!(
            "#TITLE Test\n#BPM 130\n#PLAYLEVEL {}\n#DIFFICULTY {}\n",
            level, difficulty
        );
        let path = write_text(&text);
        let m = bms::read_meta(&path).unwrap();
        prop_assert_eq!(m.playlevel, Some(level));
        prop_assert_eq!(m.difficulty, Some(difficulty));
    }

    #[test]
    fn every_channel_note_is_ordered_and_in_range(
        // Build a tiny chart: 1..=8 measures, each with a random note slot
        measures in prop::collection::vec(0u32..=99u32, 1..=8),
        channel in prop::sample::select(&["11", "12", "13", "14", "15", "18", "19"][..])
    ) {
        let mut text = "#TITLE Prop\n#BPM 60\n#WAV01 kick.wav\n".to_string();
        for m in &measures {
            text.push_str(&format!("#{:03}{}:0100\n", m, channel));
        }
        let path = write_text(&text);
        let chart = bms::load(&path, 0.0).unwrap();
        for pair in chart.notes.windows(2) {
            prop_assert!(pair[0].time_ms <= pair[1].time_ms);
        }
        for note in &chart.notes {
            prop_assert!(note.lane < chart.lane_count);
        }
    }

    #[test]
    fn ascii_titles_survive_the_round_trip(
        // The parser trims trailing/leading whitespace via split_cmd_value,
        // so restrict to inputs that don't rely on it being kept.
        title in "[A-Za-z0-9][A-Za-z0-9 ]{0,30}[A-Za-z0-9]|[A-Za-z0-9]"
    ) {
        let text = format!("#TITLE {}\n#BPM 120\n", title);
        let path = write_text(&text);
        let m = bms::read_meta(&path).unwrap();
        prop_assert_eq!(m.title, title);
    }
}
