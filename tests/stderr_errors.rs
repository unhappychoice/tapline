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
    let mut command = Command::new(env!("CARGO_BIN_EXE_tapline"));
    command
        .arg("--dir")
        .arg(songs)
        .env("TAPLINE_DATA_DIR", "/dev/null");
    let (status, output) = run_interactive(command, &[(b"First Steps", b"q")]);
    assert_eq!(status.code(), Some(0), "{output:?}");
    assert!(output.contains("T A P L I N E"), "{output:?}");
    assert!(output.contains("First Steps"), "{output:?}");
    assert!(output.contains("\x1b[?1049l"), "{output:?}");
}

#[test]
fn built_in_practice_retries_from_interrupted_results() {
    let data = std::env::temp_dir().join(format!("tapline-practice-{}", std::process::id()));
    std::fs::create_dir(&data).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_tapline"));
    command
        .args(["--built-in", "--countdown-ms", "0", "--bpm", "60"])
        .env("TAPLINE_DATA_DIR", &data);
    let (status, output) = run_interactive(
        command,
        &[
            (b"quit Esc / Q", b"q"),
            (b"R: retry / other: exit", b"r"),
            (b"quit Esc / Q", b"q"),
            (b"R: retry / other: exit", b" "),
        ],
    );
    let saved = data.join("results.json").exists();
    std::fs::remove_dir_all(data).unwrap();
    assert_eq!(status.code(), Some(0), "{output:?}");
    assert!(output.contains("Built-in Practice"), "{output:?}");
    assert!(output.contains("═"), "{output:?}");
    assert_eq!(output.matches("Interrupted: not saved").count(), 2);
    assert!(!saved);
    assert!(output.contains("\x1b[?1049l"), "{output:?}");
}

#[test]
fn keymap_overrides_the_built_in_gameplay_labels_on_retry() {
    let mut command = Command::new(env!("CARGO_BIN_EXE_tapline"));
    command.args([
        "--built-in",
        "--countdown-ms",
        "0",
        "--keymap",
        concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/keymap.json"),
    ]);
    let (status, output) = run_interactive(
        command,
        &[
            (b"keys U I O P", b"q"),
            (b"R: retry / other: exit", b"r"),
            (b"keys U I O P", b"q"),
            (b"R: retry / other: exit", b" "),
        ],
    );
    assert_eq!(status.code(), Some(0), "{output:?}");
    assert_eq!(output.matches("Interrupted: not saved").count(), 2);
}

