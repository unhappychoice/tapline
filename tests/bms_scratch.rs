use std::sync::atomic::{AtomicUsize, Ordering};
use tapline::{bms, chart::keys_for, game::Game, runtime::lane_for_key};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn load(channels: &str, expected: usize) -> tapline::chart::Chart {
    let path = std::env::temp_dir().join(format!(
        "tapline-scratch-{}-{}.bms",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&path, format!("#BPM 60\n{channels}\n")).unwrap();
    let chart = bms::load(&path, 0.0).unwrap();
    assert_eq!(chart.lane_count, expected);
    assert_eq!(bms::read_meta(&path).unwrap().lane_count, expected);
    assert_eq!(chart.keys.len(), expected);
    std::fs::remove_file(path).unwrap();
    chart
}

#[test]
fn scratch_input_judges_notes_and_returns_the_keysound() {
    [("#00116:02", 6, 5), ("#00119:01\n#00116:02", 8, 7)]
        .iter()
        .for_each(|(channels, count, scratch)| {
            ['a', 'A', ';'].into_iter().for_each(|key| {
                let mut game = Game::new(load(channels, *count));
                let lane = lane_for_key(key, &game.chart.keys).unwrap();
                assert_eq!(lane, *scratch);
                assert_eq!(game.hit(lane, 4000.0), Some(2));
                assert_eq!(game.perfect, 1);
            });
        });
}

#[test]
fn scratch_long_notes_and_mines_are_playable_on_both_sides() {
    [("", 12, 5, 11), ("#00119:01\n#00129:01\n", 16, 7, 15)]
        .iter()
        .for_each(|(high_keys, count, p1, p2)| {
            let chart = load(
                &format!("{high_keys}#00156:0202\n#00166:0303\n#002D6:64\n#002E6:32"),
                *count,
            );
            assert!(chart.p2_notes.is_empty());
            assert!(chart.p2_mines.is_empty());
            let mut game = Game::new(chart);
            [(*p1, 'A', 2), (*p2, ';', 3)]
                .iter()
                .for_each(|(lane, key, sound)| {
                    assert_eq!(lane_for_key(*key, &game.chart.keys), Some(*lane));
                    assert_eq!(game.hit(*lane, 4000.0), Some(*sound));
                    assert!(game
                        .chart
                        .notes
                        .iter()
                        .any(|n| n.lane == *lane && n.end_ms == Some(6000.0)));
                    game.hit(*lane, 8000.0);
                });
            assert_eq!(game.miss, 0);
            assert!(game.chart.mines.iter().all(|mine| mine.exploded));
        });
}

#[test]
fn normal_scratch_dp_and_lnobj_preserve_both_sides() {
    [
        ("", 12, vec![5, 11]),
        ("#00119:03\n#00129:04\n", 16, vec![6, 14, 7, 15]),
    ]
    .iter()
    .for_each(|(high_keys, count, lanes)| {
        let chart = load(&format!("{high_keys}#00116:01\n#00126:02"), *count);
        let mut actual = chart.notes.iter().map(|n| n.lane).collect::<Vec<_>>();
        let mut expected = lanes.clone();
        actual.sort_unstable();
        expected.sort_unstable();
        assert_eq!(actual, expected);
    });
    let chart = load("#LNOBJ ZZ\n#00116:01ZZ", 6);
    assert_eq!(chart.notes.len(), 1);
    assert!(chart.notes[0].end_ms.is_some());
}

#[test]
fn scratch_mines_and_empty_or_unpaired_channels_infer_consistent_modes() {
    ["#001D6:01", "#00156:0101"].iter().for_each(|channels| {
        load(channels, 6);
    });
    [
        "#00116:00",
        "#00156:01",
        "#00117:01\n#00127:01\n#00157:0101\n#00167:0101\n#001D7:01\n#001E7:01",
    ]
    .iter()
    .for_each(|channels| {
        let chart = load(channels, 5);
        assert!(chart.notes.is_empty());
        assert!(chart.mines.is_empty());
        assert!(chart.p2_notes.is_empty());
        assert!(chart.bgm.is_empty());
    });
}

#[test]
fn scratch_layouts_bind_every_lane_without_conflicts() {
    [6, 8, 12, 16].iter().for_each(|count| {
        let keys = keys_for(*count);
        assert_eq!(keys.len(), *count);
        assert_eq!(lane_for_key('/', &keys), None);
        assert_eq!(
            lane_for_key('A', &keys),
            Some(if *count >= 12 {
                count / 2 - 1
            } else {
                count - 1
            })
        );
        assert_eq!(lane_for_key(';', &keys), Some(count - 1));
        keys.iter().enumerate().for_each(|(lane, bindings)| {
            bindings
                .iter()
                .for_each(|key| assert_eq!(lane_for_key(*key, &keys), Some(lane)));
        });
    });
}

#[test]
fn invisible_scratch_sounds_follow_compacted_and_double_play_lanes() {
    [
        ("#00136:02", 6, vec![5]),
        ("#00119:01\n#00136:02", 8, vec![7]),
        (
            "#00116:01\n#00126:01\n#00136:02\n#00146:03",
            12,
            vec![5, 11],
        ),
        (
            "#00119:01\n#00129:01\n#00136:02\n#00146:03",
            16,
            vec![7, 15],
        ),
    ]
    .iter()
    .for_each(|(channels, count, lanes)| {
        let chart = load(channels, *count);
        assert!(chart.p2_invisible_notes.is_empty());
        assert_eq!(
            chart
                .invisible_notes
                .iter()
                .map(|n| n.lane)
                .collect::<Vec<_>>(),
            *lanes
        );
        let mut game = Game::new(chart);
        lanes.iter().enumerate().for_each(|(side, lane)| {
            game.hit(*lane, 4000.0);
            assert_eq!(game.hit(*lane, 4001.0), Some(2 + side as u32));
        });
    });
}

#[test]
fn asymmetric_scratch_dp_compacts_each_side_before_merging() {
    [
        ("#00116:02\n#00129:03", 13, vec![5, 12]),
        ("#00119:02\n#00126:03", 13, vec![6, 12]),
        ("#00116:02\n#00125:03", 11, vec![5, 10]),
    ]
    .iter()
    .for_each(|(channels, count, lanes)| {
        let chart = load(channels, *count);
        assert!(chart.p2_notes.is_empty());
        assert_eq!(
            chart.notes.iter().map(|note| note.lane).collect::<Vec<_>>(),
            *lanes
        );
    });
}
