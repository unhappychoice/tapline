#![cfg(unix)]

use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Output};
use std::time::{Duration, Instant};

#[test]
fn quitting_the_chart_selector_exits_successfully() {
    let songs = Path::new(env!("CARGO_MANIFEST_DIR")).join("songs");
    let (status, output) = run_selector(&songs);
    assert_eq!(status.code(), Some(0), "{output:?}");
    assert!(output.contains("T A P L I N E"), "{output:?}");
    assert!(output.contains("First Steps"), "{output:?}");
    assert!(output.contains("\x1b[?1049l"), "{output:?}");
}

fn run_selector(songs: &Path) -> (ExitStatus, String) {
    let (mut master, slave) = open_pty();
    let result = unsafe { libc::fcntl(master.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK) };
    assert_eq!(result, 0, "{}", io::Error::last_os_error());
    let mut command = Command::new(env!("CARGO_BIN_EXE_tapline"));
    command
        .arg("--dir")
        .arg(songs)
        .arg("--no-audio")
        .env("TAPLINE_RANDOM_SEED", "1")
        .env("TAPLINE_DATA_DIR", "/dev/null")
        .stdin(slave.try_clone().unwrap())
        .stdout(slave.try_clone().unwrap())
        .stderr(slave.try_clone().unwrap());
    // Let terminal size queries fall back to stdout even when run from a terminal.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }

    let mut child = command.spawn().unwrap();
    let mut output = Vec::new();
    let result = wait_for_selector(&mut child, &mut master, &mut output);
    if result.is_err() {
        let _ = child.kill();
        child.wait().unwrap();
    }
    // Keep the slave open while reading the last bytes after child exit.
    drop(slave);
    let output = String::from_utf8_lossy(&output).into_owned();
    let status = result.unwrap_or_else(|err| panic!("chart selector: {err}; output: {output:?}"));
    (status, output)
}

fn wait_for_selector(
    child: &mut Child,
    master: &mut File,
    output: &mut Vec<u8>,
) -> io::Result<ExitStatus> {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut sent_quit = false;
    let mut buffer = [0; 4096];
    loop {
        if Instant::now() >= deadline {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "did not exit"));
        }
        let status = child.try_wait()?;
        match master.read(&mut buffer) {
            Ok(n) if n > 0 => {
                output.extend_from_slice(&buffer[..n]);
                // The first frame confirms that the child has enabled raw mode.
                if !sent_quit
                    && status.is_none()
                    && output
                        .windows(b"First Steps".len())
                        .any(|w| w == b"First Steps")
                {
                    master.write_all(b"q")?;
                    sent_quit = true;
                }
            }
            Ok(_) => return Err(io::Error::from(io::ErrorKind::UnexpectedEof)),
            Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                if let Some(status) = status {
                    if status.success() && !sent_quit {
                        return Err(io::Error::other("exited before receiving q"));
                    }
                    return Ok(status);
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(err) => return Err(err),
        }
    }
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
    let (master, slave) = unsafe { (File::from_raw_fd(master), File::from_raw_fd(slave)) };
    for file in [&master, &slave] {
        let result = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) };
        assert_eq!(result, 0, "{}", io::Error::last_os_error());
    }
    (master, slave)
}