#[test]
fn completed_random_chart_retry_compares_with_the_saved_first_attempt() {
    use tapline::results::{Identity, Store};

    let dir = std::env::temp_dir().join(format!("tapline-retry-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let path = dir.join("retry.bms");
    std::fs::write(
        &path,
        "#BPM 6000\n#RANDOM 2\n#IF 1\n#00011:01\n#ELSE\n#00019:02\n#ENDIF\n#ENDRANDOM\n",
    )
    .unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_tapline"));
    command
        .args(["--file", path.to_str().unwrap(), "--countdown-ms", "100"])
        .env("TAPLINE_DATA_DIR", &dir);
    let prompt: &[u8] = b"R: retry / other: exit";
    let (status, output) = run_interactive(command, &[(prompt, b"r"), (prompt, b" ")]);
    let identity = Identity::file(&path, None).unwrap();
    let store = Store::load(&dir.join("results.json")).unwrap();
    std::fs::remove_dir_all(dir).unwrap();

    assert_eq!(status.code(), Some(0), "{output:?}");
    assert_eq!(output.matches("First completed play!").count(), 1);
    assert_eq!(output.matches("vs best: score +0").count(), 1);
    assert_eq!(output.matches("miss 1").count(), 2);
    assert_eq!(store.get(&identity).unwrap().latest.random_seed, Some(1));
}

#[test]
fn replaced_chart_requires_reselection_in_the_terminal() {
    let dir = std::env::temp_dir().join(format!("tapline-reselect-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let path = dir.join("chart.bms");
    std::fs::write(&path, "#TITLE Original\n#00111:01\n").unwrap();

    let mut command = Command::new(env!("CARGO_BIN_EXE_tapline"));
    command
        .args(["--countdown-ms", "0", "--dir"])
        .arg(&dir)
        .env("TAPLINE_DATA_DIR", "/dev/null");
    let (status, output) = run_interactive_with(
        command,
        &[
            (b"Original", b"\r"),
            (b"Replacement", b"\r"),
            (b"quit Esc / Q", b"q"),
            (b"R: retry / other: selector", b" "),
            (b"Replacement", b"q"),
        ],
        |prompt| {
            if prompt == b"Original" {
                std::fs::write(&path, "#TITLE Replacement\n#00111:01\n")?;
            }
            Ok(())
        },
    );
    std::fs::remove_dir_all(dir).unwrap();

    assert!(status.success(), "{output:?}");
    let (before_replacement, _) = output.split_once("Replacement").unwrap();
    let (_, after_original) = before_replacement.split_once("Original").unwrap();
    assert!(after_original.contains("select a chart"), "{output:?}");
    assert!(!before_replacement.contains("quit Esc / Q"), "{output:?}");
    assert!(
        !output.contains("Press any key to return to the selector."),
        "{output:?}"
    );
}

#[test]
fn oversized_replacement_returns_to_the_empty_selector() {
    let dir = std::env::temp_dir().join(format!(
        "tapline-oversized-selection-{}",
        std::process::id()
    ));
    std::fs::create_dir(&dir).unwrap();
    let path = dir.join("chart.bms");
    std::fs::write(&path, "#TITLE Original\n#00111:01\n").unwrap();

    let mut command = Command::new(env!("CARGO_BIN_EXE_tapline"));
    command
        .arg("--dir")
        .arg(&dir)
        .env("TAPLINE_DATA_DIR", "/dev/null");
    let (status, output) = run_interactive_with(
        command,
        &[
            (b"Original", b"\r"),
            (b"Press any key to return to the selector.", b" "),
            ("select a chart  ·  0 loaded".as_bytes(), b"q"),
        ],
        |prompt| {
            if prompt == b"Original" {
                File::options()
                    .write(true)
                    .open(&path)?
                    .set_len(16 * 1024 * 1024 + 1)?;
            }
            Ok(())
        },
    );
    std::fs::remove_dir_all(dir).unwrap();

    assert_eq!(status.code(), Some(0), "{output:?}");
    assert!(
        output.contains("chart exceeds the maximum size"),
        "{output:?}"
    );
    assert!(output.contains("select a chart  ·  0 loaded"), "{output:?}");
}

#[test]
fn dismissing_results_reopens_the_selector_after_the_last_chart_is_deleted() {
    let dir = std::env::temp_dir().join(format!("tapline-deleted-chart-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let path = dir.join("chart.bms");
    std::fs::write(&path, "#TITLE Last Chart\n#00111:01\n").unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_tapline"));
    command
        .args(["--countdown-ms", "0"])
        .arg("--dir")
        .arg(&dir)
        .env("TAPLINE_DATA_DIR", "/dev/null");

    let (status, output) = run_interactive_with(
        command,
        &[
            (b"Last Chart", b"\r"),
            (b"quit Esc / Q", b"q"),
            (b"R: retry / other: selector", b" "),
            ("select a chart  ·  0 loaded".as_bytes(), b"\x1b"),
        ],
        |prompt| {
            if prompt == b"quit Esc / Q" {
                std::fs::remove_file(&path)?;
            }
            Ok(())
        },
    );
    std::fs::remove_dir(&dir).unwrap();
    assert!(status.success(), "{output:?}");
    assert!(!output.contains("Built-in Practice"), "{output:?}");
}

#[test]
fn starting_with_an_empty_directory_plays_built_in_practice() {
    let dir = std::env::temp_dir().join(format!("tapline-empty-selector-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_tapline"));
    command
        .args(["--countdown-ms", "0"])
        .arg("--dir")
        .arg(&dir)
        .env("TAPLINE_DATA_DIR", "/dev/null");
    let (status, output) = run_interactive(
        command,
        &[(b"quit Esc / Q", b"q"), (b"R: retry / other: exit", b" ")],
    );
    std::fs::remove_dir(&dir).unwrap();
    assert!(status.success(), "{output:?}");
    assert!(output.contains("Built-in Practice"), "{output:?}");
}

fn run_interactive(command: Command, steps: &[(&[u8], &[u8])]) -> (ExitStatus, String) {
    run_interactive_with(command, steps, |_| Ok(()))
}

fn run_interactive_with(
    mut command: Command,
    steps: &[(&[u8], &[u8])],
    before_input: impl FnMut(&[u8]) -> io::Result<()>,
) -> (ExitStatus, String) {
    let (mut master, slave) = open_pty();
    let result = unsafe { libc::fcntl(master.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK) };
    assert_eq!(result, 0, "{}", io::Error::last_os_error());
    command
        .arg("--no-audio")
        .env("TAPLINE_RANDOM_SEED", "1")
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
    let result = drive_terminal(&mut child, &mut master, &mut output, steps, before_input);
    if result.is_err() {
        let _ = child.kill();
        child.wait().unwrap();
    }
    // Keep the slave open while reading the last bytes after child exit.
    drop(slave);
    let output = String::from_utf8_lossy(&output).into_owned();
    let status =
        result.unwrap_or_else(|err| panic!("interactive tapline: {err}; output: {output:?}"));
    (status, output)
}

fn drive_terminal(
    child: &mut Child,
    master: &mut File,
    output: &mut Vec<u8>,
    mut steps: &[(&[u8], &[u8])],
    mut before_input: impl FnMut(&[u8]) -> io::Result<()>,
) -> io::Result<ExitStatus> {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut search_from = 0;
    let mut buffer = [0; 4096];
    loop {
        if Instant::now() >= deadline {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "did not exit"));
        }
        let status = child.try_wait()?;
        match master.read(&mut buffer) {
            Ok(n) if n > 0 => {
                output.extend_from_slice(&buffer[..n]);
                if let Some(((prompt, input), remaining)) = steps.split_first() {
                    if status.is_none()
                        && output[search_from..]
                            .windows(prompt.len())
                            .any(|w| w == *prompt)
                    {
                        before_input(prompt)?;
                        master.write_all(input)?;
                        steps = remaining;
                        search_from = output.len();
                    }
                }
            }
            Ok(_) => return Err(io::Error::from(io::ErrorKind::UnexpectedEof)),
            Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                if let Some(status) = status {
                    if status.success() && !steps.is_empty() {
                        return Err(io::Error::other("exited before receiving all input"));
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
fn invalid_bms_bpm_reports_the_reason_on_stderr() {
    let path = std::env::temp_dir().join(format!("tapline-invalid-bpm-{}.bms", std::process::id()));
    for (bpm, diagnostic) in [
        ("invalid", "Invalid BMS BPM"),
        ("0", "BMS BPM must be finite and greater than zero"),
    ] {
        std::fs::write(&path, format!("#BPM {bpm}\n#00111:01\n")).unwrap();
        let output = run_chart(&path);
        std::fs::remove_file(&path).unwrap();

        let stderr = String::from_utf8(output.stderr).unwrap();
        assert_eq!(output.status.code(), Some(1), "{stderr}");
        assert!(
            stderr.starts_with(&format!("Error: {diagnostic}")),
            "{stderr}"
        );
    }
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
