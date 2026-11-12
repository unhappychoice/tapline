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

fn long_note_game(header: &str, channel: &str) -> Game {
    Game::new(write_and_load(&format!("#BPM 60\n{header}\n{channel}\n")))
}

#[test]
fn lnmode_is_loaded_and_invalid_or_missing_values_default_to_cn() {
    use tapline::chart::LnMode;
    for (header, mode) in [
        ("#LNMODE 1", LnMode::Ln),
        ("#LNMODE 2", LnMode::Cn),
        ("#LNMODE 3", LnMode::Hcn),
        ("#lnmode 1", LnMode::Ln),
        ("", LnMode::Cn),
        ("#LNMODE 0", LnMode::Cn),
        ("#LNMODE 4", LnMode::Cn),
        ("#LNMODE invalid", LnMode::Cn),
    ] {
        let game = long_note_game(header, "#00151:0101");
        assert_eq!(game.chart.ln_mode, mode, "{header}");
    }
}

#[test]
fn repeated_lnmode_headers_use_the_last_value() {
    use tapline::chart::LnMode;
    for (value, mode) in [
        ("1", LnMode::Ln),
        ("2", LnMode::Cn),
        ("3", LnMode::Hcn),
        ("0", LnMode::Cn),
        ("4", LnMode::Cn),
        ("invalid", LnMode::Cn),
    ] {
        for previous in ["1", "3"] {
            let header = format!("#LNMODE {previous}\n#LNMODE {value}");
            let game = long_note_game(&header, "#00151:0101");
            assert_eq!(game.chart.ln_mode, mode, "{header}");
        }
    }
}

#[test]
fn ln_completion_does_not_depend_on_release_timing_or_encoding() {
    for channel in ["#00151:0101", "#LNOBJ ZZ\n#00111:01ZZ"] {
        for release_delay in [0.0, 60.0, 120.0, 500.0] {
            let mut game = long_note_game("#LNMODE 1", channel);
            game.hit(0, 4000.0);
            game.release(0, 6000.0 + release_delay);
            game.check_misses(7000.0);
            assert_eq!(
                (game.perfect, game.great, game.good, game.miss),
                (2, 0, 0, 0)
            );
            assert!(game.chart.notes[0].hit);
            assert!(game.chart.notes[0].held_since.is_none());
        }
    }
}

#[test]
fn ln_completes_at_the_end_without_a_release_event() {
    let mut game = long_note_game("#LNMODE 1", "#00151:0101");
    game.hit(0, 4000.0);
    game.check_misses(5999.0);
    assert!(!game.chart.notes[0].hit);
    game.check_misses(6000.0);
    assert!(game.chart.notes[0].hit);
    game.release(0, 6500.0);
    game.check_misses(7000.0);
    assert_eq!((game.perfect, game.miss, game.combo), (2, 0, 2));
}

#[test]
fn ln_requires_holding_through_the_end() {
    let mut game = long_note_game("#LNMODE 1", "#00151:0101");
    game.hit(0, 4000.0);
    game.release(0, 5999.0);
    game.check_misses(7000.0);
    assert_eq!((game.perfect, game.miss, game.combo), (1, 1, 0));
}

#[test]
fn cn_and_hcn_preserve_release_judgments_and_auto_completion() {
    for header in ["", "#LNMODE 2", "#LNMODE 3"] {
        for (delay, expected) in [
            (0.0, (2, 0, 0, 0)),
            (60.0, (1, 1, 0, 0)),
            (120.0, (1, 0, 1, 0)),
            (500.0, (1, 0, 0, 1)),
        ] {
            let mut game = long_note_game(header, "#00151:0101");
            game.hit(0, 4000.0);
            game.release(0, 6000.0 + delay);
            assert_eq!((game.perfect, game.great, game.good, game.miss), expected);
        }
        let mut game = long_note_game(header, "#00151:0101");
        game.hit(0, 4000.0);
        game.check_misses(6000.0);
        assert!(!game.chart.notes[0].hit);
        game.check_misses(6161.0);
        assert_eq!((game.perfect, game.miss), (2, 0));
    }
}

#[test]
fn ln_with_an_unpressed_start_is_missed_once() {
    let mut game = long_note_game("#LNMODE 1", "#00151:0101");
    game.check_misses(4161.0);
    game.release(0, 6000.0);
    game.check_misses(7000.0);
    assert_eq!((game.perfect, game.miss), (0, 1));
    assert!(game.chart.notes[0].hit);
}

#[test]
fn invisible_notes_update_empty_press_sounds_without_judgment() {
    let chart = write_and_load(
        "#BPM 60\n#00011:01\n#00031:000Z0000\n#00031:00020000\n#00031:00000003\n#00111:04\n",
    );
    assert_eq!(chart.notes.len(), 2);
    assert_eq!(chart.invisible_notes.len(), 2);
    assert_eq!(chart.invisible_notes[0].keysound, 2);
    assert_eq!(chart.invisible_notes[0].time_ms, 1000.0);
    assert!(chart.bgm.is_empty());
    let mut game = Game::new(chart);
    assert_eq!(game.hit(0, 0.0), Some(1));
    assert_eq!(game.hit(0, 999.0), Some(1));
    assert_eq!(game.hit(0, 1000.0), Some(2));
    game.check_misses(3001.0);
    assert_eq!(game.hit(0, 3001.0), Some(3));
    assert_eq!(game.hit(1, 3001.0), None);
    assert_eq!(game.hit(0, 4000.0), Some(4));
    game.check_misses(6000.0);
    assert_eq!((game.score, game.combo, game.max_combo), (603, 2, 2));
    assert_eq!(
        (game.perfect, game.great, game.good, game.miss),
        (2, 0, 0, 0)
    );
}

