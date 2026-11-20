mod extensions;
pub use extensions::*;

use crate::{chart::BgaLayer, game::Game};
use crossterm::{
    cursor, queue,
    style::{Color, Print, ResetColor, SetBackgroundColor, SetForegroundColor},
};
use image::{imageops, Rgba, RgbaImage};
use std::{
    cell::{RefCell, RefMut},
    collections::HashMap,
    io::Write,
    path::PathBuf,
};

mod crops;
mod effect_timeline;
mod image_file;
mod presentation;
pub use presentation::Presentation;
mod timeline;
mod video;
mod video_worker;

#[derive(Default)]
struct Playback {
    event: Option<usize>,
    video: Option<video::Video>,
    worker: Option<video_worker::Worker>,
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
    RefCell<Option<RenderedFrame>>,
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
                    let image = image_file::load(path)?
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
            RefCell::default(),
        )
    }

    #[cfg(test)]
    pub(crate) fn set_test_decoder<D>(
        &mut self,
        mut open: impl FnMut(&std::path::Path) -> Option<(D, [u8; 4])> + Send + 'static,
    ) {
        self.2.get_mut()[timeline::slot(BgaLayer::Base)].worker =
            video_worker::Worker::start(move |path| {
                let (decoder, color) = open(path)?;
                Some((decoder, video::Video::test_frame(color)))
            });
    }

    pub fn prepare_crops(&mut self, chart: &crate::chart::Chart) {
        *self.5.get_mut() = None;
        self.0.extend(crops::prepare(chart, |path| {
            image_file::load(path).map(|image| image.to_rgba8())
        }));
    }

    /// Return the BMP id currently active on a given BGA layer at `now_ms`,
    /// or `None` if no event on that layer has fired yet. `events` must be
    /// the same slice `self` was built from. Uses the per-layer timeline
    /// built once in `ImageBank::load`, so the lookup is O(log n) per layer
    /// rather than a per-frame linear scan.
    pub fn active_bga_id(
        &self,
        events: &[crate::chart::BgaEvent],
        layer: BgaLayer,
        now_ms: f64,
    ) -> Option<u32> {
        self.3
            .latest(layer, now_ms)
            .map(|index| events[index].bmp_id)
    }

    fn layer(&self, game: &Game, layer: BgaLayer, now_ms: f64) -> Option<LayerImage> {
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
            state.video = None;
            let path = event.and_then(|(_, event)| self.1.get(&event.bmp_id));
            if path.is_some() && state.worker.is_none() {
                state.worker = video_worker::Worker::start(video::Video::open);
            }
            if let Some(worker) = &state.worker {
                worker.request(path.cloned());
            }
        }
        if state.video.is_none() {
            state.video = state
                .worker
                .as_ref()
                .and_then(video_worker::Worker::take_video);
        }
        let (_, event) = event?;
        if self.0.contains_key(&event.bmp_id) {
            Some(LayerImage::Bitmap(event.bmp_id))
        } else {
            state
                .video
                .as_mut()?
                .frame(now_ms - event.time_ms)
                .map(LayerImage::Video)
        }
    }

    #[cfg(test)]
    fn frame(&self, game: &Game, now_ms: f64) -> RgbaImage {
        self.rendered_frame(game, now_ms, SIZE).clone()
    }

    fn rendered_frame(&self, game: &Game, now_ms: f64, width: u32) -> RefMut<'_, RgbaImage> {
        let layers = self.visible_layers(game, now_ms);
        let bitmaps = layers.each_ref().map(|layer| match layer {
            Some((LayerImage::Bitmap(id), style)) => Some((*id, *style)),
            _ => None,
        });
        let has_video = layers
            .iter()
            .any(|layer| matches!(layer, Some((LayerImage::Video(_), _))));
        let key = (!has_video).then_some(FrameKey { width, bitmaps });
        let mut cached = self.5.borrow_mut();
        if key.is_none() || !cached.as_ref().is_some_and(|frame| frame.key == key) {
            let mut frame = RgbaImage::from_pixel(SIZE, SIZE, Rgba([0, 0, 0, 255]));
            for (image, style) in layers.iter().flatten() {
                let image = match image {
                    LayerImage::Bitmap(id) => &self.0[id],
                    LayerImage::Video(image) => image,
                };
                composite(&mut frame, image, style.alpha, style.key);
            }
            let pixels = imageops::resize(&frame, width, width, imageops::FilterType::Nearest);
            *cached = Some(RenderedFrame { key, pixels });
        }
        RefMut::map(cached, |frame| &mut frame.as_mut().unwrap().pixels)
    }

    fn visible_layers(&self, game: &Game, now_ms: f64) -> [Option<Layer>; 5] {
        let poor_mode = game.chart.bga_extensions.poor_mode;
        let poor = (poor_mode != 2
            && game
                .flash
                .last_miss_at
                .is_some_and(|at| (0.0..POOR_MS).contains(&(now_ms - at))))
        .then(|| self.styled_layer(game, BgaLayer::Poor, now_ms))
        .flatten();
        let normal = [BgaLayer::Base, BgaLayer::Overlay, BgaLayer::Overlay2];
        let [base, overlay, overlay2] = if poor.is_some() && poor_mode == 0 {
            [None, None, None]
        } else {
            normal.map(|layer| self.styled_layer(game, layer, now_ms))
        };
        [
            base,
            overlay,
            overlay2,
            poor,
            self.switch_layer(game, now_ms),
        ]
    }

    fn styled_layer(&self, game: &Game, layer: BgaLayer, now_ms: f64) -> Option<Layer> {
        let image = self.layer(game, layer, now_ms)?;
        let state = self.4.state(layer, now_ms);
        let extensions = &game.chart.bga_extensions;
        let key = state.key.or_else(|| {
            (extensions.overlay_transparency == OverlayTransparency::BlackColorKey
                && layer != BgaLayer::Base
                && (layer != BgaLayer::Poor || extensions.poor_mode == 1))
                .then_some([0, 0, 0])
        });
        Some((
            image,
            LayerStyle {
                alpha: state.alpha,
                key: key.map(ColorKey::Rgb),
            },
        ))
    }

    fn switch_layer(&self, game: &Game, now_ms: f64) -> Option<Layer> {
        let (id, _, start) = game.bga_switch.active?;
        let switch = game.chart.bga_extensions.switches.get(&id)?;
        let elapsed = now_ms - start;
        let visible = elapsed >= 0.0
            && if switch.duration_ms == 0 {
                game.bga_switch.held
            } else {
                elapsed < switch.duration_ms as f64
            };
        if !visible || switch.pattern.is_empty() {
            return None;
        }
        let index = (elapsed / switch.frame_ms as f64) as usize;
        let index = if switch.looping {
            index % switch.pattern.len()
        } else {
            index.min(switch.pattern.len() - 1)
        };
        let id = switch.pattern[index];
        let [a, r, g, b] = switch.argb;
        self.0.contains_key(&id).then_some((
            LayerImage::Bitmap(id),
            LayerStyle {
                alpha: 255,
                key: Some(ColorKey::Rgba([r, g, b, a])),
            },
        ))
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

pub(crate) fn panel_width(left_margin: u16, height: u16) -> Option<u16> {
    let width = left_margin
        .saturating_sub(2)
        .min(32)
        .min(height.saturating_mul(2));
    (width >= 16).then_some(width / 2 * 2)
}

pub fn draw(
    out: &mut impl Write,
    game: &Game,
    bank: &ImageBank,
    now_ms: f64,
    left_margin: u16,
    top: u16,
    height: u16,
) -> anyhow::Result<()> {
    let Some(width) =
        panel_width(left_margin, height).filter(|_| !bank.0.is_empty() || !bank.1.is_empty())
    else {
        return Ok(());
    };
    let frame = bank.rendered_frame(game, now_ms, width.into());
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

type Layer = (LayerImage, LayerStyle);

enum LayerImage {
    Bitmap(u32),
    Video(RgbaImage),
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct LayerStyle {
    alpha: u8,
    key: Option<ColorKey>,
}

#[derive(PartialEq, Eq)]
struct FrameKey {
    width: u32,
    bitmaps: [Option<(u32, LayerStyle)>; 5],
}

struct RenderedFrame {
    key: Option<FrameKey>,
    pixels: RgbaImage,
}

#[derive(Clone, Copy, PartialEq, Eq)]
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
