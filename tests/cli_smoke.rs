use std::path::PathBuf;
use std::process::Command;

fn tapline_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_tapline"))
}

fn run_with(args: &[&str]) -> (i32, String, String) {
    let out = Command::new(tapline_bin())
        .args(args)
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

#[test]
fn cli_exits_nonzero_when_terminal_is_not_a_tty() {
    // `cargo test` has no PTY attached, so raw-mode setup fails before we
    // reach the game loop. This guards against regressions where the binary
    // silently returns 0 on a hard error.
    let (code, _, stderr) = run_with(&["--built-in"]);
    assert_ne!(code, 0, "no-TTY invocation should not succeed silently");
    assert!(
        !stderr.is_empty(),
        "expected some diagnostic on stderr, got empty"
    );
}
