use crate::assets::AssetIndex;
use crate::fs_guard;
use crossterm::{
    cursor, queue,
    style::{Color, Print, ResetColor, SetBackgroundColor, SetForegroundColor},
};
use image::{DynamicImage, ImageReader, RgbaImage};
use std::{
    cell::RefCell,
    collections::VecDeque,
    io::{BufReader, Write},
    path::{Path, PathBuf},
    time::SystemTime,
};

const THUMBNAIL_CACHE_CAPACITY: usize = 32;

thread_local! {
    static CACHE: RefCell<ArtworkCache> = const { RefCell::new(ArtworkCache {
        thumbnails: VecDeque::new(),
        paths: VecDeque::new(),
    }) };
    #[cfg(test)]
    static LOAD_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
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
    CACHE.with(
        |cache| match cache.borrow_mut().pixels(dir, name, (area.2, area.3)) {
            Some(image) => draw_pixels(out, image, area),
            None => Ok(()),
        },
    )
}

struct ArtworkCache {
    thumbnails: VecDeque<Thumbnail>,
    paths: VecDeque<ResolvedPath>,
}

struct ResolvedPath {
    dir: PathBuf,
    name: String,
    path: PathBuf,
}

struct Thumbnail {
    path: PathBuf,
    size: (u16, u16),
    source: SourceVersion,
    pixels: Option<RgbaImage>,
}

#[derive(PartialEq, Eq)]
struct SourceVersion {
    modified: SystemTime,
    len: u64,
    #[cfg(unix)]
    identity: (u64, u64, i64, i64),
}

impl ArtworkCache {
    fn pixels(&mut self, dir: &Path, name: &str, size: (u16, u16)) -> Option<&RgbaImage> {
        let (path, source) = self.resolve(dir, name)?;
        self.thumbnails
            .retain(|entry| entry.path != path || entry.source == source);
        let index = self
            .thumbnails
            .iter()
            .position(|entry| entry.path == path && entry.size == size);
        let entry = match index.and_then(|index| self.thumbnails.remove(index)) {
            Some(entry) => entry,
            None => Thumbnail {
                pixels: load_image(&path).map(|image| resize(&image, size)),
                path,
                size,
                source,
            },
        };
        self.thumbnails.push_front(entry);
        self.thumbnails.truncate(THUMBNAIL_CACHE_CAPACITY);
        self.thumbnails.front()?.pixels.as_ref()
    }

    fn resolve(&mut self, dir: &Path, name: &str) -> Option<(PathBuf, SourceVersion)> {
        let index = self
            .paths
            .iter()
            .position(|entry| entry.dir == dir && entry.name == name);
        let cached = index
            .and_then(|index| self.paths.remove(index))
            .and_then(|entry| Some((SourceVersion::read(&entry.path)?, entry)));
        if let Some((source, entry)) = cached {
            let path = entry.path.clone();
            self.paths.push_front(entry);
            return Some((path, source));
        }
        let path = AssetIndex::new(dir).resolve_path(name, &[])?;
        let source = SourceVersion::read(&path)?;
        self.paths.push_front(ResolvedPath {
            dir: dir.to_path_buf(),
            name: name.to_owned(),
            path: path.clone(),
        });
        self.paths.truncate(THUMBNAIL_CACHE_CAPACITY);
        Some((path, source))
    }
}

impl SourceVersion {
    fn read(path: &Path) -> Option<Self> {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;

        let metadata = path.metadata().ok()?;
        if !metadata.is_file() {
            return None;
        }
        Some(Self {
            modified: metadata.modified().ok()?,
            len: metadata.len(),
            #[cfg(unix)]
            identity: (
                metadata.dev(),
                metadata.ino(),
                metadata.ctime(),
                metadata.ctime_nsec(),
            ),
        })
    }
}

fn resize(image: &DynamicImage, (width, height): (u16, u16)) -> RgbaImage {
    #[cfg(test)]
    RESIZE_COUNT.with(|count| count.set(count.get() + 1));
    image
        .thumbnail(u32::from(width), u32::from(height) * 2)
        .to_rgba8()
}

