use super::*;
use serde_json::{json, Value};
use std::cell::Cell;

thread_local! {
    pub(super) static CHANNEL_BUILDS: Cell<usize> = const { Cell::new(0) };
    pub(super) static ASSET_RESOLUTIONS: Cell<usize> = const { Cell::new(0) };
}

fn with_chart(data: &Value, test: impl FnOnce(&Path)) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static COUNT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "tapline-bmson-meta-{}-{}",
        std::process::id(),
        COUNT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("chart.bmson");
    std::fs::write(&path, data.to_string()).unwrap();
    test(&path);
    std::fs::remove_dir_all(dir).unwrap();
}

fn base() -> Value {
    json!({"version":"1.0.0", "info":{
        "title":"Title", "subtitle":"Subtitle", "artist":"Artist",
        "subartists":["One", "Two"], "genre":"Genre", "init_bpm":123,
        "level":8, "chart_name":"hYpEr", "banner_image":"banner.png"
    }, "sound_channels":[]})
}

#[test]
fn metadata_matches_gameplay_for_every_supported_mode() {
    [
        None,
        Some("beat-5k"),
        Some("beat-7k"),
        Some("beat-14k"),
        Some("popn-5k"),
        Some("generic-4keys"),
        Some("generic-5keys"),
        Some("generic-7keys"),
        Some("generic-14keys"),
    ]
    .into_iter()
    .for_each(|mode| {
        let mut data = base();
        data["info"]["mode_hint"] = json!(mode);
        with_chart(&data, |path| {
            let meta = read_meta(path).unwrap();
            let chart = load(path, 0.0).unwrap();
            assert_eq!(
                (
                    meta.title,
                    meta.subtitle,
                    meta.artist,
                    meta.subartist,
                    meta.genre,
                    meta.banner,
                    meta.comment
                ),
                (
                    chart.title,
                    chart.subtitle,
                    chart.artist,
                    chart.subartist,
                    chart.genre,
                    chart.banner,
                    chart.comment
                )
            );
            assert_eq!(
                (meta.bpm, meta.playlevel, meta.difficulty, meta.lane_count),
                (
                    chart.bpm,
                    chart.playlevel,
                    chart.difficulty,
                    chart.lane_count
                )
            );
            assert_eq!(meta.path, path);
            assert_eq!(meta.random_seed, 0);
            assert!(meta.preview.is_none());
        });
    });
}

#[test]
fn metadata_never_builds_channels_or_resolves_assets_as_content_grows() {
    [(1, 1), (20, 100), (200, 200)]
        .into_iter()
        .for_each(|(channels, notes)| {
            let mut data = base();
            data["sound_channels"] = json!((0..channels)
                .map(|_| json!({
                    "name":"missing.wav", "notes":(0..notes).map(|y|
                        json!({"x":1,"y":y})).collect::<Vec<_>>()
                }))
                .collect::<Vec<_>>());
            with_chart(&data, |path| {
                CHANNEL_BUILDS.set(0);
                ASSET_RESOLUTIONS.set(0);
                assert_eq!(read_meta(path).unwrap().title, "Title");
                assert_eq!(CHANNEL_BUILDS.get(), 0);
                assert_eq!(ASSET_RESOLUTIONS.get(), 0);
                let chart = load(path, 0.0).unwrap();
                assert_eq!(CHANNEL_BUILDS.get(), channels);
                assert_eq!(ASSET_RESOLUTIONS.get(), channels);
                assert_eq!(chart.sample_slices.len(), channels * notes);
            });
        });
}

#[test]
fn metadata_rejects_invalid_format_mode_and_timing() {
    let mut variants = vec![base(); 5];
    variants[0]["version"] = json!("0.21.0");
    variants[1]["info"]["mode_hint"] = json!("unsupported");
    variants[2]["info"]["init_bpm"] = json!(0);
    variants[3]["bpm_events"] = json!([{"y":240,"bpm":-1}]);
    variants[4]["info"].as_object_mut().unwrap().remove("title");
    variants.iter().for_each(|data| {
        with_chart(data, |path| {
            assert!(read_meta(path).is_err());
            assert!(load(path, 0.0).is_err());
        })
    });
}

#[test]
fn gameplay_asset_and_layer_validation_is_deferred_until_selection() {
    let mut data = base();
    data["sound_channels"] = json!([{"name":"../outside.wav","notes":[]}]);
    with_chart(&data, |path| {
        assert!(read_meta(path).is_ok());
        assert!(load(path, 0.0)
            .err()
            .unwrap()
            .to_string()
            .contains("Invalid asset path"));
    });
    data["sound_channels"] = json!([{"name":"missing.wav","notes":[
        {"x":1,"y":0}, {"x":1,"y":0,"l":240}
    ]}]);
    with_chart(&data, |path| {
        assert!(read_meta(path).is_ok());
        assert!(load(path, 0.0)
            .err()
            .unwrap()
            .to_string()
            .contains("unequal lengths"));
    });
}
