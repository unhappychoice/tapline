use super::{crops, image_file, Extensions, SIZE};
use image::{imageops, DynamicImage, RgbaImage};
use std::{collections::HashMap, path::PathBuf, sync::Arc};

pub(super) fn load(
    paths: &HashMap<u32, PathBuf>,
    extensions: &Extensions,
) -> HashMap<u32, Arc<RgbaImage>> {
    let mut aliases = HashMap::<_, Vec<_>>::new();
    paths.iter().for_each(|(&id, path)| {
        aliases.entry(path.as_path()).or_default().push(id);
    });
    let mut crop_groups = crops::group(paths, &extensions.crops);
    let mut bitmaps = HashMap::new();
    let mut cropped = HashMap::new();
    for (path, ids) in aliases {
        let Some(source) = image_file::load(path) else {
            continue;
        };
        let bitmap = Arc::new(render(&source));
        bitmaps.extend(ids.into_iter().map(|id| (id, Arc::clone(&bitmap))));
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
