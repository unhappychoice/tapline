use crate::fs_guard;
use image::{DynamicImage, ImageReader};
#[cfg(test)]
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Mutex, MutexGuard, PoisonError,
};
use std::{io::BufReader, path::Path};

#[cfg(test)]
pub(super) static LOAD_CALLS: AtomicUsize = AtomicUsize::new(0);

// Decoding now runs on rayon's shared worker threads, so LOAD_CALLS is shared across all
// concurrently running tests. Every test that triggers a real decode (directly or through
// Presentation/ChartImages) must hold this lock, not just the ones asserting on LOAD_CALLS,
// or an unrelated test's decode can skew the count.
#[cfg(test)]
pub(super) fn load_calls_test_lock() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(super) fn load(path: &Path) -> Option<DynamicImage> {
    #[cfg(test)]
    LOAD_CALLS.fetch_add(1, Ordering::SeqCst);
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
        let _lock = load_calls_test_lock();
        let path =
            std::env::temp_dir().join(format!("tapline-bga-image-bomb-{}.png", std::process::id()));
        std::fs::write(&path, crate::image_limits::oversized_png_bytes()).unwrap();
        assert!(load(&path).is_none());
        std::fs::remove_file(&path).unwrap();
    }
}
