mod extensions;
pub use extensions::*;

use crate::{chart::BgaLayer, game::Game};
use crossterm::{
    cursor, queue,
    style::{Color, Print, ResetColor, SetBackgroundColor, SetForegroundColor},
};
use image::{imageops, Rgba, RgbaImage};
use std::{cell::RefCell, collections::HashMap, io::Write, path::PathBuf};

mod crops;
mod effect_timeline;
mod timeline;
mod video;

#[derive(Default)]
struct Playback {
    event: Option<usize>,
    video: Option<video::Video>,
}

const SIZE: u32 = 64;
const POOR_MS: f64 = 500.0;

#[derive(Default)]
pub struct ImageBank(
    HashMap<u32, RgbaImage>,
    HashMap<u32, PathBuf>,
    RefCell<[Playback; 4]>,
    timeline::Timeline,
    effect_timeline::Timeline,
);

impl ImageBank {
    pub fn load(
        paths: &HashMap<u32, PathBuf>,
        events: &[crate::chart::BgaEvent],
        extensions: &Extensions,
    ) -> Self {
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
            paths
                .iter()
                .filter(|(_, path)| is_video(path))
                .map(|(id, path)| (*id, path.clone()))
                .collect(),
            RefCell::default(),
            timeline::Timeline::new(events),
            effect_timeline::Timeline::new(extensions),
        )
    }

    pub fn prepare_crops(&mut self, chart: &crate::chart::Chart) {
        self.0.extend(crops::prepare(chart, |path| {
            image::open(path).ok().map(|image| image.to_rgba8())
        }));
    }

    fn layer(&self, game: &Game, layer: BgaLayer, now_ms: f64) -> Option<RgbaImage> {
        let slot = timeline::slot(layer);
        let event = self
            .3
            .latest(layer, now_ms)
            .map(|index| (index, &game.chart.bga[index]));
        let mut states = self.2.borrow_mut();
        let state = &mut states[slot];
        let index = event.map(|(index, _)| index);
        if state.event != index {
            state.event = index;
            state.video = event
                .and_then(|(_, event)| self.1.get(&event.bmp_id))
                .and_then(|path| video::Video::open(path));
        }
        let (_, event) = event?;
        self.0
            .get(&event.bmp_id)
            .cloned()
            .or_else(|| state.video.as_mut()?.frame(now_ms - event.time_ms))
    }

    fn frame(&self, game: &Game, now_ms: f64) -> RgbaImage {
        let ext = &game.chart.bga_extensions;
        let mut frame = RgbaImage::from_pixel(SIZE, SIZE, Rgba([0, 0, 0, 255]));
        let poor = ext.poor_mode != 2
            && game
                .flash
                .last_miss_at
                .is_some_and(|at| (0.0..POOR_MS).contains(&(now_ms - at)))
            && self.layer(game, BgaLayer::Poor, now_ms).is_some();
        let layers = if poor && ext.poor_mode == 0 {
            vec![BgaLayer::Poor]
        } else {
            let mut layers = vec![BgaLayer::Base, BgaLayer::Overlay, BgaLayer::Overlay2];
            if poor {
                layers.push(BgaLayer::Poor);
            }
            layers
        };
        for layer in layers {
            let Some(image) = self.layer(game, layer, now_ms) else {
                continue;
            };
            let state = self.4.state(layer, now_ms);
            let key = state.key.or_else(|| {
                (layer != BgaLayer::Base && (layer != BgaLayer::Poor || ext.poor_mode == 1))
                    .then_some([0, 0, 0])
            });
            composite(&mut frame, &image, state.alpha, key.map(ColorKey::Rgb));
        }
        if let Some((id, _, start)) = game.bga_switch.active {
            if let Some(switch) = ext.switches.get(&id) {
                let elapsed = now_ms - start;
                let visible = elapsed >= 0.0
                    && if switch.duration_ms == 0 {
                        game.bga_switch.held
                    } else {
                        elapsed < switch.duration_ms as f64
                    };
                if visible && !switch.pattern.is_empty() {
                    let index = (elapsed / switch.frame_ms as f64) as usize;
                    let index = if switch.looping {
                        index % switch.pattern.len()
                    } else {
                        index.min(switch.pattern.len() - 1)
                    };
                    if let Some(image) = self.0.get(&switch.pattern[index]) {
                        let [a, r, g, b] = switch.argb;
                        composite(&mut frame, image, 255, Some(ColorKey::Rgba([r, g, b, a])));
                    }
                }
            }
        }
        frame
    }
}

fn is_video(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            matches!(
                ext.to_ascii_lowercase().as_str(),
                "mp4" | "webm" | "mpg" | "mpeg" | "avi" | "mkv" | "mov" | "m4v"
            )
        })
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
    let Some(width) = panel_width(left_margin, height)
        .filter(|_| !game.bga.0.is_empty() || !game.bga.1.is_empty())
    else {
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

enum ColorKey {
    Rgb([u8; 3]),
    Rgba([u8; 4]),
}

fn composite(frame: &mut RgbaImage, image: &RgbaImage, alpha: u8, key: Option<ColorKey>) {
    let mut image = image.clone();
    image.pixels_mut().for_each(|pixel| {
        if key.as_ref().is_some_and(|key| match key {
            ColorKey::Rgb(rgb) => pixel.0[..3] == rgb[..],
            ColorKey::Rgba(rgba) => pixel.0 == *rgba,
        }) {
            pixel[3] = 0;
        }
        pixel[3] = (u16::from(pixel[3]) * u16::from(alpha) / 255) as u8;
    });
    imageops::overlay(frame, &image, 0, 0);
}
