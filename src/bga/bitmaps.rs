use super::{crops, image_file, Effect, Extensions, SIZE};
use crate::chart::BgaEvent;
use image::{imageops, DynamicImage, RgbaImage};
use rayon::prelude::*;
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
};

pub(crate) type Bitmaps = HashMap<u32, Arc<RgbaImage>>;

pub(super) fn load(
    paths: &HashMap<u32, PathBuf>,
    events: &[BgaEvent],
    extensions: &Extensions,
) -> Bitmaps {
    if paths.is_empty() {
        return HashMap::new();
    }

    let ids = reachable_ids(events, extensions);
    let mut crop_groups = crops::group(
        paths,
        ids.iter()
            .filter_map(|&id| extensions.crops.get(&id).map(|crop| (id, crop))),
    );
    let mut aliases = HashMap::<_, Vec<_>>::new();
    ids.into_iter().for_each(|id| {
        if let Some(path) = paths.get(&id) {
            aliases.entry(path.as_path()).or_default().push(id);
        }
    });
    crop_groups.keys().for_each(|&path| {
        aliases.entry(path).or_default();
    });
    let tasks: Vec<_> = aliases
        .into_iter()
        .map(|(path, ids)| (path, ids, crop_groups.remove(path).unwrap_or_default()))
        .collect();
    let (bitmap_batches, cropped_batches): (Vec<_>, Vec<_>) = tasks
        .into_par_iter()
        .filter_map(|(path, ids, requests)| {
            let source = image_file::load(path)?;
            let bitmap_entries = if ids.is_empty() {
                Vec::new()
            } else {
                let bitmap = Arc::new(render(&source));
                ids.into_iter()
                    .map(|id| (id, Arc::clone(&bitmap)))
                    .collect()
            };
            let cropped_entries = if requests.is_empty() {
                Vec::new()
            } else {
                let source = source.into_rgba8();
                requests
                    .into_iter()
                    .map(|(id, crop)| (id, Arc::new(crops::render(&source, crop))))
                    .collect()
            };
            Some((bitmap_entries, cropped_entries))
        })
        .unzip();
    let mut bitmaps = HashMap::new();
    bitmaps.extend(bitmap_batches.into_iter().flatten());
    // A successfully rendered crop takes precedence over a bitmap with the same ID.
    bitmaps.extend(cropped_batches.into_iter().flatten());
    bitmaps
}

fn reachable_ids(events: &[BgaEvent], extensions: &Extensions) -> HashSet<u32> {
    events
        .iter()
        .map(|event| event.bmp_id)
        .chain(switch_frame_ids(extensions))
        .collect()
}

fn switch_frame_ids(extensions: &Extensions) -> impl Iterator<Item = u32> + '_ {
    let mut seen = HashSet::new();
    extensions
        .events
        .iter()
        .filter_map(move |event| match event.effect {
            Effect::Switch(id) if seen.insert(id) => extensions.switches.get(&id),
            _ => None,
        })
        .filter(|switch| switch.lane.is_some())
        .flat_map(|switch| switch.pattern.iter().copied())
}

fn render(source: &DynamicImage) -> RgbaImage {
    let image = source
        .resize(SIZE, SIZE, imageops::FilterType::Nearest)
        .to_rgba8();
    let mut canvas = RgbaImage::new(SIZE, SIZE);
    imageops::overlay(
        &mut canvas,
        &image,
        ((SIZE - image.width()) / 2) as i64,
        ((SIZE - image.height()) / 2) as i64,
    );
    canvas
}

#[cfg(test)]
mod tests;
