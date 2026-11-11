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
