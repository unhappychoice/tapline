use crate::assets::AssetIndex;
use crossterm::{
    cursor, queue,
    style::{Color, Print, ResetColor, SetBackgroundColor, SetForegroundColor},
};
use image::{DynamicImage, ImageReader};
use std::{
    cell::RefCell,
    fs::OpenOptions,
    io::{BufReader, Write},
    path::{Path, PathBuf},
};

thread_local! {
    static CACHE: RefCell<Option<(PathBuf, Option<DynamicImage>)>> = const { RefCell::new(None) };
}

/// Draw an image with two vertical pixels per terminal cell, preserving its aspect ratio.
/// Missing or unsupported artwork is optional and never prevents play.
pub(crate) fn draw(
    out: &mut impl Write,
    dir: &Path,
    name: &str,
    area: (u16, u16, u16, u16),
) -> std::io::Result<()> {
    if name.is_empty() || area.2 == 0 || area.3 == 0 {
        return Ok(());
    }
    let path = dir.join(name.replace('\\', "/"));
    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.as_ref().map(|entry| &entry.0) != Some(&path) {
            let image = AssetIndex::new(dir)
                .resolve_path(name, &[])
                .and_then(|path| load_image(path));
            *cache = Some((path, image));
        }
        match cache.as_ref().and_then(|entry| entry.1.as_ref()) {
            Some(image) => draw_pixels(out, image, area),
            None => Ok(()),
        }
    })
}

fn load_image(path: &Path) -> Option<DynamicImage> {
    if !path.metadata().ok()?.is_file() {
        return None;
    }
    let mut options = OpenOptions::new();
    options.read(true);
    // Avoid blocking if a regular file is replaced with a FIFO before opening.
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = options.open(path).ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    ImageReader::new(BufReader::new(file))
        .with_guessed_format()
        .ok()?
        .decode()
        .ok()
}

fn draw_pixels(
    out: &mut impl Write,
    image: &DynamicImage,
    (x, y, width, height): (u16, u16, u16, u16),
) -> std::io::Result<()> {
    let pixels = image
        .thumbnail(u32::from(width), u32::from(height) * 2)
        .to_rgba8();
    let left = x + (width - pixels.width() as u16) / 2;
    for row in (0..pixels.height()).step_by(2) {
        queue!(out, cursor::MoveTo(left, y + (row / 2) as u16))?;
        for col in 0..pixels.width() {
            let top = color(pixels.get_pixel(col, row).0);
            let bottom = if row + 1 < pixels.height() {
                color(pixels.get_pixel(col, row + 1).0)
            } else {
                Color::Black
            };
            queue!(
                out,
                SetForegroundColor(top),
                SetBackgroundColor(bottom),
                Print("▀")
            )?;
        }
    }
    queue!(out, ResetColor)
}

