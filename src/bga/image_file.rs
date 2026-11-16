use crate::fs_guard;
use image::{DynamicImage, ImageReader};
use std::{io::BufReader, path::Path};

#[cfg(test)]
thread_local! {
    pub(super) static LOAD_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

pub(super) fn load(path: &Path) -> Option<DynamicImage> {
    #[cfg(test)]
    LOAD_CALLS.set(LOAD_CALLS.get() + 1);
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
