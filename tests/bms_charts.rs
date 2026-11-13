use clap::Parser;
use std::path::{Path, PathBuf};
use tapline::{bms, bmson, chart::Chart, cli};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn load(path: &Path, lead_in_ms: f64) -> Chart {
    let mut args = cli::Args::parse_from(["tapline", "--no-audio"]);
    args.countdown_ms = lead_in_ms;
    cli::load_chart(&cli::ChartChoice::File(path.into()), &args)
        .unwrap()
        .unwrap()
}

fn read_meta(path: &Path) -> bms::ChartMeta {
    if bmson::is_bmson(path) {
        bmson::read_meta(path).unwrap()
    } else {
        bms::read_meta(path).unwrap()
    }
}

struct Expect {
    file: &'static str,
    title: &'static str,
    lanes: usize,
    level: u8,
    difficulty: u8,
    bpm: f64,
}

const CHARTS: &[Expect] = &[
    Expect {
        file: "songs/01-first-steps.bmson",
        title: "First Steps",
        lanes: 4,
        level: 1,
        difficulty: 1,
        bpm: 100.0,
    },
    Expect {
        file: "songs/02-steady-rain.bms",
        title: "Steady Rain",
        lanes: 5,
        level: 4,
        difficulty: 2,
        bpm: 130.0,
    },
    Expect {
        file: "songs/03-rolling-thunder.bms",
        title: "Rolling Thunder",
        lanes: 7,
        level: 8,
        difficulty: 3,
        bpm: 155.0,
    },
    Expect {
        file: "songs/04-neon-drive.bmson",
        title: "Neon Drive",
        lanes: 4,
        level: 3,
        difficulty: 2,
        bpm: 128.0,
    },
    Expect {
        file: "songs/05-circuit-breaker.bms",
        title: "Circuit Breaker",
        lanes: 5,
        level: 7,
        difficulty: 3,
        bpm: 148.0,
    },
    Expect {
        file: "songs/06-blue-hour.bms",
        title: "Blue Hour",
        lanes: 7,
        level: 5,
        difficulty: 2,
        bpm: 140.0,
    },
    Expect {
        file: "songs/07-final-cascade.bms",
        title: "Final Cascade",
        lanes: 7,
        level: 11,
        difficulty: 4,
        bpm: 175.0,
    },
    Expect {
        file: "songs/08-arcade-hero.bmson",
        title: "Arcade Hero",
        lanes: 4,
        level: 7,
        difficulty: 3,
        bpm: 165.0,
    },
];

#[test]
fn read_meta_matches_declared_shape_for_every_chart() {
    for e in CHARTS {
        let path = root().join(e.file);
        let m = read_meta(&path);
        assert_eq!(m.title, e.title, "{}: title", e.file);
        assert_eq!(m.playlevel, Some(e.level), "{}: level", e.file);
        assert_eq!(m.difficulty, Some(e.difficulty), "{}: difficulty", e.file);
        assert!(
            (m.bpm - e.bpm).abs() < 1e-6,
            "{}: bpm {} != {}",
            e.file,
            m.bpm,
            e.bpm
        );
    }
}

#[test]
fn every_bundled_chart_loads_end_to_end() {
    for e in CHARTS {
        let path = root().join(e.file);
        let chart = load(&path, 2000.0);
        assert!(!chart.notes.is_empty(), "{}: no notes materialised", e.file);
        assert_eq!(chart.title, e.title, "{}: title", e.file);
        assert_eq!(chart.lane_count, e.lanes, "{}: lane_count", e.file);
        assert!(chart.duration_ms > 0.0, "{}: duration is zero", e.file);
    }
}

#[test]
fn every_chart_has_time_sorted_notes() {
    for e in CHARTS {
        let path = root().join(e.file);
        let chart = load(&path, 0.0);
        for pair in chart.notes.windows(2) {
            assert!(
                pair[0].time_ms <= pair[1].time_ms,
                "{}: notes not sorted at {} vs {}",
                e.file,
                pair[0].time_ms,
                pair[1].time_ms
            );
        }
    }
}

