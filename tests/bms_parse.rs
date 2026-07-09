use std::path::Path;

fn tapline_bin() -> &'static str { env!("CARGO_BIN_EXE_tapline") }

#[test]
fn help_lists_dir_flag() {
    let out = std::process::Command::new(tapline_bin())
        .arg("--help")
        .output()
        .expect("run tapline --help");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("--dir"), "expected --dir in help output: {}", stdout);
    assert!(stdout.contains("--file"), "expected --file in help output: {}", stdout);
}

#[test]
fn sample_charts_have_expected_shape() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let expected = [
        ("songs/01-first-steps.bms",     "First Steps",      4, 1u8, 1u8),
        ("songs/02-steady-rain.bms",     "Steady Rain",      5, 4,   2),
        ("songs/03-rolling-thunder.bms", "Rolling Thunder",  7, 8,   3),
        ("songs/04-neon-drive.bms",      "Neon Drive",       4, 3,   2),
        ("songs/05-circuit-breaker.bms", "Circuit Breaker",  5, 7,   3),
        ("songs/06-blue-hour.bms",       "Blue Hour",        7, 5,   2),
        ("songs/07-final-cascade.bms",   "Final Cascade",    7, 11,  4),
        ("songs/08-arcade-hero.bms",     "Arcade Hero",      4, 7,   3),
    ];
    for (rel, title, lanes, level, difficulty) in expected {
        let text = std::fs::read_to_string(root.join(rel))
            .unwrap_or_else(|_| panic!("missing {}", rel));
        assert!(text.contains(&format!("#TITLE {}", title)),  "{}: title mismatch", rel);
        assert!(text.contains(&format!("#PLAYLEVEL {}", level)),     "{}: level mismatch", rel);
        assert!(text.contains(&format!("#DIFFICULTY {}", difficulty)),"{}: difficulty mismatch", rel);
        let highest_channel = match lanes {
            4 => "14", 5 => "15", 7 => "19", _ => panic!("unexpected lanes"),
        };
        assert!(text.contains(highest_channel), "{}: expected a channel {} line", rel, highest_channel);
    }
}
