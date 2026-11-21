use super::*;
use crate::{
    bga::Crop,
    chart::{BgaEvent, BgaLayer, Chart},
    game::Game,
};
use std::{os::unix::fs::symlink, sync::mpsc, time::Duration};

fn with_fifo(name: &str, check: impl FnOnce(&Path) + Send + 'static) {
    let dir = std::env::temp_dir().join(format!("tapline-bga-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let fifo = dir.join("blocked.png");
    let name = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        check(&fifo);
        sender.send(()).ok();
    });
    let result = receiver.recv_timeout(Duration::from_secs(5));
    std::fs::remove_dir_all(dir).unwrap();
    result.expect("special-file BGA must not block");
}

#[test]
fn bmson_fifo_and_symlink_headers_without_events_do_not_block_initialization() {
    with_fifo("headers", |fifo| {
        let dir = fifo.parent().unwrap();
        symlink(fifo, dir.join("link.png")).unwrap();
        let path = dir.join("chart.bmson");
        std::fs::write(&path, r#"{"version":"1.0.0","info":{"title":"FIFO","artist":"Test","init_bpm":120},"sound_channels":[],"bga":{"bga_header":[{"id":1,"name":"blocked.png"},{"id":2,"name":"link.png"},{"id":3,"name":"."}]}}"#).unwrap();
        let game = Game::new(crate::bmson::load(&path, 0.0).unwrap());
        let bank = crate::bga::tests::load_bank(&game.chart);
        assert!(bank.0.is_empty());
    });
}

#[test]
fn chart_bga_skips_file_and_directory_links_to_other_directories() {
    let root = std::env::temp_dir().join(format!("tapline-bga-links-{}", std::process::id()));
    let song = root.join("song");
    std::fs::create_dir_all(song.join("images")).unwrap();
    let outside = root.join("cover.png");
    image::RgbaImage::from_pixel(1, 1, image::Rgba([12, 34, 56, 255]))
        .save(&outside)
        .unwrap();
    std::fs::copy(&outside, song.join("images/Cover.PNG")).unwrap();
    symlink(&outside, song.join("cover.png")).unwrap();
    symlink(&root, song.join("linked")).unwrap();

    let bmson = song.join("chart.bmson");
    let data = serde_json::json!({
        "version": "1.0.0",
        "info": {"title": "BGA test", "artist": "tapline", "init_bpm": 120},
        "sound_channels": [],
        "bga": {
            "bga_header": [
                {"id": 1, "name": "cover.png"},
                {"id": 2, "name": "linked/cover.png"},
                {"id": 3, "name": "IMAGES/cover.png"}
            ],
            "bga_events": [{"y": 0, "id": 1}, {"y": 240, "id": 2}, {"y": 480, "id": 3}]
        }
    });
    std::fs::write(&bmson, data.to_string()).unwrap();
    let bms = song.join("chart.bms");
    std::fs::write(&bms, "#BPM 120\n#BMP01 cover.png\n#BMP02 linked/cover.png\n#BMP03 IMAGES/cover.png\n#00104:010203\n").unwrap();

    for chart in [
        crate::bmson::load(&bmson, 0.0).unwrap(),
        crate::bms::load(&bms, 0.0).unwrap(),
    ] {
        let bank = crate::bga::tests::load_bank(&chart);
        assert_eq!(chart.bmp_paths.len(), 1);
        assert_eq!(bank.0.len(), 1);
        assert!(bank.0.contains_key(&3));
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn crops_skip_special_files_and_preserve_regular_images() {
    with_fifo("crops", |fifo| {
        let dir = fifo.parent().unwrap();
        let link = dir.join("link.png");
        symlink(fifo, &link).unwrap();
        let regular = dir.join("regular.png");
        let color = image::Rgba([12, 34, 56, 255]);
        image::RgbaImage::from_pixel(256, 256, color)
            .save(&regular)
            .unwrap();
        let mut chart = Chart {
            bmp_paths: [
                (1, fifo.to_path_buf()),
                (2, link),
                (3, dir.to_path_buf()),
                (4, regular),
            ]
            .into(),
            bga: [4, 11, 12, 13, 14]
                .map(|bmp_id| BgaEvent {
                    time_ms: bmp_id as f64,
                    layer: BgaLayer::Base,
                    bmp_id,
                })
                .into(),
            ..Default::default()
        };
        chart.bga_extensions.crops = (1..=4)
            .map(|image| {
                (
                    image + 10,
                    Crop {
                        image,
                        rect: [0, 0, 128, 128, 128, 128],
                    },
                )
            })
            .collect();
        let game = Game::new(chart);
        let bank = crate::bga::tests::load_bank(&game.chart);
        assert_eq!(bank.0.len(), 2);
        assert_eq!(*bank.0[&4].get_pixel(0, 0), color);
        assert_eq!(*bank.0[&14].get_pixel(63, 63), color);
        assert_eq!(bank.0[&14].get_pixel(0, 0).0, [0; 4]);
    });
}

#[test]
fn replacement_fifo_and_device_are_rejected_after_open() {
    with_fifo("replacement", |fifo| {
        let path = fifo.with_file_name("regular.png");
        std::fs::write(&path, b"regular").unwrap();
        assert!(path.metadata().unwrap().is_file());
        std::fs::rename(fifo, &path).unwrap();
        assert!(crate::fs_guard::open_verified(&path).is_err());
        assert!(crate::fs_guard::open_verified(Path::new("/dev/zero")).is_err());
    });
}
