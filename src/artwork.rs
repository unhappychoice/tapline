use crossterm::{
    cursor, queue,
    style::{Color, Print, ResetColor, SetBackgroundColor, SetForegroundColor},
};
use image::{DynamicImage, ImageReader};
use std::{
    cell::RefCell,
    io::Write,
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
            let image = ImageReader::open(&path)
                .ok()
                .and_then(|reader| reader.with_guessed_format().ok())
                .and_then(|reader| reader.decode().ok());
            *cache = Some((path, image));
        }
        match cache.as_ref().and_then(|entry| entry.1.as_ref()) {
            Some(image) => draw_pixels(out, image, area),
            None => Ok(()),
        }
    })
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
}
