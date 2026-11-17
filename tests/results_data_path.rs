use std::{path::PathBuf, process::Command};

#[test]
fn explicit_data_directory_overrides_platform_defaults() {
    assert_data_path(
        &[
            ("TAPLINE_DATA_DIR", "custom-data"),
            ("HOME", "home"),
            ("XDG_DATA_HOME", "xdg"),
            ("LOCALAPPDATA", "local"),
        ],
        Some(PathBuf::from("custom-data/results.json")),
    );
}

#[test]
fn missing_environment_has_no_data_directory() {
    assert_data_path(&[], None);
}

#[cfg(target_os = "windows")]
#[test]
fn windows_uses_local_app_data() {
    assert_data_path(
        &[("LOCALAPPDATA", "local"), ("HOME", "home")],
        Some(PathBuf::from("local/tapline/results.json")),
    );
}

#[cfg(target_os = "macos")]
#[test]
fn macos_uses_application_support() {
    assert_data_path(
        &[("HOME", "home"), ("XDG_DATA_HOME", "xdg")],
        Some(PathBuf::from(
            "home/Library/Application Support/tapline/results.json",
        )),
    );
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
#[test]
fn absolute_xdg_directory_takes_precedence_over_home() {
    let xdg = std::env::current_dir().unwrap().join("xdg-data");
    assert_data_path(
        &[("XDG_DATA_HOME", xdg.to_str().unwrap()), ("HOME", "home")],
        Some(xdg.join("tapline/results.json")),
    );
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
#[test]
fn missing_or_relative_xdg_directory_falls_back_to_home() {
    [None, Some("relative-data")].iter().for_each(|xdg| {
        let mut environment = vec![("HOME", "home")];
        environment.extend(xdg.map(|value| ("XDG_DATA_HOME", value)));
        assert_data_path(
            &environment,
            Some(PathBuf::from("home/.local/share/tapline/results.json")),
        );
    });
    assert_data_path(&[("XDG_DATA_HOME", "relative-data")], None);
}

#[test]
fn data_path_probe() {
    let Ok(expected) = std::env::var("TAPLINE_EXPECTED_DATA_PATH") else {
        return;
    };
    let expected: Option<PathBuf> = serde_json::from_str(&expected).unwrap();
    assert_eq!(tapline::results::data_path(), expected);
}

fn assert_data_path(environment: &[(&str, &str)], expected: Option<PathBuf>) {
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "data_path_probe", "--nocapture"])
        .env_remove("TAPLINE_DATA_DIR")
        .env_remove("HOME")
        .env_remove("XDG_DATA_HOME")
        .env_remove("LOCALAPPDATA")
        .envs(environment.iter().copied())
        .env(
            "TAPLINE_EXPECTED_DATA_PATH",
            serde_json::to_string(&expected).unwrap(),
        )
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "data path probe failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
