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
    let mut reader = ImageReader::new(BufReader::new(file))
        .with_guessed_format()
        .ok()?;
    reader.limits(crate::image_limits::decode_limits());
    reader.decode().ok()
}

#[cfg(all(test, unix))]
mod tests;

#[cfg(test)]
mod limit_tests {
    use super::*;

    #[test]
    fn rejects_images_whose_declared_dimensions_exceed_the_decode_limit() {
        let path = std::env::temp_dir().join(format!(
            "tapline-bga-image-bomb-{}.png",
            std::process::id()
        ));
        std::fs::write(&path, crate::image_limits::oversized_png_bytes()).unwrap();
        assert!(load(&path).is_none());
        std::fs::remove_file(&path).unwrap();
    }
}
