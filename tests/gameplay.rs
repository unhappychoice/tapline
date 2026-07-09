use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use tapline::bms;
use tapline::game::{Game, Judgment};
use tapline::runtime::lane_for_key;

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn tempdir() -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "tapline-gameplay-test-{}-{}",
        std::process::id(),
        n
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_and_load(bms_text: &str) -> tapline::chart::Chart {
    let dir = tempdir();
    let path = dir.join("song.bms");
    std::fs::write(&path, bms_text).unwrap();
    bms::load(&path, 0.0).unwrap()
}

#[test]
fn full_run_of_a_five_note_chart_awards_five_perfects() {
    // BPM 60 → measure = 4000ms. 4 slots per measure = 1000ms each.
    let chart = write_and_load(
        "\
#TITLE Simulator
#BPM 60
#WAV01 kick.wav
#00111:01010101
#00211:0100
",
    );
    assert_eq!(chart.notes.len(), 5);
    let mut game = Game::new(chart);
    let notes_before: Vec<_> = game.chart.notes.iter().map(|n| n.time_ms).collect();
    for &t in &notes_before {
        // Press right on the beat, in-lane.
        assert!(game.hit(0, t).is_some());
    }
    assert_eq!(game.perfect, 5);
    assert_eq!(game.great, 0);
    assert_eq!(game.good, 0);
    assert_eq!(game.miss, 0);
    assert_eq!(game.combo, 5);
    assert_eq!(game.max_combo, 5);
    assert!((game.accuracy() - 100.0).abs() < 1e-9);
    assert_eq!(
        game.score,
        5 * Judgment::Perfect.points() + (1 + 2 + 3 + 4 + 5)
    );
}

#[test]
fn missing_a_note_resets_combo_and_bumps_the_miss_counter() {
    let chart = write_and_load(
        "\
#TITLE Miss
#BPM 60
#WAV01 kick.wav
#00111:01010001
",
    );
    let mut game = Game::new(chart);
    let notes: Vec<_> = game.chart.notes.iter().map(|n| n.time_ms).collect();
    // Hit first two on-beat; skip the last one entirely.
    game.hit(0, notes[0]);
    game.hit(0, notes[1]);
    assert_eq!(game.combo, 2);
    game.check_misses(notes[2] + 200.0);
    assert_eq!(game.miss, 1);
    assert_eq!(game.combo, 0);
    assert_eq!(game.max_combo, 2);
}

#[test]
fn keyboard_input_reaches_the_matching_lane_for_a_5k_chart() {
    let chart = write_and_load(
        "\
#TITLE Keyboard
#BPM 60
#WAV01 kick.wav
#00111:0100
#00113:0100
#00115:0100
",
    );
    assert_eq!(chart.lane_count, 5);
    let notes = chart.notes.clone();
    let mut game = Game::new(chart);
    // 5K binds: S D F/J K L — indices 0..4.
    assert_eq!(lane_for_key('S', &game.chart.keys), Some(0));
    assert_eq!(lane_for_key('F', &game.chart.keys), Some(2));
    assert_eq!(lane_for_key('J', &game.chart.keys), Some(2));
    assert_eq!(lane_for_key('L', &game.chart.keys), Some(4));

    // Note on lane 0 (S), lane 2 (F/J), lane 4 (L) — one each.
    let (t0, l0) = (notes[0].time_ms, notes[0].lane);
    let (t1, l1) = (notes[1].time_ms, notes[1].lane);
    let (t2, l2) = (notes[2].time_ms, notes[2].lane);
    assert_eq!((l0, l1, l2), (0, 2, 4));
    game.hit(lane_for_key('S', &game.chart.keys).unwrap(), t0);
    // Hit the center lane via the alternate binding to be sure J works too.
    game.hit(lane_for_key('J', &game.chart.keys).unwrap(), t1);
    game.hit(lane_for_key('L', &game.chart.keys).unwrap(), t2);
    assert_eq!(game.perfect, 3);
    assert_eq!(game.combo, 3);
}

#[test]
fn hits_return_the_note_s_keysound_for_the_audio_pipeline() {
    let chart = write_and_load(
        "\
#TITLE Keysound
#BPM 60
#WAV0Z kick.wav
#00111:0Z00
",
    );
    let mut game = Game::new(chart);
    let t = game.chart.notes[0].time_ms;
    let ks = game.hit(0, t).expect("hit should succeed");
    // 0Z in base-36 = 35
    assert_eq!(ks, 35);
}

#[test]
fn mixed_judgment_tiers_produce_the_expected_accuracy() {
    let chart = write_and_load(
        "\
#TITLE Tiered
#BPM 60
#WAV01 kick.wav
#00111:01010101
",
    );
    let mut game = Game::new(chart);
    let notes: Vec<_> = game.chart.notes.iter().map(|n| n.time_ms).collect();
    // Perfect / great / good / miss, in that order.
    game.hit(0, notes[0]);
    game.hit(0, notes[1] + tapline::game::WINDOW_PERFECT + 1.0);
    game.hit(0, notes[2] + tapline::game::WINDOW_GREAT + 1.0);
    game.check_misses(notes[3] + tapline::game::MISS_AFTER + 1.0);
    assert_eq!(game.perfect, 1);
    assert_eq!(game.great, 1);
    assert_eq!(game.good, 1);
    assert_eq!(game.miss, 1);
    let expected = (1.0 + 0.65 + 0.3 + 0.0) / 4.0 * 100.0;
    assert!((game.accuracy() - expected).abs() < 1e-9);
}

#[test]
fn playing_a_shipped_chart_through_hits_the_full_note_count() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("songs/01-first-steps.bms");
    let chart = bms::load(&path, 0.0).unwrap();
    let total_notes = chart.notes.len();
    let notes: Vec<_> = chart.notes.iter().map(|n| (n.time_ms, n.lane)).collect();
    let mut game = Game::new(chart);
    for (t, lane) in notes {
        game.hit(lane, t);
    }
    // Chord notes at the same time_ms both count.
    assert_eq!(
        game.perfect, total_notes as u32,
        "expected every note to be hit for a Perfect run"
    );
    assert_eq!(game.miss, 0);
    assert_eq!(game.combo, total_notes as u32);
}
