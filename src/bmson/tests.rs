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
            assert_eq!(chart.lanes.len(), chart.lane_count);
            chart.lanes.iter().enumerate().for_each(|(index, lane)| {
                assert_eq!(lane.role, crate::chart::LaneRole::Key);
                assert_eq!(
                    lane.player,
                    if chart.lane_count == 14 && index >= 7 {
                        crate::chart::Player::Two
                    } else {
                        crate::chart::Player::One
                    }
                );
            });
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
fn channel_pulse_groups_preserve_slices_and_events() {
    let mut data = base();
    data["info"]["init_bpm"] = json!(125);
    data["bpm_events"] = json!([{"y":240,"bpm":250}]);
    data["stop_events"] = json!([{"y":240,"duration":120}]);
    data["sound_channels"] = json!([
        {"name":"empty.wav","notes":[]},
        {"name":"missing.wav","notes":[
            {"x":3,"y":720,"c":true},
            {"x":1,"y":120,"c":true},
            {"x":0,"y":240,"c":true},
            {"x":2,"y":120,"c":true},
            {"x":0,"y":480},
            {"x":0,"y":120,"c":true},
            {"x":0,"y":480,"c":true}
        ]}
    ]);
    with_chart(&data, |path| {
        let chart = load(path, 100.0).unwrap();
        let slices = [
            (0.0, Some(240.0)),
            (240.0, Some(360.0)),
            (0.0, Some(240.0)),
            (240.0, None),
        ];
        assert_eq!(chart.sample_slices.len(), slices.len());
        for (id, (start, duration)) in slices.into_iter().enumerate() {
            let slice = &chart.sample_slices[&(id as u32)];
            assert_eq!((slice.start_ms, slice.duration_ms), (start, duration));
        }
        let notes: Vec<_> = chart
            .notes
            .iter()
            .map(|n| (n.time_ms, n.lane, n.keysound))
            .collect();
        assert_eq!(
            notes,
            [
                (340.0, 0, Some(0)),
                (340.0, 1, Some(0)),
                (1180.0, 2, Some(3))
            ]
        );
        let bgm: Vec<_> = chart.bgm.iter().map(|e| (e.time_ms, e.keysound)).collect();
        assert_eq!(bgm, [(580.0, 1), (940.0, 2)]);
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
fn gameplay_layer_validation_is_deferred_until_selection() {
    let mut data = base();
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

#[test]
fn banner_and_bga_share_case_insensitive_asset_resolution() {
    ["COVER.PNG", "IMAGES\\COVER.PNG", ".\\IMAGES\\COVER.PNG"]
        .iter()
        .for_each(|name| {
            let mut data = base();
            data["info"]["banner_image"] = json!(name);
            data["bga"] = json!({"bga_header":[{"id":1,"name":name}],
                "bga_events":[{"y":0,"id":1}]});
            with_chart(&data, |path| {
                let dir = path.parent().unwrap();
                std::fs::create_dir(dir.join("images")).unwrap();
                let image = image::DynamicImage::new_rgb8(2, 2);
                image.save(dir.join("cover.png")).unwrap();
                image.save(dir.join("images/cover.png")).unwrap();
                let chart = load(path, 0.0).unwrap();
                assert!(image::open(&chart.bmp_paths[&1]).is_ok());
                assert_eq!(chart.bga[0].bmp_id, 1);
                assert_eq!(chart.bga[0].layer, BgaLayer::Base);
                let meta = read_meta(path).unwrap();
                let mut output = Vec::new();
                crate::artwork::draw(&mut output, dir, &meta.banner, (0, 0, 2, 1)).unwrap();
                assert!(String::from_utf8(output).unwrap().contains('▀'));
            });
        });
}

#[test]
fn audio_resolves_directory_filename_and_fallback_extension_case() {
    [
        ("SUB/KICK.WAV", "sub/kick.wav"),
        (".\\SUB\\KICK.WAV", "sub/kick.wav"),
        ("SUB/KICK.WAV", "sub/kick.OGG"),
        ("SUB/KICK", "sub/kick.MP3"),
        ("SUB/KICK.OGG", "sub/kick.WAV"),
    ]
    .iter()
    .for_each(|(name, actual)| {
        let mut data = base();
        data["sound_channels"] = json!([{"name":name,"notes":[{"x":1,"y":0}]}]);
        with_chart(&data, |path| {
            let dir = path.parent().unwrap();
            std::fs::create_dir(dir.join("sub")).unwrap();
            std::fs::write(dir.join(actual), []).unwrap();
            let chart = load(path, 0.0).unwrap();
            assert_eq!(chart.wav_paths[&0].as_ref(), dir.join(actual));
        });
    });
}

#[test]
fn distinct_pulses_in_one_channel_share_the_resolved_audio_path() {
    const PULSE_COUNT: u32 = 1024;
    let notes: Vec<_> = (0..PULSE_COUNT).map(|y| json!({"x":1,"y":y})).collect();
    let mut data = base();
    data["sound_channels"] = json!([{"name":"tone.wav","notes":notes}]);
    with_chart(&data, |path| {
        let wav = path.with_file_name("tone.wav");
        std::fs::write(&wav, []).unwrap();
        let chart = load(path, 0.0).unwrap();
        let shared = &chart.wav_paths[&0];
        assert_eq!(shared.as_ref(), wav);
        assert_eq!(chart.sample_slices.len(), PULSE_COUNT as usize);
        assert_eq!(chart.wav_paths.len(), PULSE_COUNT as usize);
        for id in chart.sample_slices.keys() {
            assert!(Arc::ptr_eq(shared, &chart.wav_paths[id]), "slice {id}");
        }
    });
}

#[test]
fn assets_preserve_exact_matches_and_fallback_order_and_omit_missing_paths() {
    let mut data = base();
    data["sound_channels"] = json!([{"name":"kick.OGG","notes":[{"x":1,"y":0}]}]);
    data["bga"] = json!({"bga_header":[{"id":1,"name":"Cover.png"}]});
    with_chart(&data, |path| {
        let dir = path.parent().unwrap();
        ["kick.OGG", "kick.wav", "kick.mp3", "Cover.png"]
            .iter()
            .for_each(|name| std::fs::write(dir.join(name), []).unwrap());
        if !dir.join("kick.ogg").exists() {
            std::fs::write(dir.join("kick.ogg"), []).unwrap();
            std::fs::write(dir.join("cover.png"), []).unwrap();
        }
        let chart = load(path, 0.0).unwrap();
        assert_eq!(chart.wav_paths[&0].as_ref(), dir.join("kick.OGG"));
        assert_eq!(chart.bmp_paths[&1], dir.join("Cover.png"));
        std::fs::remove_file(dir.join("kick.OGG")).unwrap();
        if dir.join("kick.ogg").exists() {
            std::fs::remove_file(dir.join("kick.ogg")).unwrap();
        }
        assert_eq!(
            load(path, 0.0).unwrap().wav_paths[&0].as_ref(),
            dir.join("kick.wav")
        );
        std::fs::remove_file(dir.join("kick.wav")).unwrap();
        std::fs::remove_file(dir.join("kick.mp3")).unwrap();
        std::fs::remove_file(dir.join("Cover.png")).unwrap();
        if dir.join("cover.png").exists() {
            std::fs::remove_file(dir.join("cover.png")).unwrap();
        }
        let chart = load(path, 0.0).unwrap();
        assert!(chart.wav_paths.is_empty());
        assert!(chart.bmp_paths.is_empty());
        assert_eq!(chart.notes[0].keysound, Some(0));
    });
}

#[test]
fn audio_and_bga_treat_invalid_paths_as_missing() {
    [
        "../outside.wav",
        "SUB\\..\\outside.wav",
        "/absolute.wav",
        "C:\\kick.wav",
        "bad\0.wav",
    ]
    .iter()
    .for_each(|name| {
        ["sound_channels", "bga"].iter().for_each(|kind| {
            let mut data = base();
            data[kind] = if *kind == "bga" {
                json!({"bga_header":[{"id":1,"name":name}]})
            } else {
                json!([{"name":name,"notes":[{"x":1,"y":0}]}])
            };
            with_chart(&data, |path| {
                let chart = load(path, 0.0).unwrap();
                assert!(chart.wav_paths.is_empty());
                assert!(chart.bmp_paths.is_empty());
            });
        });
    });
}
