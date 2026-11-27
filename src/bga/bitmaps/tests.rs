use super::*;
use image::Rgba;

#[test]
fn scans_each_referenced_playable_switch_pattern_once() {
    let chart = crate::bms::load_bytes(
        std::path::Path::new("chart.bms"),
        b"#BPM 120\n\
          #SWBGA01 100:500:11:0:255,0,0,0 030304\n\
          #SWBGA02 100:500:12:0:255,0,0,0 05\n\
          #SWBGA03 100:500:17:0:255,0,0,0 06\n\
          #SWBGA04 100:500:11:0:255,0,0,0 07\n\
          #001A5:01020103020105\n",
        0.0,
        0,
    )
    .unwrap();

    let frames: Vec<_> = switch_frame_ids(&chart.bga_extensions).collect();
    assert_eq!(frames, [3, 3, 4, 5]);
}

#[test]
fn loads_direct_bitmaps_and_switch_frames_with_crop_sources() {
    let _lock = image_file::load_calls_test_lock();
    let dir =
        std::env::temp_dir().join(format!("tapline-bga-switch-bitmaps-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let color = Rgba([20, 40, 60, 255]);
    for name in ["direct.png", "frame.png", "source.png"] {
        RgbaImage::from_pixel(SIZE, SIZE, color)
            .save(dir.join(name))
            .unwrap();
    }
    let chart = crate::bms::load_bytes(
        &dir.join("chart.bms"),
        b"#BPM 120\n\
          #BMP01 direct.png\n#BMP02 frame.png\n#BMP04 source.png\n\
          #BGA03 04 0 0 64 64 0 0\n\
          #SWBGA01 100:500:11:0:255,0,0,0 02\n\
          #SWBGA02 100:500:12:0:255,0,0,0 03\n\
          #00104:01\n#001A5:010201\n",
        0.0,
        0,
    )
    .unwrap();
    let bitmaps = load(&chart.bmp_paths, &chart.bga, &chart.bga_extensions);
    std::fs::remove_dir_all(dir).unwrap();

    assert_eq!(bitmaps.len(), 3);
    for id in [1, 2, 3] {
        assert_eq!(*bitmaps[&id].get_pixel(0, 0), color);
    }
}
