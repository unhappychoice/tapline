use tapline::{bms, game::Game};

fn chart(headers: &str, channels: &str) -> tapline::chart::Chart {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(format!("exrank-{:?}.bms", std::thread::current().id()));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, format!("#BPM 120\n{headers}\n{channels}\n")).unwrap();
    let chart = bms::load(&path, 500.0).unwrap();
    std::fs::remove_file(path).unwrap();
    chart
}

#[test]
fn rank_scales_normal_windows_and_defaults_to_normal() {
    for (rank, scale) in [(0, 0.5), (1, 0.75), (2, 1.0), (3, 1.25), (4, 1.5)] {
        let c = chart(&format!("#RANK {rank}"), "#00011:01");
        assert_eq!(c.judgment_scale(500.0), scale);
        let mut game = Game::new(c);
        game.hit(0, 500.0 + 45.0 * scale);
        assert_eq!(game.perfect, 1);
    }
    assert_eq!(chart("", "").judgment_scale(0.0), 1.0);
}

#[test]
fn defexrank_overrides_rank_in_either_header_order_and_halves_all_windows() {
    for headers in ["#DEFEXRANK 50\n#RANK 4", "#RANK 4\n#DEFEXRANK 50"] {
        for (offset, counts) in [
            (22.5, (1, 0, 0)),
            (23.0, (0, 1, 0)),
            (45.0, (0, 1, 0)),
            (46.0, (0, 0, 1)),
            (70.0, (0, 0, 1)),
            (71.0, (0, 0, 0)),
        ] {
            let mut game = Game::new(chart(headers, "#00011:01"));
            game.hit(0, 500.0 + offset);
            assert_eq!((game.perfect, game.great, game.good), counts);
        }
        let mut game = Game::new(chart(headers, "#00011:01"));
        game.check_misses(580.0);
        assert_eq!(game.miss, 0);
        game.check_misses(580.1);
        assert_eq!(game.miss, 1);
    }
}

#[test]
fn a0_changes_windows_at_note_time_including_early_hits_and_ln_ends() {
    let headers = "#DEFEXRANK 100\n#EXRANKzz 50\n#EXRANK02 200";
    let channels = "#00011:0101\n#000A0:00ZZ\n#001A0:02\n#00111:01";
    let c = chart(headers, channels);
    assert_eq!(c.exrank_defs[&1295], 50.0);
    assert_eq!(c.judgment_scale(1499.0), 1.0);
    assert_eq!(c.judgment_scale(1500.0), 0.5);
    assert_eq!(c.judgment_scale(2500.0), 2.0);
    let mut game = Game::new(c);
    game.hit(0, 470.0);
    game.hit(0, 1470.0);
    game.hit(0, 2430.0);
    assert_eq!((game.perfect, game.great), (2, 1));

    let mut game = Game::new(chart(headers, "#00051:0101\n#000A0:00ZZ"));
    game.hit(0, 500.0);
    game.release(0, 1530.0);
    assert_eq!((game.perfect, game.great), (1, 1));
    let mut game = Game::new(chart(headers, "#00051:0101\n#000A0:00ZZ"));
    game.hit(0, 500.0);
    game.check_misses(1580.1);
    assert!(game.chart.notes[0].hit);
}

#[test]
fn invalid_percentages_and_undefined_a0_objects_are_ignored() {
    let c = chart("#RANK 2\n#DEFEXRANK NaN\n#DEFEXRANK inf\n#DEFEXRANK -1\n#EXRANK01 0\n#EXRANK02 NaN\n#EXRANK03 inf\n#EXRANK000 50", "#000A0:00010203ZZ");
    assert!(c.def_exrank.is_none());
    assert!(c.exrank_defs.is_empty());
    assert!(c.exrank_events.is_empty());
    assert_eq!(c.judgment_scale(2000.0), 1.0);
}

#[test]
fn a0_uses_bpm_stop_measure_length_and_lead_in_timing() {
    let c = chart(
        "#EXRANK01 50\n#BPM01 240\n#STOP01 48",
        "#00002:0.5\n#00008:01\n#00009:01\n#001A0:01\n#00111:01",
    );
    assert_eq!(c.exrank_events[0].time_ms, 1250.0);
    assert_eq!(c.notes[0].time_ms, 1250.0);
}

#[test]
fn a0_overrides_rank_only_from_the_event_time() {
    let c = chart("#RANK 4\n#EXRANK01 50", "#00011:0101\n#000A0:0001");
    assert_eq!(c.judgment_scale(1499.0), 1.5);
    assert_eq!(c.judgment_scale(1500.0), 0.5);
    let mut game = Game::new(c);
    game.hit(0, 560.0);
    game.hit(0, 1530.0);
    assert_eq!((game.perfect, game.great), (1, 1));
}

#[test]
fn narrowed_windows_preserve_invisible_keysounds_on_empty_presses() {
    let mut game = Game::new(chart("#DEFEXRANK 50", "#00011:0101\n#00031:0002"));
    assert_eq!(game.hit(0, 1600.0), Some(2));
    assert_eq!(game.perfect + game.great + game.good + game.miss, 0);
}

#[test]
fn judgment_and_volume_changes_coexist_at_the_same_timestamp() {
    use tapline::chart::VolumeTarget;

    let c = chart(
        "#DEFEXRANK 75\n#EXRANK01 50",
        "#00011:0101\n#000A0:0001\n#00097:0040\n#00098:0080",
    );
    assert_eq!(c.judgment_scale(1499.0), 0.75);
    assert_eq!(c.judgment_scale(1500.0), 0.5);
    assert_eq!(c.volume_events.len(), 2);
    assert_eq!(c.volume_events[0].target, VolumeTarget::Bgm);
    assert_eq!(c.volume_events[0].volume, 64);
    assert_eq!(c.volume_events[1].target, VolumeTarget::Key);
    assert_eq!(c.volume_events[1].volume, 128);
    assert!(c.volume_events.iter().all(|event| event.time_ms == 1500.0));
}
