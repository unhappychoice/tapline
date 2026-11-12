use crate::{chart::BgaLayer, game::Game, render::active_bga_id};
use crossterm::{
    cursor, queue,
    style::{Color, Print, ResetColor, SetBackgroundColor, SetForegroundColor},
};
use image::{imageops, Rgba, RgbaImage};
use std::{collections::HashMap, io::Write, path::PathBuf};

const SIZE: u32 = 64;
const POOR_MS: f64 = 500.0;

#[derive(Default)]
pub struct ImageBank(HashMap<u32, RgbaImage>);

impl ImageBank {
    pub fn load(paths: &HashMap<u32, PathBuf>) -> Self {
        Self(
            paths
                .iter()
                .filter_map(|(id, path)| {
                    let reader = image::ImageReader::open(path)
                        .ok()?
                        .with_guessed_format()
                        .ok()?;
                    let image = reader
                        .decode()
                        .ok()?
                        .resize(SIZE, SIZE, imageops::FilterType::Nearest)
                        .to_rgba8();
                    let mut canvas = RgbaImage::new(SIZE, SIZE);
                    imageops::overlay(
                        &mut canvas,
                        &image,
                        ((SIZE - image.width()) / 2) as i64,
                        ((SIZE - image.height()) / 2) as i64,
                    );
                    Some((*id, canvas))
                })
                .collect(),
        )
    }

    fn frame(&self, game: &Game, now_ms: f64) -> RgbaImage {
        let layer =
            |layer| active_bga_id(&game.chart.bga, layer, now_ms).and_then(|id| self.0.get(&id));
        let mut frame = RgbaImage::from_pixel(SIZE, SIZE, Rgba([0, 0, 0, 255]));
        let poor = game
            .flash
            .last_miss_at
            .filter(|at| (0.0..POOR_MS).contains(&(now_ms - at)))
            .and_then(|_| layer(BgaLayer::Poor));
        if let Some(poor) = poor {
            imageops::overlay(&mut frame, poor, 0, 0);
            return frame;
        }
        if let Some(base) = layer(BgaLayer::Base) {
            imageops::overlay(&mut frame, base, 0, 0);
        }
        if let Some(overlay) = layer(BgaLayer::Overlay) {
            let mut overlay = overlay.clone();
            overlay
                .pixels_mut()
                .filter(|p| p.0[..3] == [0, 0, 0])
                .for_each(|p| p.0[3] = 0);
            imageops::overlay(&mut frame, &overlay, 0, 0);
        }
        frame
    }
}

fn panel_width(left_margin: u16, height: u16) -> Option<u16> {
    let width = left_margin
        .saturating_sub(2)
        .min(32)
        .min(height.saturating_mul(2));
    (width >= 16).then_some(width / 2 * 2)
}

pub fn draw(
    out: &mut impl Write,
    game: &Game,
    now_ms: f64,
    left_margin: u16,
    top: u16,
    height: u16,
) -> anyhow::Result<()> {
    let Some(width) = panel_width(left_margin, height).filter(|_| !game.bga.0.is_empty()) else {
        return Ok(());
    };
    let frame = imageops::resize(
        &game.bga.frame(game, now_ms),
        width.into(),
        width.into(),
        imageops::FilterType::Nearest,
    );
    let color = |x, y| {
        let p = frame.get_pixel(x, y);
        Color::Rgb {
            r: p[0],
            g: p[1],
            b: p[2],
        }
    };
    for y in 0..width / 2 {
        queue!(out, cursor::MoveTo(left_margin - width - 2, top + y))?;
        for x in 0..width {
            queue!(
                out,
                SetForegroundColor(color(x.into(), u32::from(y) * 2)),
                SetBackgroundColor(color(x.into(), u32::from(y) * 2 + 1)),
                Print('▀')
            )?;
        }
        queue!(out, ResetColor)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
