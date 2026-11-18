#![cfg(unix)]

use std::fs::File;
use std::os::fd::FromRawFd;
use std::path::Path;
use std::process::{Command, Output};

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
    let result = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    assert_eq!(result, 0, "{}", std::io::Error::last_os_error());
    unsafe { (File::from_raw_fd(master), File::from_raw_fd(slave)) }
}
