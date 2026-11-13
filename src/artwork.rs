use crate::assets::AssetIndex;
use crossterm::{
    cursor, queue,
    style::{Color, Print, ResetColor, SetBackgroundColor, SetForegroundColor},
};
use image::{DynamicImage, ImageReader, RgbaImage};
use std::{
    cell::RefCell,
    fs::OpenOptions,
    io::{BufReader, Write},
    path::{Path, PathBuf},
};

thread_local! {
    static CACHE: RefCell<Option<ArtworkCache>> = const { RefCell::new(None) };
    #[cfg(test)]
    static RESIZE_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
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
        if cache.as_ref().map(|entry| &entry.path) != Some(&path) {
            let image = AssetIndex::new(dir)
                .resolve_path(name, &[])
                .and_then(|path| load_image(path));
            *cache = Some(ArtworkCache {
                path,
                image,
                resized: None,
            });
        }
        match cache
            .as_mut()
            .and_then(|entry| entry.pixels(area.2, area.3))
        {
            Some(image) => draw_pixels(out, image, area),
            None => Ok(()),
        }
    })
}

struct ArtworkCache {
    path: PathBuf,
    image: Option<DynamicImage>,
    resized: Option<((u16, u16), RgbaImage)>,
}

impl ArtworkCache {
    fn pixels(&mut self, width: u16, height: u16) -> Option<&RgbaImage> {
        let image = self.image.as_ref()?;
        if self.resized.as_ref().map(|entry| entry.0) != Some((width, height)) {
            let pixels = image
                .thumbnail(u32::from(width), u32::from(height) * 2)
                .to_rgba8();
            #[cfg(test)]
            RESIZE_COUNT.with(|count| count.set(count.get() + 1));
            self.resized = Some(((width, height), pixels));
        }
        self.resized.as_ref().map(|entry| &entry.1)
    }
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
    pixels: &RgbaImage,
    (x, y, width, _height): (u16, u16, u16, u16),
) -> std::io::Result<()> {
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
    fn reuses_resized_pixels_and_refreshes_for_image_and_area_changes() {
        crossterm::style::force_color_output(true);
        CACHE.with(|cache| *cache.borrow_mut() = None);
        RESIZE_COUNT.with(|count| count.set(0));
        let dir =
            std::env::temp_dir().join(format!("tapline-artwork-cache-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let first = DynamicImage::ImageRgba8(RgbaImage::from_fn(120, 90, |_, _| {
            image::Rgba([200, 100, 50, 128])
        }));
        let second = DynamicImage::ImageRgba8(RgbaImage::from_fn(90, 120, |_, _| {
            image::Rgba([0, 255, 0, 255])
        }));
        first.save(dir.join("first.png")).unwrap();
        second.save(dir.join("second.png")).unwrap();
        let cases = [
            ("first.png", &first, (0, 0, 8, 3), 1),
            ("first.png", &first, (0, 0, 8, 3), 1),
            ("first.png", &first, (5, 7, 8, 3), 1),
            ("first.png", &first, (5, 7, 4, 3), 2),
            ("first.png", &first, (5, 7, 4, 1), 3),
            ("second.png", &second, (5, 7, 4, 1), 4),
            ("second.png", &second, (5, 7, 4, 1), 4),
            ("first.png", &first, (0, 0, 8, 3), 5),
        ];
        cases.iter().for_each(|(name, image, area, count)| {
            let mut actual = Vec::new();
            draw(&mut actual, &dir, name, *area).unwrap();
            let pixels = image
                .thumbnail(u32::from(area.2), u32::from(area.3) * 2)
                .to_rgba8();
            let mut expected = Vec::new();
            draw_pixels(&mut expected, &pixels, *area).unwrap();
            assert_eq!(actual, expected);
            RESIZE_COUNT.with(|resizes| assert_eq!(resizes.get(), *count));
        });
        let mut missing = Vec::new();
        draw(&mut missing, &dir, "missing.png", (0, 0, 8, 3)).unwrap();
        draw(&mut missing, &dir, "missing.png", (0, 0, 4, 1)).unwrap();
        assert!(missing.is_empty());
        RESIZE_COUNT.with(|count| assert_eq!(count.get(), 5));
        let mut restored = Vec::new();
        draw(&mut restored, &dir, "first.png", (0, 0, 8, 3)).unwrap();
        assert!(!restored.is_empty());
        RESIZE_COUNT.with(|count| assert_eq!(count.get(), 6));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn blends_alpha_and_pads_an_odd_pixel_height_with_black() {
        crossterm::style::force_color_output(true);
        let pixels = RgbaImage::from_pixel(1, 1, image::Rgba([200, 100, 50, 128]));
        let mut out = Vec::new();
        draw_pixels(&mut out, &pixels, (0, 0, 3, 1)).unwrap();
        let output = String::from_utf8(out).unwrap();
        assert!(output.contains("\x1b[1;2H"));
        assert!(output.contains("38;2;100;50;25"));
        assert!(output.contains("48;5;0m"));
        assert_eq!(output.matches('▀').count(), 1);
    }

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
        draw_pixels(&mut out, &image.to_rgba8(), (0, 3, 4, 1)).unwrap();
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
