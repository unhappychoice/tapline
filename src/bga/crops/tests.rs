use super::*;
use crate::{
    bga::{image_file::LOAD_CALLS, ChartImages, Effect, EffectEvent, Presentation, Switch},
    chart::{BgaEvent, BgaLayer},
};
use image::Rgba;
use std::sync::Arc;

#[test]
fn prepares_only_crops_with_two_digit_bms_ids() {
    for (base, suffix, id) in [(36, "zZ", 1295), (62, "zz", 3843)] {
        let text = format!(
            "#BASE {base}\n#BPM 120\n#00111:01\n#00104:{suffix}\n\
             #bga{suffix} 01 0 0 128 128 0 0\n\
             #BGA100 01 0 0 128 128 0 0\n\
             #BGA0{suffix} 01 0 0 128 128 128 128\n\
             #BGA1 01 0 0 128 128 0 0\n\
             #BGA+2 01 0 0 128 128 0 0\n\
             #BGAé 01 0 0 128 128 0 0\n"
        );
        let mut chart =
            crate::bms::load_bytes(Path::new("crop.bms"), text.as_bytes(), 0.0, 0).unwrap();
        chart.bmp_paths.insert(1, "sheet.png".into());
        let color = Rgba([255, 0, 0, 255]);
        let images = prepare(&chart, |_| Some(RgbaImage::from_pixel(128, 128, color)));

        assert_eq!(images.len(), 1, "base {base}");
        assert_eq!(chart.bga_extensions.crops.len(), 1, "base {base}");
        assert_eq!(chart.bga[0].bmp_id, id);
        assert_eq!(
            images[&id],
            RgbaImage::from_fn(SIZE, SIZE, |x, y| {
                if x < SIZE / 2 && y < SIZE / 2 {
                    color
                } else {
                    Rgba([0, 0, 0, 0])
                }
            })
        );
    }
}

