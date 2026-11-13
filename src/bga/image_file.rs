use image::{DynamicImage, ImageReader};
use std::{fs::OpenOptions, io::BufReader, path::Path};

pub(super) fn load(path: &Path) -> Option<DynamicImage> {
    path.metadata().ok()?.is_file().then_some(())?;
    load_verified(path)
}

fn load_verified(path: &Path) -> Option<DynamicImage> {
    let mut options = OpenOptions::new();
    options.read(true);
    // The path may have been replaced with a FIFO after the metadata check.
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = options.open(path).ok()?;
    file.metadata().ok()?.is_file().then_some(())?;
    ImageReader::new(BufReader::new(file))
        .with_guessed_format()
        .ok()?
        .decode()
        .ok()
}

#[cfg(all(test, unix))]
mod tests;
