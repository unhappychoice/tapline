use proptest::prelude::*;
use tapline::chart::{keys_for, Chart, Note};
use tapline::game::{Game, MISS_AFTER, WINDOW_GOOD, WINDOW_GREAT, WINDOW_PERFECT};

fn chart_with(notes: Vec<Note>, lane_count: usize) -> Chart {
    Chart {
        title: "prop".into(),
        bpm: 120.0,
        notes,
        duration_ms: 60_000.0,
        lane_count,
        keys: keys_for(lane_count),
        ..Chart::default()
    }
}

fn note(time_ms: f64, lane: usize) -> Note {
    Note {
        time_ms,
        lane,
        hit: false,
        keysound: Some(1),
    }
}

proptest! {
    #[test]
    fn hitting_every_note_perfectly_never_produces_a_miss(
        starts in prop::collection::vec(0.0f64..10_000.0, 1..20)
    ) {
        let mut sorted = starts;
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        // Spread notes at least one MISS_AFTER apart so successive presses
        // don't accidentally hit the wrong note.
        let mut t = 0.0;
        let notes: Vec<_> = sorted.into_iter().map(|delta| {
            t += delta.max(MISS_AFTER + 10.0);
            note(t, 0)
        }).collect();
        let total = notes.len() as u32;

        let mut game = Game::new(chart_with(notes.clone(), 4));
        for n in &notes {
            game.hit(0, n.time_ms);
        }
        prop_assert_eq!(game.miss, 0);
        prop_assert_eq!(game.perfect + game.great + game.good, total);
    }

    #[test]
    fn accuracy_stays_between_zero_and_one_hundred(
        count in 1usize..20,
        offsets in prop::collection::vec(-200.0f64..200.0, 20)
    ) {
        let notes: Vec<_> = (0..count).map(|i| note(1000.0 + i as f64 * 1000.0, 0)).collect();
        let times: Vec<_> = notes.iter().map(|n| n.time_ms).collect();
        let mut game = Game::new(chart_with(notes, 4));
        for (i, &t) in times.iter().enumerate() {
            let off = offsets[i % offsets.len()];
            game.hit(0, t + off);
        }
        game.check_misses(1_000_000.0);
        let acc = game.accuracy();
        prop_assert!((0.0..=100.0).contains(&acc), "accuracy out of range: {}", acc);
    }

    #[test]
    fn score_grows_monotonically_with_each_non_miss_hit(
        count in 1usize..15
    ) {
        let notes: Vec<_> = (0..count).map(|i| note(1000.0 + i as f64 * 1000.0, 0)).collect();
        let mut game = Game::new(chart_with(notes.clone(), 4));
        let mut last_score = 0u32;
        for n in &notes {
            game.hit(0, n.time_ms);
            prop_assert!(game.score > last_score, "score should grow after a Perfect hit");
            last_score = game.score;
        }
    }

    #[test]
    fn miss_never_decreases_score(
        count in 1usize..15
    ) {
        let notes: Vec<_> = (0..count).map(|i| note(1000.0 + i as f64 * 1000.0, 0)).collect();
        let mut game = Game::new(chart_with(notes.clone(), 4));
        let before = game.score;
        game.check_misses(1_000_000.0);
        prop_assert_eq!(game.miss, count as u32);
        prop_assert_eq!(game.score, before, "miss must not add to score");
    }

    #[test]
    fn no_press_and_no_time_advance_keeps_state_pristine(
        count in 0usize..10
    ) {
        let notes: Vec<_> = (0..count).map(|i| note(10_000.0 + i as f64 * 1000.0, 0)).collect();
        let mut game = Game::new(chart_with(notes, 4));
        game.check_misses(0.0);
        prop_assert_eq!(game.perfect + game.great + game.good + game.miss, 0);
        prop_assert_eq!(game.score, 0);
        prop_assert_eq!(game.combo, 0);
    }

    #[test]
    fn a_hit_within_the_documented_windows_produces_the_expected_tier(
        offset in -(WINDOW_GOOD)..WINDOW_GOOD
    ) {
        let mut game = Game::new(chart_with(vec![note(1000.0, 0)], 4));
        game.hit(0, 1000.0 + offset);
        let abs = offset.abs();
        if abs <= WINDOW_PERFECT {
            prop_assert_eq!(game.perfect, 1);
        } else if abs <= WINDOW_GREAT {
            prop_assert_eq!(game.great, 1);
        } else {
            prop_assert_eq!(game.good, 1);
        }
        prop_assert_eq!(game.miss, 0);
    }
}
