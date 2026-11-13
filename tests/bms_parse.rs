use std::path::Path;

fn tapline_bin() -> &'static str {
    env!("CARGO_BIN_EXE_tapline")
}

#[test]
fn help_lists_dir_flag() {
    let out = std::process::Command::new(tapline_bin())
        .arg("--help")
        .output()
        .expect("run tapline --help");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("--dir"),
        "expected --dir in help output: {}",
        stdout
    );
    assert!(
        stdout.contains("--file"),
        "expected --file in help output: {}",
        stdout
    );
}

#[test]
fn sample_charts_have_expected_shape() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let expected = [
        ("songs/01-first-steps.bmson", "First Steps", 4, 1u8, 1u8),
        ("songs/02-steady-rain.bms", "Steady Rain", 5, 4, 2),
        ("songs/03-rolling-thunder.bms", "Rolling Thunder", 7, 8, 3),
        ("songs/04-neon-drive.bmson", "Neon Drive", 4, 3, 2),
        ("songs/05-circuit-breaker.bms", "Circuit Breaker", 5, 7, 3),
        ("songs/06-blue-hour.bms", "Blue Hour", 7, 5, 2),
        ("songs/07-final-cascade.bms", "Final Cascade", 7, 11, 4),
        ("songs/08-arcade-hero.bmson", "Arcade Hero", 4, 7, 3),
    ];
    for (rel, title, lanes, level, difficulty) in expected {
        let path = root.join(rel);
        let meta = if tapline::bmson::is_bmson(&path) {
            tapline::bmson::read_meta(&path).unwrap()
        } else {
            tapline::bms::read_meta(&path).unwrap()
        };
        assert_eq!(meta.title, title, "{rel}: title");
        assert_eq!(meta.playlevel, Some(level), "{rel}: level");
        assert_eq!(meta.difficulty, Some(difficulty), "{rel}: difficulty");
        assert_eq!(meta.lane_count, lanes, "{rel}: lanes");
    }
}
