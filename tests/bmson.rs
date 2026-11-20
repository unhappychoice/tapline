use clap::Parser;
use serde_json::{json, Value};
use std::path::Path;
use tapline::{bmson, chart::BgaLayer, cli, game::Game, select};

fn fixture() -> &'static Path {
    Path::new("tests/fixtures/minimal.bmson")
}

fn load_value(value: Value) -> anyhow::Result<tapline::chart::Chart> {
    load_with_lead_in(value, 1000.0)
}

fn load_with_lead_in(value: Value, lead_in_ms: f64) -> anyhow::Result<tapline::chart::Chart> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static COUNT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "tapline-bmson-{}-{}",
        std::process::id(),
        COUNT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("test.bmson");
    std::fs::write(&path, value.to_string()).unwrap();
    let result = bmson::load(&path, lead_in_ms);
    std::fs::remove_dir_all(dir).unwrap();
    result
}

fn base() -> Value {
    json!({"version":"1.0.0", "info":{"title":"Test", "artist":"Artist", "init_bpm":120},
        "sound_channels":[]})
}

#[test]
fn fixture_loads_through_cli_and_can_be_played() {
    let args = cli::Args::parse_from([
        "tapline",
        "--file",
        fixture().to_str().unwrap(),
        "--countdown-ms",
        "1000",
    ]);
    let choice = cli::choose_chart(&mut std::io::stdout(), &args).unwrap();
    let chart = cli::load_chart(&choice, &args).unwrap().unwrap();
    assert_eq!(chart.playlevel, Some(256));
    assert_eq!(chart.lane_count, 4);
    assert_eq!(chart.notes.len(), 2);
    assert_eq!(chart.notes[1].time_ms, 1500.0);
    assert_eq!(chart.notes[1].end_ms, Some(1750.0));
    assert_eq!(chart.bgm[0].time_ms, 1750.0);
    assert_eq!(chart.sample_slices[&1].start_ms, 500.0);
    assert_eq!(chart.sample_slices[&1].duration_ms, Some(250.0));
    assert_eq!(chart.sample_slices[&2].start_ms, 0.0);
    assert_eq!(chart.sample_slices[&2].duration_ms, None);
    let mut game = Game::new(chart);
    assert_eq!(game.hit(0, 1000.0), Some(0));
    assert_eq!(game.hit(1, 1500.0), Some(1));
    game.release(1, 1750.0);
    assert_eq!(game.miss, 0);
    assert!(game.perfect >= 2);
}

#[test]
fn total_scales_gauge_recovery_from_a_perfect_hit() {
    [
        (None, 65.0),
        (Some(100.0), 65.0),
        (Some(50.0), 32.5),
        (Some(0.0), 0.0),
        (Some(-50.0), 32.5),
    ]
    .into_iter()
    .for_each(|(total, gain)| {
        let mut data = base();
        if let Some(total) = total {
            data["info"]["total"] = json!(total);
        }
        data["sound_channels"] = json!([{"name":"sound.wav","notes":[
            {"x":1,"y":0}, {"x":1,"y":240}, {"x":1,"y":480}, {"x":1,"y":720}
        ]}]);
        let mut game = Game::new(load_value(data).unwrap());
        assert_eq!(game.hit(0, 1000.0), Some(0));
        assert_eq!(game.perfect, 1);
        assert_eq!(game.gauge.value(), 20.0 + gain, "total: {total:?}");
    });
}

#[test]
fn judge_rank_scales_taps_releases_and_deadlines() {
    let mut value: Value = serde_json::from_str(include_str!("fixtures/minimal.bmson")).unwrap();
    value["info"]["judge_rank"] = json!(50);
    let mut game = Game::new(load_value(value.clone()).unwrap());
    game.hit(0, 1100.0);
    assert!(!game.chart.notes[0].hit);

    game.hit(1, 1500.0);
    game.release(1, 1850.0);
    assert_eq!((game.good, game.perfect, game.miss), (0, 1, 1));

    let mut game = Game::new(load_value(value).unwrap());
    game.check_misses(1080.0);
    assert_eq!(game.miss, 0);
    game.check_misses(1081.0);
    assert_eq!(game.miss, 1);
    game.hit(1, 1500.0);
    game.check_misses(1830.0);
    assert!(!game.chart.notes[1].hit);
    game.check_misses(1831.0);
    assert!(game.chart.notes[1].hit);
    assert_eq!(game.perfect, 2);
}

#[test]
fn missing_or_invalid_judge_rank_uses_normal_windows() {
    let mut value: Value = serde_json::from_str(include_str!("fixtures/minimal.bmson")).unwrap();
    let mut game = Game::new(load_value(value.clone()).unwrap());
    game.hit(0, 1100.0);
    assert_eq!(game.good, 1);

    for invalid in [0.0, -50.0, 1.7e308] {
        value["info"]["judge_rank"] = json!(invalid);
        let chart = load_value(value.clone()).unwrap();
        assert_eq!(chart.judgment_scale(1000.0), 1.0);
    }
}

