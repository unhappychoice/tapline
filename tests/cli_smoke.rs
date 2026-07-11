use std::path::PathBuf;
use std::process::Command;

fn tapline_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_tapline"))
}

fn run_with(args: &[&str]) -> (i32, String, String) {
    let out = Command::new(tapline_bin())
        .args(args)
        .env("TAPLINE_TEST_TONE_QUIET", "1")
        .output()
        .expect("failed to spawn tapline binary");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn version_flag_prints_the_crate_version() {
    let (code, stdout, _) = run_with(&["--version"]);
    assert_eq!(code, 0);
    assert!(
        stdout.contains("tapline "),
        "expected package name in --version output: {}",
        stdout
    );
    assert!(
        stdout.contains(env!("CARGO_PKG_VERSION")),
        "expected {} in --version output: {}",
        env!("CARGO_PKG_VERSION"),
        stdout
    );
}

#[test]
fn help_flag_documents_every_public_flag() {
    let (code, stdout, _) = run_with(&["--help"]);
    assert_eq!(code, 0);
    for flag in [
        "--file",
        "--dir",
        "--bpm",
        "--countdown-ms",
        "--built-in",
        "--no-audio",
        "--test-tone",
        "--synth",
        "--auto-ks",
        "--audio-lead-ms",
    ] {
        assert!(
            stdout.contains(flag),
            "expected {} in --help output:\n{}",
            flag,
            stdout
        );
    }
}

#[test]
fn help_short_form_is_wired() {
    let (code, stdout, _) = run_with(&["-h"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("--file"), "short help should print flags");
}

#[test]
fn unknown_flag_fails_with_a_helpful_stderr() {
    let (code, _, stderr) = run_with(&["--not-a-real-flag"]);
    assert_ne!(code, 0);
    assert!(
        stderr.contains("--not-a-real-flag") || stderr.contains("unexpected"),
        "clap should call out the unknown flag: {}",
        stderr
    );
}

// `--built-in` spawns the game loop and, on any platform where crossterm's
// raw-mode setup does succeed without a PTY (Windows Actions runners are
// one), it blocks in the result screen's `press any key to exit` loop. The
// Linux/macOS runners fail at raw-mode setup and exit fast, but Windows
// would hang indefinitely, so we don't spawn the interactive path here.

#[test]
fn test_tone_flag_exits_cleanly_on_ci_without_audio_backend() {
    // On CI runners without a real sound card, `--test-tone` reports
    // "audio backend unavailable" on stderr and exits with code 1. On a
    // developer machine with speakers wired up the same call plays four
    // beeps and exits 0 after ~1.3s.
    let (code, _, stderr) = run_with(&["--test-tone"]);
    assert!(
        code == 0 || code == 1,
        "expected exit code 0 or 1 from --test-tone, got {}",
        code
    );
    if code == 1 {
        assert!(
            stderr.contains("audio backend unavailable"),
            "expected audio-unavailable diagnostic on stderr, got: {}",
            stderr
        );
    }
}
