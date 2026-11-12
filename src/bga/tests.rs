use super::*;
use crate::chart::{BgaEvent, Chart, Note};

fn game() -> Game {
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
    let mut game = Game::new(Chart {
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
    });
    game.bga = ImageBank(
        [
            (1, [255, 0, 0, 255]),
            (2, [0, 255, 0, 255]),
            (3, [0, 0, 0, 255]),
            (4, [0, 0, 255, 255]),
        ]
        .map(|(id, color)| (id, RgbaImage::from_pixel(SIZE, SIZE, Rgba(color))))
        .into(),
    );
    game.bga
        .0
        .get_mut(&3)
        .unwrap()
        .put_pixel(0, 0, Rgba([255, 255, 255, 255]));
    game
}

#[test]
fn switches_layers_and_keys_out_black() {
    let game = game();
    assert_eq!(
        game.bga.frame(&game, -1.0).get_pixel(1, 1).0,
        [0, 0, 0, 255]
    );
    assert_eq!(
        game.bga.frame(&game, 0.0).get_pixel(1, 1).0,
        [255, 0, 0, 255]
    );
    let frame = game.bga.frame(&game, 100.0);
    assert_eq!(frame.get_pixel(1, 1).0, [0, 255, 0, 255]);
    assert_eq!(frame.get_pixel(0, 0).0, [255, 255, 255, 255]);
}

#[test]
fn poor_expires_even_when_followed_by_a_hit() {
    let mut game = game();
    game.check_misses(361.0);
    game.hit(1, 400.0);
    assert_eq!(
        game.bga.frame(&game, 400.0).get_pixel(1, 1).0,
        [0, 0, 255, 255]
    );
    assert_eq!(
        game.bga.frame(&game, 860.0).get_pixel(1, 1).0,
        [0, 0, 255, 255]
    );
    assert_eq!(
        game.bga.frame(&game, 861.0).get_pixel(1, 1).0,
        [0, 255, 0, 255]
    );
    game.bga.0.remove(&4);
    assert_eq!(
        game.bga.frame(&game, 400.0).get_pixel(1, 1).0,
        [0, 255, 0, 255]
    );
}

#[test]
fn narrow_panels_emit_nothing_and_wide_panels_use_half_blocks() {
    let game = game();
    let mut out = Vec::new();
    draw(&mut out, &game, 0.0, 17, 4, 16).unwrap();
    assert!(out.is_empty());
    draw(&mut out, &game, 0.0, 18, 4, 16).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert_eq!(text.matches('▀').count(), 16 * 8);
    assert_eq!(panel_width(40, 7), None);
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
    assert_eq!(game.hit(0, 0.0), Some(7));
    let bank = game.bga;
    std::fs::remove_file(path).unwrap();
    std::fs::remove_file(invalid).unwrap();
    assert_eq!(bank.0.len(), 1);
    assert_eq!(bank.0[&1].dimensions(), (SIZE, SIZE));
    assert_eq!(bank.0[&1].get_pixel(0, 0).0[3], 0);
    assert_eq!(bank.0[&1].get_pixel(32, 32).0, [255, 0, 0, 255]);
}

#[test]
fn bmson_images_are_loaded_and_rendered_through_game() {
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
    std::fs::remove_dir_all(dir).unwrap();
    assert_eq!(
        game.bga.frame(&game, 999.0).get_pixel(0, 0).0,
        [0, 0, 0, 255]
    );
    assert_eq!(
        game.bga.frame(&game, 1000.0).get_pixel(0, 0).0,
        [12, 34, 56, 255]
    );
    let mut out = Vec::new();
    draw(&mut out, &game, 1000.0, 18, 4, 16).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert_eq!(text.matches('▀').count(), 16 * 8);
}

#[test]
fn extended_layers_opacity_and_poor_modes() {
    let mut game = game();
    game.chart.bga.push(BgaEvent {
        time_ms: 0.0,
        layer: BgaLayer::Overlay2,
        bmp_id: 4,
    });
    game.chart.bga_extensions.events.push(EffectEvent {
        time_ms: 50.0,
        effect: Effect::Opacity(BgaLayer::Overlay2, 128),
    });
    assert_eq!(
        game.bga.frame(&game, 0.0).get_pixel(1, 1).0,
        [0, 0, 255, 255]
    );
    let pixel = game.bga.frame(&game, 50.0).get_pixel(1, 1).0;
    assert_eq!(pixel[..3], [127, 0, 128]);
    game.flash.last_miss_at = Some(50.0);
    assert_eq!(
        game.bga.frame(&game, 50.0).get_pixel(1, 1).0,
        [0, 0, 255, 255]
    );
    game.chart.bga_extensions.poor_mode = 2;
    assert_eq!(game.bga.frame(&game, 50.0).get_pixel(1, 1).0, pixel);
    game.chart.bga_extensions.poor_mode = 1;
    game.bga
        .0
        .insert(4, RgbaImage::from_pixel(SIZE, SIZE, Rgba([0, 0, 0, 255])));
    assert_eq!(
        game.bga.frame(&game, 50.0).get_pixel(1, 1).0,
        [255, 0, 0, 255]
    );
}

