use super::*;
use crate::chart::{BgaEvent, Chart, Note};

fn game() -> (Game, ImageBank) {
    game_with_chart(|_| {})
}

fn game_with_chart(configure: impl FnOnce(&mut Chart)) -> (Game, ImageBank) {
    let bga = [
        (0.0, BgaLayer::Base, 1),
        (100.0, BgaLayer::Base, 2),
        (100.0, BgaLayer::Overlay, 3),
        (0.0, BgaLayer::Poor, 4),
    ]
    .map(|(time_ms, layer, bmp_id)| BgaEvent {
        time_ms,
        layer,
        bmp_id,
    })
    .to_vec();
    let mut chart = Chart {
        bga,
        notes: vec![
            Note {
                time_ms: 200.0,
                ..Default::default()
            },
            Note {
                time_ms: 400.0,
                lane: 1,
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    configure(&mut chart);
    let game = Game::new(chart);
    let mut bank = ImageBank(
        [
            (1, [255, 0, 0, 255]),
            (2, [0, 255, 0, 255]),
            (3, [0, 0, 0, 255]),
            (4, [0, 0, 255, 255]),
        ]
        .map(|(id, color)| (id, Arc::new(RgbaImage::from_pixel(SIZE, SIZE, Rgba(color)))))
        .into(),
        HashMap::new(),
        RefCell::default(),
        timeline::Timeline::new(&game.chart().bga),
        effect_timeline::Timeline::new(&game.chart().bga_extensions),
        RefCell::default(),
    );
    Arc::get_mut(bank.0.get_mut(&3).unwrap())
        .unwrap()
        .put_pixel(0, 0, Rgba([255, 255, 255, 255]));
    (game, bank)
}

#[test]
fn active_bga_id_costs_logarithmic_work_for_large_charts() {
    for count in [1_usize, 100, 100_000] {
        let events: Vec<BgaEvent> = (0..count)
            .map(|index| BgaEvent {
                time_ms: index as f64 * 1000.0,
                layer: BgaLayer::Base,
                bmp_id: index as u32,
            })
            .collect();
        let bank = ImageBank::load(&HashMap::new(), &events, &Extensions::default());
        for _ in 0..2 {
            bank.3.reset_comparisons();
            assert_eq!(bank.active_bga_id(BgaLayer::Base, 0.0), Some(0));
            assert!(bank.3.comparisons() <= count.ilog2() as usize + 2);
        }
        bank.3.reset_comparisons();
        assert_eq!(bank.active_bga_id(BgaLayer::Overlay, 0.0), None);
        assert_eq!(bank.3.comparisons(), 0);
    }
}

#[test]
fn reuses_static_pixels_across_frames_and_repeated_bitmap_events() {
    let (game, bank) = game_with_chart(|chart| {
        chart.bga.insert(
            1,
            BgaEvent {
                time_ms: 50.0,
                layer: BgaLayer::Base,
                bmp_id: 1,
            },
        );
    });

    let pixels = bank.rendered_frame(&game, 0.0, 16).as_ptr();
    assert_eq!(bank.rendered_frame(&game, 16.0, 16).as_ptr(), pixels);
    assert_eq!(bank.rendered_frame(&game, 50.0, 16).as_ptr(), pixels);

    let resized = bank.rendered_frame(&game, 50.0, 32);
    assert_ne!(resized.as_ptr(), pixels);
    assert_eq!(resized.dimensions(), (32, 32));
    assert_eq!(resized.get_pixel(16, 16).0, [255, 0, 0, 255]);
    drop(resized);

    let changed = bank.rendered_frame(&game, 100.0, 32);
    assert_eq!(changed.get_pixel(16, 16).0, [0, 255, 0, 255]);
}

#[test]
fn switches_layers_and_keys_out_black() {
    let (game, bank) = game();
    assert_eq!(bank.frame(&game, -1.0).get_pixel(1, 1).0, [0, 0, 0, 255]);
    assert_eq!(bank.frame(&game, 0.0).get_pixel(1, 1).0, [255, 0, 0, 255]);
    let frame = bank.frame(&game, 100.0);
    assert_eq!(frame.get_pixel(1, 1).0, [0, 255, 0, 255]);
    assert_eq!(frame.get_pixel(0, 0).0, [255, 255, 255, 255]);
}

#[test]
fn overlay_keys_out_semitransparent_black_and_blends_other_colors() {
    let (game, mut bank) = game();
    let overlay = Arc::get_mut(bank.0.get_mut(&3).unwrap()).unwrap();
    overlay.put_pixel(0, 0, Rgba([0, 0, 0, 128]));
    overlay.put_pixel(1, 0, Rgba([255, 0, 0, 128]));
    let frame = bank.frame(&game, 100.0);
    assert_eq!(frame.get_pixel(0, 0).0, [0, 255, 0, 255]);
    assert_eq!(frame.get_pixel(1, 0).0, [128, 127, 0, 254]);
}

#[test]
fn poor_expires_even_when_followed_by_a_hit() {
    let (mut game, mut bank) = game();
    game.check_misses(361.0);
    game.hit(1, 400.0);
    assert_eq!(bank.frame(&game, 400.0).get_pixel(1, 1).0, [0, 0, 255, 255]);
    assert_eq!(bank.frame(&game, 860.0).get_pixel(1, 1).0, [0, 0, 255, 255]);
    assert_eq!(bank.frame(&game, 861.0).get_pixel(1, 1).0, [0, 255, 0, 255]);
    bank.0.remove(&4);
    assert_eq!(bank.frame(&game, 400.0).get_pixel(1, 1).0, [0, 255, 0, 255]);
}

#[test]
fn narrow_panels_emit_nothing_and_wide_panels_use_half_blocks() {
    let (game, bank) = game();
    let mut out = Vec::new();
    draw(&mut out, &game, &bank, 0.0, 17, 4, 16).unwrap();
    assert!(out.is_empty());
    draw(&mut out, &game, &bank, 0.0, 18, 4, 16).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert_eq!(text.matches('▀').count(), 16 * 8);
    assert_eq!(panel_width(40, 7), None);
}

#[test]
fn trailing_layer_effects_render_before_completion_with_and_without_audio() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/effects.bms");
    for (effect, expected_red) in [("#0050B:80", 128), ("#005A1:01", 0)] {
        let source = format!(
            "#BPM 120\n#WAV01 tone.wav\n#00001:01\n#00004:01\n#ARGB01 255,255,0,0\n{effect}\n#02011:00\n"
        );
        let chart = crate::bms::load_bytes(&path, source.as_bytes(), 0.0, 0).unwrap();
        let parsed_duration = chart.duration_ms;
        assert!(parsed_duration > 12500.0);
        let mut audio = crate::audio::SampleBank::new(&chart.wav_paths);
        assert_eq!(audio.sample_count(), 1);
        audio.enabled = true;
        let silent = crate::audio::SampleBank::silent();
        let mut images = ImageBank::load(&chart.bmp_paths, &chart.bga, &chart.bga_extensions);
        assert_eq!(chart.bga.len(), 1);
        images.0.insert(
            chart.bga[0].bmp_id,
            Arc::new(RgbaImage::from_pixel(SIZE, SIZE, Rgba([255, 0, 0, 255]))),
        );
        let durations = [
            (audio.chart_duration_ms(&chart, false, false, 0.0), 12500.0),
            (
                silent.chart_duration_ms(&chart, false, false, 0.0),
                parsed_duration,
            ),
        ];
        for (duration, expected_duration) in durations {
            let mut chart = chart.clone();
            chart.duration_ms = duration;
            let mut game = Game::new(chart);
            assert!(!game.advance_completion(9984.0));
            assert_eq!(images.frame(&game, 9984.0).get_pixel(0, 0)[0], 255);
            assert!(!game.advance_completion(10000.0));
            assert_eq!(
                images.frame(&game, 10000.0).get_pixel(0, 0)[0],
                expected_red
            );
            assert_eq!(duration, expected_duration);
            assert!(game.advance_completion(duration));
        }
    }
}

#[test]
fn preloaded_images_survive_deleted_files_and_bad_files_are_skipped() {
    let path = std::env::temp_dir().join(format!("tapline-bga-{}.png", std::process::id()));
    RgbaImage::from_pixel(2, 1, Rgba([255, 0, 0, 255]))
        .save(&path)
        .unwrap();
    let invalid = path.with_extension("invalid");
    std::fs::write(&invalid, b"invalid image").unwrap();
    let paths = [
        (1, path.clone()),
        (2, path.with_extension("missing")),
        (3, invalid.clone()),
    ]
    .into();
    let mut game = Game::new(Chart {
        bmp_paths: paths,
        notes: vec![Note {
            time_ms: 1000.0,
            keysound: Some(7),
            ..Default::default()
        }],
        ..Default::default()
    });
    let bank = load_bank(game.chart());
    assert_eq!(game.hit(0, 0.0), Some(7));
    std::fs::remove_file(path).unwrap();
    std::fs::remove_file(invalid).unwrap();
    assert_eq!(bank.0.len(), 1);
    assert_eq!(bank.0[&1].dimensions(), (SIZE, SIZE));
    assert_eq!(bank.0[&1].get_pixel(0, 0).0[3], 0);
    assert_eq!(bank.0[&1].get_pixel(32, 32).0, [255, 0, 0, 255]);
}

#[test]
fn bmson_images_are_loaded_and_rendered_through_presentation() {
    let dir = std::env::temp_dir().join(format!("tapline-bga-bmson-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    RgbaImage::from_pixel(2, 2, Rgba([12, 34, 56, 255]))
        .save(dir.join("image.png"))
        .unwrap();
    let path = dir.join("chart.bmson");
    std::fs::write(
        &path,
        r#"{"version":"1.0.0","info":{"title":"BGA","artist":"Test","init_bpm":120},
        "sound_channels":[],"bga":{"bga_header":[{"id":42,"name":"image.png"}],
        "bga_events":[{"y":0,"id":42}]}}"#,
    )
    .unwrap();
    let game = Game::new(crate::bmson::load(&path, 1000.0).unwrap());
    let presentation = Presentation::new(game.chart(), true);
    let bank = presentation.bank;

    let mut deferred = Presentation::new(game.chart(), false);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while deferred.bank.0.is_empty() {
        deferred.prepare_deferred(game.chart(), true, Presentation::load_assets);
        assert!(std::time::Instant::now() < deadline, "BGA worker timed out");
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    std::fs::remove_dir_all(dir).unwrap();
    assert_eq!(
        deferred.bank.frame(&game, 1000.0),
        bank.frame(&game, 1000.0)
    );
    assert_eq!(bank.frame(&game, 999.0).get_pixel(0, 0).0, [0, 0, 0, 255]);
    assert_eq!(
        bank.frame(&game, 1000.0).get_pixel(0, 0).0,
        [12, 34, 56, 255]
    );
    let mut out = Vec::new();
    draw(&mut out, &game, &bank, 1000.0, 18, 4, 16).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert_eq!(text.matches('▀').count(), 16 * 8);
}

#[test]
fn overlay_transparency_follows_chart_format() {
    let dir = std::env::temp_dir().join(format!("tapline-bga-alpha-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let white = [255, 255, 255, 255];
    let black = [0, 0, 0, 255];
    RgbaImage::from_pixel(SIZE, SIZE, Rgba(white))
        .save(dir.join("base.png"))
        .unwrap();
    let mut overlay = RgbaImage::from_pixel(SIZE, SIZE, Rgba(black));
    overlay.put_pixel(1, 0, Rgba([255, 0, 0, 0]));
    overlay.put_pixel(2, 0, Rgba([255, 0, 0, 255]));
    overlay.save(dir.join("overlay.png")).unwrap();

    let bmson = crate::bmson::load_bytes(
        &dir.join("chart.bmson"),
        br#"{"version":"1.0.0","info":{"title":"BGA","artist":"Test","init_bpm":120},
        "sound_channels":[],"bga":{"bga_header":[{"id":1,"name":"base.png"},
        {"id":2,"name":"overlay.png"}],"bga_events":[{"y":0,"id":1}],
        "layer_events":[{"y":0,"id":2}]}}"#,
        0.0,
    )
    .unwrap();
    let path = dir.join("chart.bms");
    std::fs::write(
        &path,
        "#BPM 120\n#BMP01 base.png\n#BMP02 overlay.png\n#00004:01\n#00007:02\n",
    )
    .unwrap();
    let bms = crate::bms::load(&path, 0.0).unwrap();
    let frames = [bmson, bms].map(|chart| load_bank(&chart).frame(&Game::new(chart), 0.0));
    std::fs::remove_dir_all(dir).unwrap();

    let [bmson, bms] = frames;
    assert_eq!(bmson.get_pixel(0, 0).0, black);
    assert_eq!(bms.get_pixel(0, 0).0, white);
    for frame in [bmson, bms] {
        assert_eq!(frame.get_pixel(1, 0).0, white);
        assert_eq!(frame.get_pixel(2, 0).0, [255, 0, 0, 255]);
    }
}

#[test]
fn extended_layers_opacity_and_poor_modes() {
    let (mut game, mut bank) = game_with_chart(|chart| {
        chart.bga.push(BgaEvent {
            time_ms: 0.0,
            layer: BgaLayer::Overlay2,
            bmp_id: 4,
        });
        chart.bga_extensions.events.push(EffectEvent {
            time_ms: 50.0,
            effect: Effect::Opacity(BgaLayer::Overlay2, 128),
        });
    });
    assert_eq!(bank.frame(&game, 0.0).get_pixel(1, 1).0, [0, 0, 255, 255]);
    let pixel = bank.frame(&game, 50.0).get_pixel(1, 1).0;
    assert_eq!(pixel[..3], [127, 0, 128]);
    game.flash.last_miss_at = Some(50.0);
    assert_eq!(bank.frame(&game, 50.0).get_pixel(1, 1).0, [0, 0, 255, 255]);
    let mut chart = game.chart().clone();
    chart.bga_extensions.poor_mode = 2;
    let mut normal = Game::new(chart.clone());
    normal.flash.last_miss_at = Some(50.0);
    assert_eq!(bank.frame(&normal, 50.0).get_pixel(1, 1).0, pixel);
    chart.bga_extensions.poor_mode = 1;
    let mut game = Game::new(chart);
    game.flash.last_miss_at = Some(50.0);
    bank.0.insert(
        4,
        Arc::new(RgbaImage::from_pixel(SIZE, SIZE, Rgba([0, 0, 0, 255]))),
    );
    assert_eq!(bank.frame(&game, 50.0).get_pixel(1, 1).0, [255, 0, 0, 255]);
}

#[test]
fn ten_key_double_play_renders_p2_key_bound_animations() {
    let dir = std::env::temp_dir().join(format!("tapline-bga-dp-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let color = [20, 40, 60, 255];
    RgbaImage::from_pixel(256, 256, Rgba(color))
        .save(dir.join("image.png"))
        .unwrap();
    let path = dir.join("chart.bms");
    for key in 1..=5 {
        std::fs::write(
            &path,
            format!("#BPM 120\n#PLAYER 3\n#BMP01 image.png\n#00015:01\n#00025:01\n#SWBGA01 100:300:2{key}:0:255,255,255,255 01\n#000A5:01\n"),
        )
        .unwrap();
        let chart = crate::bms::load(&path, 0.0).unwrap();
        assert_eq!(chart.lane_count, 10);
        let mut game = Game::new(chart);
        let bank = load_bank(game.chart());
        game.hit(key - 1, 0.0);
        assert!(game.bga_switch.active.is_none());
        assert_ne!(bank.frame(&game, 0.0).get_pixel(0, 0).0, color);
        game.hit(key + 4, 0.0);
        assert_eq!(game.bga_switch.active, Some((1, key + 4, 0.0)));
        assert_eq!(bank.frame(&game, 0.0).get_pixel(0, 0).0, color);
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn parses_extensions_and_renders_key_bound_animation() {
    let dir = std::env::temp_dir().join(format!("tapline-bga-ext-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    RgbaImage::from_pixel(256, 256, Rgba([20, 40, 60, 255]))
        .save(dir.join("image.png"))
        .unwrap();
    let path = dir.join("chart.bms");
    std::fs::write(&path, "#BPM 120\n#DEFEXRANK 50\n#EXRANK01 200\n#001A0:01\n#BMP00 image.png\n#BMP01 image.png\n#BGA02 01 0 0 128 128 128 128\n#ARGB01 255,20,40,60\n#SWBGA01 100:300:11:0:255,255,255,255 0002\n#POORBGA 1\n#00004:01\n#0000A:02\n#0000B:80\n#0000C:40\n#0000D:20\n#0000E:10\n#001A1:01\n#000A5:01\n").unwrap();
    let chart = crate::bms::load(&path, 500.0).unwrap();
    assert_eq!(chart.judgment_scale(500.0), 0.5);
    assert_eq!(chart.judgment_scale(2500.0), 2.0);
    assert_eq!(chart.bga_extensions.events.len(), 6);
    assert_eq!(chart.bga_extensions.poor_mode, 1);
    assert_eq!(chart.bga_extensions.argb[&1], [255, 20, 40, 60]);
    assert!(chart
        .bga
        .iter()
        .any(|e| e.layer == BgaLayer::Overlay2 && e.time_ms == 500.0));
    let mut game = Game::new(chart);
    let bank = load_bank(game.chart());
    assert_eq!(bank.0[&2].get_pixel(0, 0).0, [0, 0, 0, 0]);
    assert_eq!(bank.0[&2].get_pixel(63, 63).0, [20, 40, 60, 255]);
    game.hit(0, 499.0);
    assert!(game.bga_switch.active.is_none());
    game.hit(0, 500.0);
    assert_eq!(
        bank.frame(&game, 500.0).get_pixel(0, 0).0,
        [20, 40, 60, 255]
    );
    assert_ne!(
        bank.frame(&game, 600.0).get_pixel(0, 0).0,
        [20, 40, 60, 255]
    );
    assert_eq!(bank.frame(&game, 700.0), bank.frame(&game, 799.0));
    let mut chart = game.chart().clone();
    let switch = chart.bga_extensions.switches.get_mut(&1).unwrap();
    switch.duration_ms = 0;
    switch.looping = true;
    let mut game = Game::new(chart);
    game.hit(0, 500.0);
    assert_eq!(
        bank.frame(&game, 700.0).get_pixel(0, 0).0,
        [20, 40, 60, 255]
    );
    game.release(0, 701.0);
    assert_ne!(
        bank.frame(&game, 701.0).get_pixel(0, 0).0,
        [20, 40, 60, 255]
    );
    assert_eq!(bank.frame(&game, 2500.0).get_pixel(0, 0).0, [0, 0, 0, 255]);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn argb_and_opacity_update_the_same_layer_state() {
    let (game, mut bank) = game_with_chart(|chart| {
        chart.bga_extensions.argb.insert(1, [128, 0, 0, 255]);
        chart.bga_extensions.events = vec![
            EffectEvent {
                time_ms: 0.0,
                effect: Effect::Color(BgaLayer::Base, 1),
            },
            EffectEvent {
                time_ms: 50.0,
                effect: Effect::Opacity(BgaLayer::Base, 255),
            },
        ];
    });
    assert_eq!(bank.frame(&game, 0.0).get_pixel(1, 1).0[..3], [128, 0, 0]);
    assert_eq!(bank.frame(&game, 50.0).get_pixel(1, 1).0, [255, 0, 0, 255]);
    let mut extensions = game.chart().bga_extensions.clone();
    extensions.argb.insert(1, [255, 255, 0, 0]);
    bank.4 = effect_timeline::Timeline::new(&extensions);
    assert_eq!(bank.frame(&game, 50.0).get_pixel(1, 1).0, [0, 0, 0, 255]);
}

#[test]
fn held_video_composite_is_reused_until_layers_style_or_width_change() {
    let (game, mut bank) = game_with_chart(|chart| {
        chart.bga_extensions.events.push(EffectEvent {
            time_ms: 10.0,
            effect: Effect::Opacity(BgaLayer::Base, 128),
        });
    });
    bank.0.remove(&1);
    let playback = &mut bank.2.get_mut()[timeline::slot(BgaLayer::Base)];
    playback.event = Some(0);
    playback.video = Some(video::Video::test_frame([255, 0, 0, 255]));

    let render = |time, width| {
        let frame = bank.rendered_frame(&game, time, width);
        assert_eq!(frame.dimensions(), (width, width));
        (frame.as_raw().as_ptr(), frame.get_pixel(0, 0).0)
    };
    let (initial, color) = render(0.0, 16);
    assert_eq!(color, [255, 0, 0, 255]);
    assert_eq!(render(5.0, 16), (initial, color));

    let (styled, color) = render(10.0, 16);
    assert_ne!(styled, initial);
    assert_eq!(color[..3], [128, 0, 0]);
    let (resized, color) = render(15.0, SIZE);
    assert_ne!(resized, styled);
    assert_eq!(render(20.0, SIZE), (resized, color));
    assert_eq!(render(100.0, SIZE).1, [255, 255, 255, 255]);
}

#[test]
fn bms_videos_decode_at_event_time_and_restart_for_repeated_ids() {
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    if Command::new("ffmpeg")
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_err()
    {
        eprintln!("Skipping video integration test: ffmpeg is not installed");
        return;
    }
    let dir = std::env::temp_dir().join(format!("tapline-video-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let raw = dir.join("frames.rgb");
    let bytes: Vec<u8> = [[255, 0, 0], [0, 255, 0]]
        .into_iter()
        .flat_map(|color| std::iter::repeat_n(color, 64 * 64).flatten())
        .collect();
    std::fs::write(&raw, bytes).unwrap();
    for (extension, codec) in [("mp4", "mpeg4"), ("webm", "libvpx-vp9")] {
        let video = dir.join(format!("video.{extension}"));
        assert!(Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-y",
                "-f",
                "rawvideo",
                "-pixel_format",
                "rgb24",
                "-video_size",
                "64x64",
                "-framerate",
                "2",
                "-i"
            ])
            .arg(&raw)
            .args(["-c:v", codec, "-pix_fmt", "yuv420p"])
            .arg(&video)
            .status()
            .unwrap()
            .success());
        let chart = dir.join("chart.bms");
        std::fs::write(
            &chart,
            format!("#TITLE Video\n#BPM 120\n#BMP01 video.{extension}\n#00004:0101\n#00111:01\n"),
        )
        .unwrap();
        let mut fixture = crate::bms::load(&chart, 1000.0).unwrap();
        let bank = load_bank(&fixture);
        // The compositor must still play both events from the bank.
        fixture.bga.truncate(1);
        let game = Game::new(fixture);
        assert_eq!(bank.frame(&game, 999.0).get_pixel(32, 32).0, [0, 0, 0, 255]);
        let wait_for = |time, channel: usize| {
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                let frame = bank.frame(&game, time);
                if frame.get_pixel(32, 32)[channel] > 240 {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "Video frame did not arrive at {time} ms"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
        };
        wait_for(1000.0, 0);
        wait_for(1500.0, 1);
        wait_for(1999.0, 1);
        wait_for(2000.0, 0);
        for layer in [
            BgaLayer::Base,
            BgaLayer::Poor,
            BgaLayer::Overlay,
            BgaLayer::Overlay2,
        ] {
            let mut fixture = crate::bms::load(&chart, 1000.0).unwrap();
            fixture.bga.truncate(1);
            fixture.bga[0].layer = layer;
            let mut delayed_bank = load_bank(&fixture);
            let mut delayed = Game::new(fixture);
            delayed.flash.last_miss_at = Some(1750.0);
            if layer == BgaLayer::Base {
                let deadline = Instant::now() + Duration::from_secs(10);
                loop {
                    if delayed_bank.frame(&delayed, 1000.0).get_pixel(32, 32)[0] > 240 {
                        break;
                    }
                    assert!(Instant::now() < deadline);
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
            let mut hidden = Vec::new();
            draw(&mut hidden, &delayed, &delayed_bank, 1000.0, 17, 4, 16).unwrap();
            assert!(hidden.is_empty());
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                let mut output = Vec::new();
                draw(&mut output, &delayed, &delayed_bank, 1750.0, 18, 4, 16).unwrap();
                let pixel = delayed_bank.frame(&delayed, 1750.0).get_pixel(32, 32).0;
                if layer == BgaLayer::Base {
                    assert!(pixel[0] > 240 || pixel[1] > 240, "Video frame disappeared");
                }
                if pixel[1] > 240 {
                    break;
                }
                assert!(Instant::now() < deadline, "Delayed video did not catch up");
                std::thread::sleep(Duration::from_millis(5));
            }
            let mut extensions = delayed.chart().bga_extensions.clone();
            extensions.events.push(EffectEvent {
                time_ms: 1750.0,
                effect: Effect::Opacity(layer, 128),
            });
            delayed_bank.4 = effect_timeline::Timeline::new(&extensions);
            let pixel = delayed_bank.frame(&delayed, 1750.0).get_pixel(32, 32).0;
            assert!((120..=128).contains(&pixel[1]));
            extensions.events.clear();
            delayed_bank.4 = effect_timeline::Timeline::new(&extensions);
            if layer == BgaLayer::Poor {
                let mut fixture = delayed.chart().clone();
                fixture.bga_extensions.poor_mode = 1;
                let mut delayed = Game::new(fixture.clone());
                delayed.flash.last_miss_at = Some(1750.0);
                assert!(delayed_bank.frame(&delayed, 1750.0).get_pixel(32, 32)[1] > 240);
                fixture.bga_extensions.poor_mode = 2;
                let mut delayed = Game::new(fixture);
                delayed.flash.last_miss_at = Some(1750.0);
                assert_eq!(
                    delayed_bank.frame(&delayed, 1750.0).get_pixel(32, 32).0,
                    [0, 0, 0, 255]
                );
            }
            if layer == BgaLayer::Base {
                let mut hidden = Vec::new();
                draw(&mut hidden, &delayed, &delayed_bank, 1760.0, 17, 4, 16).unwrap();
                let deadline = Instant::now() + Duration::from_secs(10);
                loop {
                    draw(&mut hidden, &delayed, &delayed_bank, 10000.0, 18, 4, 16).unwrap();
                    let playback = &delayed_bank.2.borrow()[timeline::slot(layer)];
                    if playback
                        .video
                        .as_ref()
                        .is_some_and(video::Video::reached_eof)
                    {
                        break;
                    }
                    assert!(Instant::now() < deadline, "Video decoder did not reach EOF");
                    std::thread::sleep(Duration::from_millis(5));
                }
                let held = delayed_bank.rendered_frame(&delayed, 10001.0, 16);
                assert!(held.get_pixel(8, 8)[1] > 240);
                let pixels = held.as_raw().as_ptr();
                drop(held);
                assert_eq!(
                    delayed_bank
                        .rendered_frame(&delayed, 20000.0, 16)
                        .as_raw()
                        .as_ptr(),
                    pixels
                );
            }
        }
        let mut output = Vec::new();
        draw(&mut output, &game, &bank, 2000.0, 18, 4, 16).unwrap();
        assert!(String::from_utf8(output).unwrap().contains('▀'));
    }
    std::fs::remove_dir_all(dir).unwrap();
}

pub(super) fn load_bank(chart: &Chart) -> ImageBank {
    Presentation::load_assets(chart)
}

#[test]
fn headless_judgments_and_hidden_presentation_never_load_images() {
    image_file::LOAD_CALLS.set(0);
    let mut chart = Chart {
        bmp_paths: [(1, PathBuf::from("unused.png"))].into(),
        notes: vec![
            Note {
                time_ms: 100.0,
                ..Default::default()
            },
            Note {
                time_ms: 200.0,
                ..Default::default()
            },
        ],
        duration_ms: 300.0,
        ..Default::default()
    };
    chart.bga_extensions.crops.insert(
        2,
        Crop {
            image: 1,
            rect: [0, 0, 128, 128, 0, 0],
        },
    );
    let mut game = Game::new(chart);
    game.hit(0, 100.0);
    game.check_misses(400.0);
    assert!(game.advance_completion(1000.0));
    assert_eq!((game.perfect, game.miss), (1, 1));
    assert_eq!(image_file::LOAD_CALLS.get(), 0);

    let mut presentation = Presentation::new(game.chart(), false);
    for (cols, rows) in [(64, 24), (120, 16)] {
        let visible = crate::render::bga_panel_visible(cols, rows, game.chart().lane_count);
        assert!(!visible);
        assert_no_deferred_load(&mut presentation, game.chart(), visible);
    }
    assert_eq!(image_file::LOAD_CALLS.get(), 0);

    let visible = crate::render::bga_panel_visible(65, 24, game.chart().lane_count);
    assert!(visible);
    let mut presentation = Presentation::new(game.chart(), visible);
    assert_eq!(
        image_file::LOAD_CALLS.get(),
        1,
        "ordinary image and crop share a load attempt before playback"
    );
    assert_no_deferred_load(&mut presentation, game.chart(), visible);
}

fn assert_no_deferred_load(presentation: &mut Presentation, chart: &Chart, visible: bool) {
    use std::sync::mpsc::{channel, TryRecvError};

    let (started, attempts) = channel();
    presentation.prepare_deferred(chart, visible, move |chart| {
        started.send(()).unwrap();
        Presentation::load_assets(chart)
    });
    assert_eq!(attempts.try_recv(), Err(TryRecvError::Disconnected));
}
