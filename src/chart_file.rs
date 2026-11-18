use crate::fs_guard;
use anyhow::{ensure, Context, Result};
use std::{io::Read, path::Path};

const MAX_CHART_BYTES: u64 = 16 * 1024 * 1024;

pub(crate) fn read(path: &Path) -> Result<Vec<u8>> {
    read_regular(path).with_context(|| format!("cannot read chart {}", path.display()))
}

fn read_regular(path: &Path) -> Result<Vec<u8>> {
    let file = fs_guard::open(path)?;
    #[cfg(test)]
    crate::select::count_content_read();
    read_bounded(file)
}

fn read_bounded(reader: impl Read) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    // Read one extra byte to distinguish an exact fit from oversized input.
    reader.take(MAX_CHART_BYTES + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_CHART_BYTES,
        "chart exceeds the maximum size of {MAX_CHART_BYTES} bytes"
    );
    Ok(bytes)
}

#[cfg(test)]
mod size_tests {
    use super::*;
    use crate::{cli, select};
    use clap::Parser;
    use std::{fs, io};

    #[test]
    fn accepts_a_chart_at_the_size_limit() {
        let input = io::repeat(b' ').take(MAX_CHART_BYTES);
        assert_eq!(read_bounded(input).unwrap().len() as u64, MAX_CHART_BYTES);
    }

    #[test]
    fn rejects_oversized_input_after_reading_only_one_extra_byte() {
        let mut input = io::repeat(b' ').take(MAX_CHART_BYTES + 2);
        let error = read_bounded(&mut input).unwrap_err();
        assert!(error.to_string().contains("exceeds the maximum size"));
        assert_eq!(input.limit(), 1);
    }

    #[test]
    fn discovery_skips_oversized_charts_and_direct_loading_reports_the_limit() {
        let dir = std::env::temp_dir().join(format!("tapline-chart-size-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let valid = dir.join("valid.bms");
        let oversized = dir.join("oversized.bms");
        let chart = include_bytes!("../tests/fixtures/sample.bms");
        fs::write(&valid, chart).unwrap();
        fs::write(&oversized, chart).unwrap();
        fs::OpenOptions::new()
            .write(true)
            .open(&oversized)
            .unwrap()
            .set_len(MAX_CHART_BYTES + 1)
            .unwrap();

        let charts = select::scan(&dir);
        let args = cli::Args::parse_from(["tapline", "--file", oversized.to_str().unwrap()]);
        let choice = cli::choose_chart(&mut io::stdout(), &args).unwrap();
        let result = cli::load_chart(&choice, &args);
        fs::remove_dir_all(dir).unwrap();

        assert_eq!(charts.len(), 1);
        assert_eq!(charts[0].path, valid);
        let error = result.err().expect("oversized chart accepted");
        let message = format!("{error:#}");
        assert!(message.contains("oversized.bms"), "{message}");
        assert!(
            message.contains(&format!("maximum size of {MAX_CHART_BYTES} bytes")),
            "{message}"
        );
    }
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
            let rejected = fs_guard::open_verified(&worker_path).is_err()
                && fs_guard::open_verified(&device).is_err();
            sender.send(rejected).ok();
        });
        let result = receiver.recv_timeout(Duration::from_secs(2));
        std::fs::remove_dir_all(dir).unwrap();
        assert!(result.unwrap());
    }
}
