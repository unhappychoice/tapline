#![cfg(unix)]

use std::fs::File;
use std::io::Write;
use std::os::fd::FromRawFd;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

#[test]
fn quitting_the_chart_selector_exits_successfully() {
    let songs = Path::new(env!("CARGO_MANIFEST_DIR")).join("songs");
    let (mut master, slave) = open_pty();
    master.write_all(b"q\n").unwrap();

    let output = run_selector(&songs, slave);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("T A P L I N E"), "{stdout:?}");
    assert!(stdout.contains("First Steps"), "{stdout:?}");
    assert!(stdout.contains("\x1b[?1049l"), "{stdout:?}");
}

fn run_selector(songs: &Path, slave: File) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_tapline"));
    command
        .arg("--dir")
        .arg(songs)
        .arg("--no-audio")
        .env("TAPLINE_RANDOM_SEED", "1")
        .env("TAPLINE_DATA_DIR", "/dev/null")
        .stdin(slave)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Give terminal size queries a PTY while retaining separate output streams.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 || libc::ioctl(0, libc::TIOCSCTTY as _, 0) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }

    let mut child = command.spawn().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            let _ = child.kill();
            let output = child.wait_with_output().unwrap();
            panic!("chart selector did not exit: {output:?}");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    child.wait_with_output().unwrap()
}

#[test]
fn missing_chart_reports_the_path_on_stderr() {
    let dir = std::env::temp_dir().join(format!("tapline-missing-chart-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let path = dir.join("missing.bmson");
    let output = run_chart(&path);
    std::fs::remove_dir(dir).unwrap();

    assert_chart_error(output, &format!("cannot read chart {}", path.display()));
}

#[test]
fn malformed_bmson_reports_the_path_on_stderr() {
    let path = std::env::temp_dir().join(format!("tapline-malformed-{}.bmson", std::process::id()));
    std::fs::write(&path, "{").unwrap();
    let output = run_chart(&path);
    std::fs::remove_file(&path).unwrap();

    assert_chart_error(output, &format!("Invalid bmson: {}", path.display()));
}

#[test]
fn unsupported_versions_are_sanitized_on_stderr() {
    assert_sanitized_error("version", "Unsupported bmson version");
}

#[test]
fn unsupported_modes_are_sanitized_on_stderr() {
    assert_sanitized_error("mode_hint", "Unsupported bmson mode");
}

fn assert_sanitized_error(field: &str, diagnostic: &str) {
    let controls: String = (0..=0x9f)
        .filter_map(char::from_u32)
        .filter(|c| c.is_control())
        .collect();
    let payload = format!("譜面 café\x1b]52;c;VEVTVA==\x07\x1b[31m\u{009b}31m{controls}");
    let mut chart = serde_json::json!({
        "version": "1.0.0",
        "info": {"title": "Test", "artist": "Test", "init_bpm": 120},
        "sound_channels": []
    });
    if field == "version" {
        chart[field] = payload.clone().into();
    } else {
        chart["info"][field] = payload.clone().into();
    }
    let path = std::env::temp_dir().join(format!(
        "tapline-stderr-{}-{field}.bmson",
        std::process::id()
    ));
    std::fs::write(&path, chart.to_string()).unwrap();
    let output = run_chart(&path);
    std::fs::remove_file(path).unwrap();
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8(output.stderr).unwrap();
    let sanitized: String = payload.chars().filter(|c| !c.is_control()).collect();
    assert_eq!(stderr, format!("Error: {diagnostic}: {sanitized}\n"));
    assert!(!stderr.trim_end_matches('\n').chars().any(char::is_control));
}

fn assert_chart_error(output: Output, diagnostic: &str) {
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(
        stderr.starts_with(&format!("Error: {diagnostic}: ")),
        "{stderr}"
    );
}

fn run_chart(path: &Path) -> Output {
    let (_master, slave) = open_pty();
    Command::new(env!("CARGO_BIN_EXE_tapline"))
        .arg("--file")
        .arg(path)
        .arg("--no-audio")
        .stdin(slave)
        .output()
        .unwrap()
}

fn open_pty() -> (File, File) {
    let (mut master, mut slave) = (-1, -1);
    let mut size = libc::winsize {
        ws_row: 24,
        ws_col: 100,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let result = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &raw mut size,
        )
    };
    assert_eq!(result, 0, "{}", std::io::Error::last_os_error());
    unsafe { (File::from_raw_fd(master), File::from_raw_fd(slave)) }
}
