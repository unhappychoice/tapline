use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;

/// Opens `path` for reading, rejecting anything that is not a regular file
/// both before and after the open to guard against FIFOs, devices, and
/// symlink races.
pub(crate) fn open(path: &Path) -> io::Result<File> {
    ensure_regular(path.metadata()?)?;
    open_verified(path)
}

/// Opens `path` without first checking its path metadata, re-verifying
/// afterwards that the opened file is still regular. Exposed separately so
/// callers that already checked the path can race-test the open itself.
pub(crate) fn open_verified(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    // A file may be replaced with a FIFO after the metadata check above.
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
            "not a regular file",
        ))
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{sync::mpsc, time::Duration};

    #[test]
    fn opening_a_replacement_fifo_does_not_block() {
        let dir = std::env::temp_dir().join(format!("tapline-fs-guard-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fifo = dir.join("blocked");
        let name = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        let replaced = dir.join("replaced");
        std::fs::write(&replaced, b"regular").unwrap();
        ensure_regular(replaced.metadata().unwrap()).unwrap();
        std::fs::rename(&fifo, &replaced).unwrap();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            sender.send(open_verified(&replaced).is_err()).ok();
        });
        let result = receiver.recv_timeout(Duration::from_secs(5));
        std::fs::remove_dir_all(dir).unwrap();
        assert!(result.expect("replacement FIFO must not block open"));
    }

    #[test]
    fn rejects_directories_and_devices() {
        assert!(open(Path::new(".")).is_err());
        assert!(open(Path::new("/dev/zero")).is_err());
    }
}