#[test]
fn invisible_only_lanes_are_available_without_creating_playable_notes() {
    let chart = write_and_load("#BPM 60\n#00039:0Z\n#00049:02\n");
    assert_eq!(chart.lane_count, 7);
    assert!(chart.notes.is_empty());
    assert!(chart.p2_notes.is_empty());
    assert_eq!(chart.p2_invisible_notes.len(), 1);
    assert_eq!(chart.p2_invisible_notes[0].lane, 6);
    let mut game = Game::new(chart);
    assert_eq!(game.hit(6, 1.0), Some(35));
    game.check_misses(5000.0);
    assert_eq!(
        (game.score, game.combo, game.miss, game.perfect),
        (0, 0, 0, 0)
    );
}

#[test]
fn double_play_invisible_notes_use_the_second_side_lanes() {
    let chart = write_and_load("#BPM 60\n#00019:01\n#00029:02\n#00039:0003\n#00049:0004\n");
    assert_eq!(chart.lane_count, 14);
    assert!(chart.p2_invisible_notes.is_empty());
    let mut game = Game::new(chart);
    assert_eq!(game.hit(6, 0.0), Some(1));
    assert_eq!(game.hit(13, 0.0), Some(2));
    assert_eq!(game.hit(6, 2001.0), Some(3));
    assert_eq!(game.hit(13, 2001.0), Some(4));
    assert_eq!(game.perfect, 2);
}

#[test]
fn noncanonical_double_play_offsets_invisible_notes_by_the_first_side_width() {
    [
        ("15", "25", "45", 10, 9),
        ("19", "25", "45", 12, 11),
        ("15", "29", "49", 12, 11),
        ("11", "25", "45", 9, 8),
    ]
    .iter()
    .for_each(|(p1, p2, invisible, lanes, last_lane)| {
        let chart = write_and_load(&format!(
            "#BPM 60\n#000{p1}:01\n#000{p2}:02\n#000{invisible}:0004\n"
        ));
        assert_eq!(chart.lane_count, *lanes);
        assert_eq!(chart.keys.len(), *lanes);
        assert!(chart.p2_invisible_notes.is_empty());
        let mut game = Game::new(chart);
        assert_eq!(game.hit(*last_lane, 0.0), Some(2));
        assert_eq!(game.hit(*last_lane, 2001.0), Some(4));
        assert_eq!(game.perfect, 1);
    });
}

#[test]
fn pms_invisible_notes_follow_the_nine_button_mapping() {
    let dir = tempdir();
    let path = dir.join("song.pms");
    std::fs::write(&path, "#BPM 60\n#00035:01\n#00042:02\n#00045:03\n").unwrap();
    let chart = bms::load(&path, 500.0).unwrap();
    assert_eq!(chart.lane_count, 9);
    assert!(chart.notes.is_empty());
    assert!(chart.p2_invisible_notes.is_empty());
    assert_eq!(
        chart
            .invisible_notes
            .iter()
            .map(|n| (n.lane, n.time_ms))
            .collect::<Vec<_>>(),
        vec![(4, 500.0), (5, 500.0), (8, 500.0)]
    );
}

#[test]
fn cn_and_hcn_use_rank_windows_for_release_and_auto_completion() {
    for mode in [2, 3] {
        for (rank, miss_delay, expected) in [(0, 80.0, (1, 0, 1)), (4, 240.0, (2, 0, 0))] {
            let header = format!("#LNMODE {mode}\n#RANK {rank}");
            let mut game = long_note_game(&header, "#00151:0101");
            game.hit(0, 4000.0);
            game.release(0, 6060.0);
            assert_eq!((game.perfect, game.great, game.good), expected);

            let mut game = long_note_game(&header, "#00151:0101");
            game.hit(0, 4000.0);
            game.check_misses(6000.0 + miss_delay);
            assert!(!game.chart.notes[0].hit);
            game.check_misses(6001.0 + miss_delay);
            assert_eq!((game.perfect, game.miss), (2, 0));
        }
    }
}

#[test]
fn ln_completion_ignores_rank_but_unpressed_starts_use_rank_delay() {
    for (rank, miss_delay) in [(0, 80.0), (4, 240.0)] {
        let header = format!("#LNMODE 1\n#RANK {rank}");
        let mut game = long_note_game(&header, "#00151:0101");
        game.hit(0, 4000.0);
        game.check_misses(6000.0);
        assert_eq!((game.perfect, game.miss), (2, 0));

        let mut game = long_note_game(&header, "#00151:0101");
        game.check_misses(4000.0 + miss_delay);
        assert_eq!(game.miss, 0);
        game.check_misses(4001.0 + miss_delay);
        assert_eq!(game.miss, 1);
    }
}