fn color([r, g, b, a]: [u8; 4]) -> Color {
    let blend = |value| (u16::from(value) * u16::from(a) / 255) as u8;
    Color::Rgb {
        r: blend(r),
        g: blend(g),
        b: blend(b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_pixels_as_colored_half_blocks_within_area() {
        crossterm::style::force_color_output(true);
        let image = DynamicImage::ImageRgba8(image::RgbaImage::from_fn(2, 2, |_, y| {
            if y == 0 {
                image::Rgba([255, 0, 0, 255])
            } else {
                image::Rgba([0, 0, 255, 255])
            }
        }));
        let mut out = Vec::new();
        draw_pixels(&mut out, &image, (0, 3, 4, 1)).unwrap();
        let output = String::from_utf8(out).unwrap();
        assert!(output.contains("\x1b[4;2H"));
        assert!(output.contains("38;2;255;0;0"));
        assert!(output.contains("48;2;0;0;255"));
        assert_eq!(output.matches('▀').count(), 2);
    }

    #[test]
    fn missing_invalid_and_zero_sized_artwork_are_ignored() {
        let mut out = Vec::new();
        draw(&mut out, Path::new("."), "", (0, 0, 10, 10)).unwrap();
        draw(&mut out, Path::new("."), "Cargo.toml", (0, 0, 10, 10)).unwrap();
        draw(&mut out, Path::new("."), "missing.png", (0, 0, 10, 10)).unwrap();
        draw(&mut out, Path::new("."), "Cargo.toml", (0, 0, 0, 0)).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn renders_regular_image_file() {
        let path = std::env::temp_dir().join(format!("tapline-artwork-{}.png", std::process::id()));
        DynamicImage::new_rgb8(2, 2).save(&path).unwrap();
        let mut out = Vec::new();
        draw(
            &mut out,
            path.parent().unwrap(),
            path.file_name().unwrap().to_str().unwrap(),
            (0, 0, 2, 1),
        )
        .unwrap();
        std::fs::remove_file(path).unwrap();
        assert!(String::from_utf8(out).unwrap().contains('▀'));
    }

    #[test]
    fn bga_stage_and_banner_share_case_insensitive_image_resolution() {
        let dir = std::env::temp_dir().join(format!("tapline-artwork-case-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("images")).unwrap();
        DynamicImage::new_rgb8(2, 2)
            .save(dir.join("images/cover.png"))
            .unwrap();
        DynamicImage::new_rgb8(2, 2)
            .save(dir.join("cover.png"))
            .unwrap();
        ["COVER.PNG", "IMAGES\\COVER.PNG", ".\\IMAGES\\COVER.PNG"]
            .iter()
            .for_each(|name| {
                let path = dir.join("song.bms");
                std::fs::write(
                    &path,
                    format!("#BMP01 {name}\n#STAGEFILE {name}\n#BANNER {name}\n#00104:01\n"),
                )
                .unwrap();
                let chart = crate::bms::load(&path, 0.0).unwrap();
                assert!(load_image(&chart.bmp_paths[&1]).is_some());
                let meta = crate::bms::read_meta(&path).unwrap();
                [&chart.stagefile, &meta.banner].iter().for_each(|name| {
                    let mut out = Vec::new();
                    draw(&mut out, &dir, name, (0, 0, 2, 1)).unwrap();
                    assert!(String::from_utf8(out).unwrap().contains('▀'));
                });
            });
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn artwork_prefers_exact_case_when_names_collide() {
        let dir =
            std::env::temp_dir().join(format!("tapline-artwork-exact-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Cover.png"), b"invalid image").unwrap();
        if !dir.join("cover.png").exists() {
            DynamicImage::new_rgb8(2, 2)
                .save(dir.join("cover.png"))
                .unwrap();
            let mut out = Vec::new();
            draw(&mut out, &dir, "cover.png", (0, 0, 2, 1)).unwrap();
            assert!(String::from_utf8(out).unwrap().contains('▀'));
            let mut out = Vec::new();
            draw(&mut out, &dir, "Cover.png", (0, 0, 2, 1)).unwrap();
            assert!(out.is_empty());
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn fifo_and_symlink_artwork_return_without_a_writer() {
        use std::{os::unix::fs::symlink, sync::mpsc, time::Duration};

        let dir = std::env::temp_dir().join(format!("tapline-artwork-fifo-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fifo = dir.join("banner.png");
        let name = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        symlink(&fifo, dir.join("stage.png")).unwrap();
        let (sender, receiver) = mpsc::channel();
        let artwork_dir = dir.clone();
        std::thread::spawn(move || {
            let mut out = Vec::new();
            ["banner.png", "stage.png", "."].iter().for_each(|name| {
                draw(&mut out, &artwork_dir, name, (0, 0, 10, 10)).unwrap();
            });
            sender.send(out).unwrap();
        });
        let result = receiver.recv_timeout(Duration::from_secs(2));
        std::fs::remove_dir_all(dir).unwrap();
        assert!(result
            .expect("special-file artwork must not block")
            .is_empty());
    }
}
