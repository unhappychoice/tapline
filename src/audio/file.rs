use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;

pub(super) fn open(path: &Path) -> io::Result<File> {
    ensure_regular(path.metadata()?)?;
    open_verified(path)
}

fn open_verified(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    ensure_regular(file.metadata()?)?;
    Ok(file)
}

fn ensure_regular(metadata: std::fs::Metadata) -> io::Result<()> {
    if metadata.is_file() {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Not a regular audio file",
        ))
    }
}

#[cfg(all(test, unix))]
mod tests {
    #[test]
    fn opening_a_replacement_fifo_does_not_block() {
        super::super::loading_tests::with_fifo(|path| {
            let audio = path.with_file_name("replaced.wav");
            std::fs::write(&audio, b"audio").unwrap();
            super::ensure_regular(audio.metadata().unwrap()).unwrap();
            std::fs::rename(path, &audio).unwrap();
            assert!(super::open_verified(&audio).is_err());
        });
    }
}
