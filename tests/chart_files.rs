#![cfg(unix)]

use clap::Parser;
use std::{
    os::unix::fs::symlink,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};
use tapline::{bms, bmson, cli, select};

#[test]
fn special_charts_are_rejected_without_blocking() {
    let dir = std::env::temp_dir().join(format!("tapline-chart-files-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    ["bms", "bmson"].iter().for_each(|ext| {
        let fifo = dir.join(format!("blocked.{ext}"));
        let name = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        symlink(&fifo, dir.join(format!("fifo-link.{ext}"))).unwrap();
        symlink("/dev/zero", dir.join(format!("endless.{ext}"))).unwrap();
        std::fs::create_dir(dir.join(format!("directory.{ext}"))).unwrap();
    });
    [("sample.bms", "bms"), ("minimal.bmson", "bmson")]
        .iter()
        .for_each(|(fixture, ext)| {
            let source = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures")
                .join(fixture);
            let target = dir.join(format!("regular.{ext}"));
            std::fs::copy(source, &target).unwrap();
            symlink(target, dir.join(format!("regular-link.{ext}"))).unwrap();
        });
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "chart_file_worker", "--nocapture"])
        .env("TAPLINE_CHART_TEST_DIR", &dir)
        .stdin(Stdio::null())
        .spawn()
        .unwrap();
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break Some(status);
        }
        if start.elapsed() > Duration::from_secs(5) {
            child.kill().unwrap();
            child.wait().unwrap();
            break None;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    std::fs::remove_dir_all(dir).unwrap();
    assert!(status.expect("chart reading timed out").success());
}

#[test]
fn chart_file_worker() {
    let Some(dir) = std::env::var_os("TAPLINE_CHART_TEST_DIR") else {
        return;
    };
    let dir = Path::new(&dir);
    ["bms", "bmson"].iter().for_each(|ext| {
        ["blocked", "fifo-link", "endless", "directory"]
            .iter()
            .for_each(|name| assert_rejected(&dir.join(format!("{name}.{ext}"))));
        ["regular", "regular-link"].iter().for_each(|name| {
            let path = dir.join(format!("{name}.{ext}"));
            let args = cli::Args::parse_from(["tapline", "--no-audio"]);
            let chart = cli::load_chart(&cli::ChartChoice::File(path), &args)
                .unwrap()
                .unwrap();
            assert!(!chart.notes.is_empty());
        });
    });
    let charts = select::scan(dir);
    assert_eq!(charts.len(), 4);
    assert!(charts.iter().all(|chart| chart
        .path
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .starts_with("regular")));
}

fn assert_rejected(path: &Path) {
    let error = if bmson::is_bmson(path) {
        bmson::read_meta(path).unwrap_err()
    } else {
        bms::read_meta(path).unwrap_err()
    };
    assert!(format!("{error:#}").contains("regular file"));
    let args = cli::Args::parse_from(["tapline", "--file", path.to_str().unwrap(), "--no-audio"]);
    let choice = cli::choose_chart(&mut std::io::stdout(), &args).unwrap();
    let error = cli::load_chart(&choice, &args)
        .err()
        .expect("special chart accepted");
    assert!(format!("{error:#}").contains("regular file"));
}
