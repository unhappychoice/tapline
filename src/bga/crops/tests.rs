use super::*;
use image::Rgba;

#[test]
fn decodes_once_per_path_across_bmp_ids_and_preserves_crop_placement() {
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
    let mut chart = Chart {
        bmp_paths: [(1, path.clone()), (2, path.clone()), (3, other.clone())].into(),
        ..Default::default()
    };
    chart.bga_extensions.crops = [
        (10, 1, [0, 0, 128, 128, 0, 0]),
        (11, 1, [128, 0, 256, 128, 128, 128]),
        (12, 2, [128, 0, 256, 128, 0, 128]),
        (13, 3, [-8, -8, 256, 256, -64, -64]),
    ]
    .map(|(id, image, rect)| (id, Crop { image, rect }))
    .into();
    let mut calls = HashMap::new();
    let images = prepare(&chart, |path| {
        *calls.entry(path.to_path_buf()).or_insert(0) += 1;
        image::open(path).ok().map(|image| image.to_rgba8())
    });
    std::fs::remove_dir_all(dir).unwrap();
    assert_eq!(calls, [(path, 1), (other, 1)].into());
    assert_eq!(images.len(), 4);
    images.iter().for_each(|(&id, image)| {
        assert_eq!(image.dimensions(), (SIZE, SIZE));
        image.enumerate_pixels().for_each(|(x, y, pixel)| {
            let expected = match id {
                10 if x < 32 && y < 32 => red,
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
    let mut calls = HashMap::new();
    assert!(prepare(&chart, |path| {
        *calls.entry(path.to_path_buf()).or_insert(0) += 1;
        image::open(path).ok().map(|image| image.to_rgba8())
    })
    .is_empty());
    assert_eq!(calls, [(path.clone(), 1), (missing, 1)].into());
    let mut bank = super::super::ImageBank::default();
    let original = RgbaImage::from_pixel(SIZE, SIZE, Rgba([12, 34, 56, 255]));
    bank.0.insert(10, original.clone());
    bank.prepare_crops(&chart);
    assert_eq!(bank.0.len(), 1);
    assert_eq!(bank.0[&10], original);
    std::fs::remove_file(path).unwrap();
}