#[test]
fn presentation_decodes_only_reachable_sources_and_preserves_bitmap_and_crop_pixels() {
    let _lock = crate::bga::image_file::load_calls_test_lock();
    let dir = std::env::temp_dir().join(format!("tapline-crops-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("sheet.png");
    let other = dir.join("other.png");
    let red = Rgba([255, 0, 0, 255]);
    let blue = Rgba([0, 0, 255, 128]);
    RgbaImage::from_fn(256, 128, |x, _| if x < 128 { red } else { blue })
        .save(&path)
        .unwrap();
    RgbaImage::from_pixel(128, 128, blue).save(&other).unwrap();
    let unused = dir.join("unused.png");
    RgbaImage::from_pixel(128, 128, red).save(&unused).unwrap();
    let mut chart = Chart {
        bmp_paths: [
            (1, path.clone()),
            (2, path.clone()),
            (3, path.clone()),
            (4, other),
            (5, unused),
            (14, path),
        ]
        .into(),
        bga: [1, 2, 3, 10, 11, 12, 14]
            .map(|bmp_id| BgaEvent {
                time_ms: bmp_id as f64,
                layer: BgaLayer::Base,
                bmp_id,
            })
            .into(),
        ..Default::default()
    };
    chart.bga_extensions.crops = [
        (10, 1, [0, 0, 128, 128, 0, 0]),
        (11, 1, [128, 0, 256, 128, 128, 128]),
        (12, 2, [128, 0, 256, 128, 0, 128]),
        (13, 4, [-8, -8, 256, 256, -64, -64]),
        (3, 1, [0, 0, 128, 128, 0, 0]),
        (14, 99, [0, 0, 128, 128, 0, 0]),
        (15, 5, [0, 0, 128, 128, 0, 0]),
    ]
    .map(|(id, image, rect)| (id, Crop { image, rect }))
    .into();
    chart.bga_extensions.switches = [(1, Some(0), 13), (2, Some(0), 5), (3, None, 5)]
        .map(|(id, lane, bmp_id)| {
            (
                id,
                Switch {
                    frame_ms: 100,
                    duration_ms: 100,
                    channel: "11".into(),
                    lane,
                    looping: false,
                    argb: [255; 4],
                    pattern: vec![bmp_id],
                },
            )
        })
        .into();
    chart.bga_extensions.events = [1, 3]
        .map(|id| EffectEvent {
            time_ms: id as f64,
            effect: Effect::Switch(id),
        })
        .into();
    LOAD_CALLS.store(0, std::sync::atomic::Ordering::SeqCst);
    let presentation = Presentation::with_cached_images(&mut ChartImages::new(&chart), true);
    std::fs::remove_dir_all(dir).unwrap();

    assert_eq!(LOAD_CALLS.load(std::sync::atomic::Ordering::SeqCst), 2);
    let images = &presentation.bank.0;
    assert_eq!(images.len(), 8);
    assert!(!images.contains_key(&4));
    assert!(!images.contains_key(&5));
    assert!(Arc::ptr_eq(&images[&1], &images[&2]));
    let transparent = Rgba([0; 4]);
    let mut blended_blue = transparent;
    image::Pixel::blend(&mut blended_blue, &blue);
    let expected_bitmap = RgbaImage::from_fn(SIZE, SIZE, |x, y| match (x, y) {
        (0..32, 16..48) => red,
        (32..64, 16..48) => blended_blue,
        _ => transparent,
    });
    assert_eq!(images[&1].as_ref(), &expected_bitmap);
    assert_eq!(images[&2].as_ref(), &expected_bitmap);
    assert_eq!(images[&14].as_ref(), &expected_bitmap);

    [3, 10, 11, 12, 13].into_iter().for_each(|id| {
        let image = &images[&id];
        assert_eq!(image.dimensions(), (SIZE, SIZE));
        image.enumerate_pixels().for_each(|(x, y, pixel)| {
            let expected = match id {
                3 | 10 if x < 32 && y < 32 => red,
                11 if x >= 32 && y >= 32 => blue,
                12 if x < 32 && y >= 32 => blue,
                13 if x < 16 && y < 16 => blue,
                _ => Rgba([0, 0, 0, 0]),
            };
            assert_eq!(*pixel, expected, "crop {id} at ({x}, {y})");
        });
    });
}

#[test]
fn missing_and_invalid_sources_are_attempted_once_and_preserve_existing_images() {
    let path =
        std::env::temp_dir().join(format!("tapline-invalid-crop-{}.png", std::process::id()));
    std::fs::write(&path, b"invalid image").unwrap();
    let missing = path.with_extension("missing");
    let mut chart = Chart {
        bmp_paths: [(1, path.clone()), (2, path.clone()), (3, missing.clone())].into(),
        ..Default::default()
    };
    chart.bga_extensions.crops = [(10, 1), (11, 2), (12, 3), (13, 3), (14, 99)]
        .map(|(id, image)| {
            (
                id,
                Crop {
                    image,
                    rect: [0, 0, 256, 256, 0, 0],
                },
            )
        })
        .into();
    let calls = std::sync::Mutex::new(HashMap::new());
    assert!(prepare(&chart, |path| {
        *calls.lock().unwrap().entry(path.to_path_buf()).or_insert(0) += 1;
        image::open(path).ok().map(|image| image.to_rgba8())
    })
    .is_empty());
    assert_eq!(
        calls.into_inner().unwrap(),
        [(path.clone(), 1), (missing, 1)].into()
    );
    let mut bank = super::super::ImageBank::default();
    let original = RgbaImage::from_pixel(SIZE, SIZE, Rgba([12, 34, 56, 255]));
    Arc::make_mut(&mut bank.0).insert(10, original.clone().into());
    bank.prepare_crops(&chart);
    assert_eq!(bank.0.len(), 1);
    assert_eq!(bank.0[&10].as_ref(), &original);
    std::fs::remove_file(path).unwrap();
}