#[test]
fn selector_discovers_bmson_metadata() {
    let charts = select::scan(Path::new("tests/fixtures"));
    let meta = charts.iter().find(|m| m.path == fixture()).unwrap();
    assert_eq!(meta.title, "Minimal bmson");
    assert_eq!(meta.bpm, 120.0);
    assert!(meta.preview.is_none());
    assert_eq!(meta.playlevel, Some(256));
    assert_eq!(meta.random_seed, 0);
    let args = cli::Args::parse_from(["tapline", "--countdown-ms", "1000"]);
    let chart = cli::load_chart(&cli::ChartChoice::Selected(Box::new(meta.clone())), &args)
        .unwrap()
        .unwrap();
    assert_eq!(chart.title, meta.title);
    assert_eq!(chart.playlevel, meta.playlevel);
    assert_eq!(chart.notes.len(), 2);
    assert_eq!(chart.notes[0].time_ms, 1000.0);
    assert!(bmson::is_bmson(Path::new("chart.BMSON")));
}

#[test]
fn artwork_metadata_is_available_to_selector_and_intro() {
    let chart = bmson::load(fixture(), 0.0).unwrap();
    let meta = bmson::read_meta(fixture()).unwrap();
    assert_eq!(chart.asset_dir, fixture().parent().unwrap());
    assert_eq!(chart.stagefile, "stage.png");
    assert_eq!(chart.banner, "banner.png");
    assert_eq!(meta.banner, chart.banner);
    assert!(meta.comment.is_empty());
}

#[test]
fn stops_follow_notes_and_use_last_bpm_at_same_pulse() {
    let mut data = base();
    data["bpm_events"] = json!([{"y":240,"bpm":60},{"y":240,"bpm":240}]);
    data["stop_events"] = json!([{"y":240,"duration":120},{"y":240,"duration":120}]);
    data["sound_channels"] = json!([{"name":"sound.wav","notes":[
        {"x":1,"y":480,"c":true}, {"x":1,"y":0},
        {"x":2,"y":240,"l":240,"c":true}]}]);
    let c = load_value(data).unwrap();
    assert_eq!(
        c.notes.iter().map(|n| n.time_ms).collect::<Vec<_>>(),
        [1000.0, 1500.0, 2000.0]
    );
    assert_eq!(c.notes[1].end_ms, Some(2000.0));
    assert_eq!(c.sample_slices[&1].duration_ms, Some(500.0));
    assert_eq!(c.sample_slices[&2].start_ms, 1000.0);
}

#[test]
fn simultaneous_notes_share_slice_and_restart_together() {
    let mut data = base();
    data["sound_channels"] = json!([{"name":"sound.wav","notes":[
        {"x":1,"y":0}, {"x":1,"y":240,"c":true},
        {"x":2,"y":240,"c":false}, {"x":3,"y":480,"c":true}]}]);
    let c = load_value(data).unwrap();
    assert_eq!(c.notes[1].keysound, c.notes[2].keysound);
    assert_eq!(c.sample_slices.len(), 3);
    assert_eq!(c.sample_slices[&1].start_ms, 0.0);
    assert_eq!(c.sample_slices[&2].start_ms, 500.0);
}

#[test]
fn bga_layers_and_background_notes_are_converted() {
    let mut data = base();
    data["bga"] = json!({"bga_header":[{"id":42,"name":"images\\image.png"}],
        "bga_events":[{"y":0,"id":42}], "layer_events":[{"y":240,"id":42}],
        "poor_events":[{"y":480,"id":42}]});
    data["sound_channels"] = json!([{"name":"sound.wav","notes":[
        {"x":null,"y":0}, {"x":8,"y":240}]}]);
    let c = load_value(data).unwrap();
    assert!(c.notes.is_empty());
    assert_eq!(c.bgm.len(), 2);
    assert_eq!(
        c.bga.iter().map(|e| e.layer).collect::<Vec<_>>(),
        [BgaLayer::Base, BgaLayer::Overlay, BgaLayer::Poor]
    );
    assert_eq!(c.bga[2].time_ms, 2000.0);
    assert!(c.bmp_paths.is_empty());
}

#[test]
fn resolution_defaults_and_double_play_mapping() {
    for resolution in [json!(null), json!(0), json!(480)] {
        let mut data = base();
        data["info"]["mode_hint"] = json!("beat-14k");
        data["info"]["resolution"] = resolution.clone();
        data["sound_channels"] = json!([{"name":"sound.wav","notes":[
            {"x":9,"y":480}, {"x":15,"y":480}, {"x":16,"y":480}]}]);
        let c = load_value(data).unwrap();
        assert_eq!(c.keys.len(), 14);
        assert_eq!(c.notes.iter().map(|n| n.lane).collect::<Vec<_>>(), [7, 13]);
        assert_eq!(
            c.notes[0].time_ms,
            if resolution == json!(480) {
                1500.0
            } else {
                2000.0
            }
        );
        assert!(c.bgm.is_empty());
    }
}

