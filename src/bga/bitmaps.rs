use super::{crops, image_file, Effect, Extensions, SIZE};
use crate::chart::BgaEvent;
use image::{imageops, DynamicImage, RgbaImage};
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
    let mut bitmaps = HashMap::new();
    let mut cropped = HashMap::new();
    for (path, ids) in aliases {
        let Some(source) = image_file::load(path) else {
            continue;
        };
        if !ids.is_empty() {
            let bitmap = Arc::new(render(&source));
            bitmaps.extend(ids.into_iter().map(|id| (id, Arc::clone(&bitmap))));
        }
        if let Some(requests) = crop_groups.remove(path) {
            let source = source.into_rgba8();
            cropped.extend(
                requests
                    .into_iter()
                    .map(|(id, crop)| (id, Arc::new(crops::render(&source, crop)))),
            );
        }
    }
    // A successfully rendered crop takes precedence over a bitmap with the same ID.
    bitmaps.extend(cropped);
    bitmaps
}

fn reachable_ids(events: &[BgaEvent], extensions: &Extensions) -> HashSet<u32> {
    let switch_frames = extensions.events.iter().filter_map(|event| {
        let Effect::Switch(id) = event.effect else {
            return None;
        };
        let switch = extensions.switches.get(&id)?;
        switch.lane.map(|_| switch.pattern.iter().copied())
    });
    events
        .iter()
        .map(|event| event.bmp_id)
        .chain(switch_frames.flatten())
        .collect()
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