#[test]
fn every_chart_has_time_sorted_bgm() {
    for e in CHARTS {
        let path = root().join(e.file);
        let chart = load(&path, 0.0);
        for pair in chart.bgm.windows(2) {
            assert!(
                pair[0].time_ms <= pair[1].time_ms,
                "{}: bgm not sorted",
                e.file
            );
        }
    }
}

#[test]
fn every_chart_only_touches_lanes_within_its_lane_count() {
    for e in CHARTS {
        let path = root().join(e.file);
        let chart = load(&path, 0.0);
        for note in &chart.notes {
            assert!(
                note.lane < chart.lane_count,
                "{}: note.lane {} >= lane_count {}",
                e.file,
                note.lane,
                chart.lane_count
            );
        }
    }
}

#[test]
fn every_chart_carries_the_expected_keymap_length() {
    for e in CHARTS {
        let path = root().join(e.file);
        let chart = load(&path, 0.0);
        assert_eq!(chart.keys.len(), chart.lane_count, "{}: keys len", e.file);
        for lane_keys in &chart.keys {
            assert!(!lane_keys.is_empty(), "{}: lane has no bindings", e.file);
        }
    }
}

#[test]
fn lead_in_shifts_the_first_note_by_the_specified_amount() {
    let path = root().join("songs/01-first-steps.bmson");
    let base = load(&path, 0.0);
    let led = load(&path, 2000.0);
    assert_eq!(base.notes.len(), led.notes.len());
    let delta = led.notes[0].time_ms - base.notes[0].time_ms;
    assert!(
        (delta - 2000.0).abs() < 1e-6,
        "expected lead-in to shift the first note by 2000 ms, got {}",
        delta
    );
}

#[test]
fn built_in_directory_scan_returns_all_shipped_charts() {
    let dir = root().join("songs");
    let charts = tapline::select::scan(&dir);
    assert_eq!(charts.len(), CHARTS.len());
    let titles: Vec<_> = charts.iter().map(|m| m.title.as_str()).collect();
    for e in CHARTS {
        assert!(
            titles.contains(&e.title),
            "scan should surface {}, got {:?}",
            e.title,
            titles
        );
    }
    for pair in titles.windows(2) {
        assert!(
            pair[0].to_ascii_lowercase() <= pair[1].to_ascii_lowercase(),
            "scan output should be title-sorted: {} > {}",
            pair[0],
            pair[1]
        );
    }
}

#[test]
fn read_meta_matches_the_lane_count_load_reports() {
    // The selector uses read_meta for its badge, so drift between the two
    // would show a chart as e.g. "7K" and then load it as 5K. Guard the
    // sample pack against that.
    for e in CHARTS {
        let path: &Path = &root().join(e.file);
        let m = read_meta(path);
        let c = load(path, 0.0);
        assert_eq!(
            m.lane_count, c.lane_count,
            "{}: meta lane_count {} != load lane_count {}",
            e.file, m.lane_count, c.lane_count
        );
    }
}

#[test]
fn bundled_four_key_charts_use_documented_bindings_from_file_and_selector() {
    let charts = tapline::select::scan(&root().join("songs"));
    let args = cli::Args::parse_from(["tapline", "--no-audio"]);
    for expected in CHARTS.iter().filter(|chart| chart.lanes == 4) {
        let meta = charts
            .iter()
            .find(|m| m.path == root().join(expected.file))
            .unwrap();
        assert_eq!(meta.lane_count, 4, "{}: selector lanes", expected.file);
        let selected = cli::load_chart(&cli::ChartChoice::Selected(Box::new(meta.clone())), &args)
            .unwrap()
            .unwrap();
        let direct = load(&meta.path, args.countdown_ms);
        for chart in [direct, selected] {
            assert_eq!(chart.lane_count, 4, "{}: gameplay lanes", expected.file);
            assert_eq!(chart.keys, vec![vec!['S'], vec!['D'], vec!['K'], vec!['L']]);
            assert!((0..4).all(|lane| chart.notes.iter().any(|note| note.lane == lane)));
        }
    }
}