#[test]
fn parses_extensions_and_renders_key_bound_animation() {
    let dir = std::env::temp_dir().join(format!("tapline-bga-ext-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    RgbaImage::from_pixel(256, 256, Rgba([20, 40, 60, 255]))
        .save(dir.join("image.png"))
        .unwrap();
    let path = dir.join("chart.bms");
    std::fs::write(&path, "#BPM 120\n#BMP00 image.png\n#BMP01 image.png\n#BGA02 01 0 0 128 128 128 128\n#ARGB01 255,20,40,60\n#SWBGA01 100:300:11:0:255,255,255,255 0002\n#POORBGA 1\n#00004:01\n#0000A:02\n#0000B:80\n#0000C:40\n#0000D:20\n#0000E:10\n#001A1:01\n#000A5:01\n").unwrap();
    let chart = crate::bms::load(&path, 500.0).unwrap();
    assert_eq!(chart.bga_extensions.events.len(), 6);
    assert_eq!(chart.bga_extensions.poor_mode, 1);
    assert_eq!(chart.bga_extensions.argb[&1], [255, 20, 40, 60]);
    assert!(chart
        .bga
        .iter()
        .any(|e| e.layer == BgaLayer::Overlay2 && e.time_ms == 500.0));
    let mut game = Game::new(chart);
    assert_eq!(game.bga.0[&2].get_pixel(0, 0).0, [0, 0, 0, 0]);
    assert_eq!(game.bga.0[&2].get_pixel(63, 63).0, [20, 40, 60, 255]);
    game.hit(0, 499.0);
    assert!(game.bga_switch.active.is_none());
    game.hit(0, 500.0);
    assert_eq!(
        game.bga.frame(&game, 500.0).get_pixel(0, 0).0,
        [20, 40, 60, 255]
    );
    assert_ne!(
        game.bga.frame(&game, 600.0).get_pixel(0, 0).0,
        [20, 40, 60, 255]
    );
    assert_eq!(game.bga.frame(&game, 700.0), game.bga.frame(&game, 799.0));
    game.chart
        .bga_extensions
        .switches
        .get_mut(&1)
        .unwrap()
        .duration_ms = 0;
    game.chart
        .bga_extensions
        .switches
        .get_mut(&1)
        .unwrap()
        .looping = true;
    assert_eq!(
        game.bga.frame(&game, 700.0).get_pixel(0, 0).0,
        [20, 40, 60, 255]
    );
    game.release(0, 701.0);
    assert_ne!(
        game.bga.frame(&game, 701.0).get_pixel(0, 0).0,
        [20, 40, 60, 255]
    );
    assert_eq!(
        game.bga.frame(&game, 2500.0).get_pixel(0, 0).0,
        [0, 0, 0, 255]
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn argb_and_opacity_update_the_same_layer_state() {
    let mut game = game();
    game.chart.bga_extensions.argb.insert(1, [128, 0, 0, 255]);
    game.chart.bga_extensions.events = vec![
        EffectEvent {
            time_ms: 0.0,
            effect: Effect::Color(BgaLayer::Base, 1),
        },
        EffectEvent {
            time_ms: 50.0,
            effect: Effect::Opacity(BgaLayer::Base, 255),
        },
    ];
    assert_eq!(
        game.bga.frame(&game, 0.0).get_pixel(1, 1).0[..3],
        [128, 0, 0]
    );
    assert_eq!(
        game.bga.frame(&game, 50.0).get_pixel(1, 1).0,
        [255, 0, 0, 255]
    );
    game.chart.bga_extensions.argb.insert(1, [255, 255, 0, 0]);
    assert_eq!(
        game.bga.frame(&game, 50.0).get_pixel(1, 1).0,
        [0, 0, 0, 255]
    );
}