#[test]
fn invalid_versions_timing_and_modes_are_rejected() {
    let mut cases = Vec::new();
    let mut invalid = base();
    invalid["version"] = json!("0.21");
    cases.push(invalid);
    let mut invalid = base();
    invalid.as_object_mut().unwrap().remove("version");
    cases.push(invalid);
    let mut invalid = base();
    invalid["info"]["init_bpm"] = json!(0);
    cases.push(invalid);
    let mut invalid = base();
    invalid["info"]["mode_hint"] = json!("popn-9k");
    cases.push(invalid);
    let mut invalid = base();
    invalid["bpm_events"] = json!([{"y":0,"bpm":-1}]);
    cases.push(invalid);
    cases
        .into_iter()
        .for_each(|data| assert!(load_value(data).is_err()));
}

#[test]
fn layered_notes_are_judged_once_and_keep_distinct_slices_in_channel_order() {
    let mut data = base();
    data["sound_channels"] = json!([
        {"name":"sound.wav","notes":[{"x":1,"y":0}, {"x":2,"y":0}, {"x":1,"y":0}]},
        {"name":"sound.wav","notes":[{"x":1,"y":0}, {"x":1,"y":0}]},
        {"name":"third.wav","notes":[{"x":1,"y":0}]}
    ]);
    let chart = load_value(data).unwrap();
    assert_eq!(chart.notes.len(), 2);
    let layered = chart.notes[0].keysound.unwrap();
    assert_eq!(chart.sound_layers[&layered], [0, 1, 2]);
    assert_eq!(chart.notes[1].keysound, Some(0));
    let mut game = Game::new(chart);
    game.hit(0, 1000.0);
    game.hit(1, 1000.0);
    game.check_misses(2000.0);
    assert_eq!(game.perfect, 2);
    assert_eq!(game.miss, 0);
}

#[test]
fn shared_playable_slices_suppress_bgm_and_duplicate_bgm_plays_once() {
    let mut data = base();
    data["sound_channels"] = json!([
        {"name":"sound.wav","notes":[
            {"x":0,"y":0}, {"x":1,"y":0}, {"x":1,"y":0},
            {"x":0,"y":240}, {"x":null,"y":240}]},
        {"name":"other.wav","notes":[{"x":0,"y":0}]}
    ]);
    let chart = load_value(data).unwrap();
    assert_eq!(chart.notes.len(), 1);
    assert_eq!(chart.notes[0].keysound, Some(0));
    assert!(chart.sound_layers.is_empty());
    assert_eq!(
        chart.bgm.iter().map(|e| e.keysound).collect::<Vec<_>>(),
        [2, 1]
    );
}

#[test]
fn layered_long_notes_merge_and_unequal_lengths_are_rejected() {
    for lengths in [[240, 240], [0, 240], [240, 480]] {
        let mut data = base();
        data["sound_channels"] = json!([
            {"name":"first.wav","notes":[{"x":1,"y":0,"l":lengths[0]}]},
            {"name":"second.wav","notes":[{"x":1,"y":0,"l":lengths[1]}]}
        ]);
        let result = load_value(data);
        if lengths[0] != lengths[1] {
            assert!(result
                .err()
                .unwrap()
                .to_string()
                .contains("unequal lengths"));
        } else {
            let chart = result.unwrap();
            assert_eq!(chart.notes.len(), 1);
            assert_eq!(chart.notes[0].end_ms, Some(1500.0));
            assert_eq!(
                chart.sound_layers[&chart.notes[0].keysound.unwrap()],
                [0, 1]
            );
        }
    }
}

fn chart_with_rounded_timestamps() -> Value {
    let mut data = base();
    data["info"]["init_bpm"] = json!(1e20);
    data["info"]["resolution"] = json!(240);
    data["info"]["mode_hint"] = json!("generic-4keys");
    data
}

#[test]
fn distinct_pulses_with_equal_timestamps_keep_separate_judgments() {
    let mut data = chart_with_rounded_timestamps();
    data["sound_channels"] = json!([
        {"name":"sound.wav","notes":[{"x":1,"y":0}, {"x":1,"y":1}]}
    ]);
    let chart = load_with_lead_in(data, 2000.0).unwrap();
    assert_eq!(chart.notes.len(), 2);
    assert!(chart.notes.iter().all(|note| note.time_ms == 2000.0));
    let mut game = Game::new(chart);
    game.hit(0, 2000.0);
    game.hit(0, 2000.0);
    assert_eq!(game.perfect, 2);
}

#[test]
fn unequal_source_lengths_with_equal_end_timestamps_are_rejected() {
    let mut data = chart_with_rounded_timestamps();
    data["sound_channels"] = json!([
        {"name":"first.wav","notes":[{"x":1,"y":0,"l":1}]},
        {"name":"second.wav","notes":[{"x":1,"y":0,"l":2}]}
    ]);
    let error = load_with_lead_in(data, 2000.0).err().unwrap();
    assert!(error.to_string().contains("unequal lengths"));
}