fn load_image(path: &Path) -> Option<DynamicImage> {
    #[cfg(test)]
    LOAD_COUNT.with(|count| count.set(count.get() + 1));
    let file = fs_guard::open(path).ok()?;
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
    use crate::assets::DIRECTORY_VISITS;

    #[test]
    fn reuses_resized_pixels_and_refreshes_for_image_and_area_changes() {
        crossterm::style::force_color_output(true);
        reset_cache();
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
            ("first.png", &first, (0, 0, 8, 3), 1, 1),
            ("first.png", &first, (0, 0, 8, 3), 1, 1),
            ("first.png", &first, (5, 7, 8, 3), 1, 1),
            ("first.png", &first, (5, 7, 4, 3), 2, 1),
            ("first.png", &first, (5, 7, 4, 1), 3, 1),
            ("second.png", &second, (5, 7, 4, 1), 4, 2),
            ("second.png", &second, (5, 7, 4, 1), 4, 2),
            ("first.png", &first, (0, 0, 8, 3), 4, 2),
            ("./first.png", &first, (5, 7, 4, 1), 4, 3),
            ("second.png", &second, (5, 7, 4, 1), 4, 3),
        ];
        cases.iter().for_each(|(name, image, area, count, visits)| {
            let mut actual = Vec::new();
            draw(&mut actual, &dir, name, *area).unwrap();
            let pixels = resize_expected(image, (area.2, area.3));
            let mut expected = Vec::new();
            draw_pixels(&mut expected, &pixels, *area).unwrap();
            assert_eq!(actual, expected);
            assert_image_work(*count);
            DIRECTORY_VISITS.with(|count| assert_eq!(count.get(), *visits));
        });
        let mut missing = Vec::new();
        draw(&mut missing, &dir, "missing.png", (0, 0, 8, 3)).unwrap();
        draw(&mut missing, &dir, "missing.png", (0, 0, 4, 1)).unwrap();
        assert!(missing.is_empty());
        assert_image_work(4);
        let mut restored = Vec::new();
        draw(&mut restored, &dir, "first.png", (0, 0, 8, 3)).unwrap();
        assert!(!restored.is_empty());
        assert_image_work(4);
        DIRECTORY_VISITS.with(|count| assert_eq!(count.get(), 5));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn refreshes_cached_sizes_when_a_banner_changes() {
        reset_cache();
        let dir =
            std::env::temp_dir().join(format!("tapline-artwork-refresh-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cover.bmp");
        let first = DynamicImage::new_rgb8(8, 8);
        let replacement =
            DynamicImage::ImageRgb8(image::RgbImage::from_pixel(8, 8, image::Rgb([255, 0, 0])));
        first.save(&path).unwrap();
        let initial = path.metadata().unwrap();
        let areas = [(0, 0, 8, 3), (0, 0, 4, 2)];
        let assert_banner = |image, area: (u16, u16, u16, u16)| {
            let mut actual = Vec::new();
            draw(&mut actual, &dir, "cover.bmp", area).unwrap();
            let mut expected = Vec::new();
            draw_pixels(
                &mut expected,
                &resize_expected(image, (area.2, area.3)),
                area,
            )
            .unwrap();
            assert_eq!(actual, expected);
        };
        areas
            .into_iter()
            .for_each(|area| assert_banner(&first, area));
        replacement.save(&path).unwrap();
        let file = std::fs::File::options().write(true).open(&path).unwrap();
        file.set_modified(initial.modified().unwrap() - std::time::Duration::from_secs(60))
            .unwrap();
        assert_eq!(file.metadata().unwrap().len(), initial.len());
        drop(file);
        areas
            .into_iter()
            .for_each(|area| assert_banner(&replacement, area));
        assert_image_work(4);
        DIRECTORY_VISITS.with(|count| assert_eq!(count.get(), 1));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn resolves_again_when_the_cached_path_disappears() {
        reset_cache();
        let dir =
            std::env::temp_dir().join(format!("tapline-artwork-rename-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let image = DynamicImage::new_rgb8(2, 2);
        let path = dir.join("cover.png");
        image.save(&path).unwrap();
        let render = || {
            let mut out = Vec::new();
            draw(&mut out, &dir, "cover.png", (0, 0, 2, 1)).unwrap();
            out
        };
        let expected = render();
        assert!(!expected.is_empty());
        std::fs::remove_file(path).unwrap();
        assert!(render().is_empty());
        image.save(dir.join("Cover.png")).unwrap();
        assert_eq!(render(), expected);
        assert_eq!(render(), expected);
        DIRECTORY_VISITS.with(|count| assert_eq!(count.get(), 3));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn evicts_the_least_recently_used_thumbnail_at_capacity() {
        reset_cache();
        let dir =
            std::env::temp_dir().join(format!("tapline-artwork-bound-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let image = DynamicImage::new_rgb8(2, 2);
        for index in 0..=THUMBNAIL_CACHE_CAPACITY {
            image.save(dir.join(format!("{index}.png"))).unwrap();
        }
        let draw_banner =
            |index| draw(&mut Vec::new(), &dir, &format!("{index}.png"), (0, 0, 2, 1)).unwrap();
        (0..THUMBNAIL_CACHE_CAPACITY).for_each(draw_banner);
        draw_banner(0);
        draw_banner(THUMBNAIL_CACHE_CAPACITY);
        CACHE.with(|cache| {
            let cache = cache.borrow();
            assert_eq!(cache.thumbnails.len(), THUMBNAIL_CACHE_CAPACITY);
            assert_eq!(cache.paths.len(), THUMBNAIL_CACHE_CAPACITY);
        });
        draw_banner(0);
        assert_image_work(THUMBNAIL_CACHE_CAPACITY + 1);
        draw_banner(1);
        assert_image_work(THUMBNAIL_CACHE_CAPACITY + 2);
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn reset_cache() {
        CACHE.with(|cache| {
            let mut cache = cache.borrow_mut();
            cache.thumbnails.clear();
            cache.paths.clear();
        });
        LOAD_COUNT.with(|count| count.set(0));
        RESIZE_COUNT.with(|count| count.set(0));
        DIRECTORY_VISITS.with(|count| count.set(0));
    }

    fn assert_image_work(expected: usize) {
        LOAD_COUNT.with(|count| assert_eq!(count.get(), expected));
        RESIZE_COUNT.with(|count| assert_eq!(count.get(), expected));
    }

    fn resize_expected(image: &DynamicImage, (width, height): (u16, u16)) -> RgbaImage {
        image
            .thumbnail(u32::from(width), u32::from(height) * 2)
            .to_rgba8()
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
