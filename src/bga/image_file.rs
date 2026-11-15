use crate::fs_guard;
use image::{DynamicImage, ImageReader};
use std::{io::BufReader, path::Path};

pub(super) fn load(path: &Path) -> Option<DynamicImage> {
    decode(fs_guard::open(path).ok()?)
}

fn decode(file: std::fs::File) -> Option<DynamicImage> {
    ImageReader::new(BufReader::new(file))
        .with_guessed_format()
        .ok()?
        .decode()
        .ok()
}

#[cfg(all(test, unix))]
mod tests;
