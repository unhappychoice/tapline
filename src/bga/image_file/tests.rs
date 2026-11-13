use super::*;
use crate::{bga::Crop, chart::Chart, game::Game};
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
        assert!(game.bga.0.is_empty());
    });
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
        assert_eq!(game.bga.0.len(), 2);
        assert_eq!(*game.bga.0[&4].get_pixel(0, 0), color);
        assert_eq!(*game.bga.0[&14].get_pixel(63, 63), color);
        assert_eq!(game.bga.0[&14].get_pixel(0, 0).0, [0; 4]);
    });
}

#[test]
fn replacement_fifo_and_device_are_rejected_after_open() {
    with_fifo("replacement", |fifo| {
        let path = fifo.with_file_name("regular.png");
        std::fs::write(&path, b"regular").unwrap();
        assert!(path.metadata().unwrap().is_file());
        std::fs::rename(fifo, &path).unwrap();
        assert!(load_verified(&path).is_none());
        assert!(load_verified(Path::new("/dev/zero")).is_none());
    });
}
