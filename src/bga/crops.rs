use super::{Crop, SIZE};
use crate::chart::Chart;
use image::{imageops, RgbaImage};
use std::{collections::HashMap, path::Path};

pub(super) fn prepare(
    chart: &Chart,
    mut load: impl FnMut(&Path) -> Option<RgbaImage>,
) -> HashMap<u32, RgbaImage> {
    let mut groups = HashMap::<_, Vec<_>>::new();
    chart.bga_extensions.crops.iter().for_each(|(&id, crop)| {
        if let Some(path) = chart.bmp_paths.get(&crop.image) {
            groups.entry(path).or_default().push((id, crop));
        }
    });
    let mut images = HashMap::new();
    groups.into_iter().for_each(|(path, crops)| {
        if let Some(source) = load(path) {
            images.extend(
                crops
                    .into_iter()
                    .map(|(id, crop)| (id, render(&source, crop))),
            );
        }
    });
    images
}

fn render(source: &RgbaImage, crop: &Crop) -> RgbaImage {
    let [x1, y1, x2, y2, dx, dy] = crop.rect;
    let mut canvas = RgbaImage::new(256, 256);
    canvas.enumerate_pixels_mut().for_each(|(x, y, pixel)| {
        let sx = x as i64 - dx as i64 + x1.max(0) as i64;
        let sy = y as i64 - dy as i64 + y1.max(0) as i64;
        if sx >= x1.max(0) as i64
            && sy >= y1.max(0) as i64
            && sx < x2 as i64
            && sy < y2 as i64
            && sx < source.width() as i64
            && sy < source.height() as i64
        {
            *pixel = *source.get_pixel(sx as u32, sy as u32);
        }
    });
    imageops::resize(&canvas, SIZE, SIZE, imageops::FilterType::Nearest)
}

#[cfg(test)]
mod tests;
