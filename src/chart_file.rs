use anyhow::{ensure, Context, Result};
use std::{fs::OpenOptions, io::Read, path::Path};

pub(crate) fn read(path: &Path) -> Result<Vec<u8>> {
    read_regular(path).with_context(|| format!("cannot read chart {}", path.display()))
}

fn read_regular(path: &Path) -> Result<Vec<u8>> {
    ensure!(path.metadata()?.is_file(), "chart must be a regular file");
    read_opened(path)
}

fn read_opened(path: &Path) -> Result<Vec<u8>> {
    let mut options = OpenOptions::new();
    options.read(true);
    // A file may be replaced with a FIFO after the path metadata check.
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let mut file = options.open(path)?;
    ensure!(file.metadata()?.is_file(), "chart must be a regular file");
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{os::unix::fs::symlink, sync::mpsc, time::Duration};

    #[test]
    fn replaced_file_is_rejected_after_open_without_blocking() {
        let dir = std::env::temp_dir().join(format!("tapline-chart-race-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("chart.bms");
        std::fs::write(&path, b"#TITLE Regular").unwrap();
        assert!(path.metadata().unwrap().is_file());
        std::fs::remove_file(&path).unwrap();
        let name = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        let device = dir.join("device.bms");
        symlink("/dev/zero", &device).unwrap();
        let (sender, receiver) = mpsc::channel();
        let worker_path = path.clone();
        std::thread::spawn(move || {
            let rejected = read_opened(&worker_path).is_err() && read_opened(&device).is_err();
            sender.send(rejected).ok();
        });
        let result = receiver.recv_timeout(Duration::from_secs(2));
        std::fs::remove_dir_all(dir).unwrap();
        assert!(result.unwrap());
    }
}
