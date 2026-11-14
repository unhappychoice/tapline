use super::*;
use cli::ChartChoice;

#[test]
fn explicit_built_in_exits_after_playback() {
    assert_single_play(&Args::parse_from(["tapline", "--built-in", "--no-audio"]));
}

#[test]
fn empty_directory_fallback_exits_after_playback() {
    let dir = std::env::temp_dir().join(format!("tapline-main-test-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let args = Args::parse_from(["tapline", "--dir", dir.to_str().unwrap(), "--no-audio"]);
    assert_single_play(&args);
    std::fs::remove_dir(dir).unwrap();
}

#[test]
fn explicit_file_exits_after_playback() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/sample.bms");
    assert_single_play(&Args::parse_from(["tapline", "--file", path, "--no-audio"]));
}

#[test]
fn selected_chart_returns_to_selector_after_playback() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let meta = tapline::select::scan(&root).remove(0);
    let mut choices = [
        ChartChoice::Selected(Box::new(meta)),
        ChartChoice::Cancelled,
    ]
    .into_iter();
    let mut selections = 0;
    let mut plays = 0;
    run_session(
        &mut stdout(),
        &Args::parse_from(["tapline", "--no-audio"]),
        |_, _| {
            selections += 1;
            Ok(choices.next().expect("unexpected selector restart"))
        },
        |_, _, _, _| {
            plays += 1;
            Ok(())
        },
        |_, error| panic!("unexpected load error: {error}"),
    )
    .unwrap();
    assert_eq!(selections, 2);
    assert_eq!(plays, 1);
}

fn assert_single_play(args: &Args) {
    let mut selections = 0;
    let mut plays = 0;
    run_session(
        &mut stdout(),
        args,
        |out, args| {
            selections += 1;
            assert_eq!(selections, 1, "chart restarted after results closed");
            cli::choose_chart(out, args)
        },
        |_, _, _, _| {
            plays += 1;
            // Both completion and interruption return Ok after closing results.
            Ok(())
        },
        |_, error| panic!("unexpected load error: {error}"),
    )
    .unwrap();
    assert_eq!(plays, 1);
}

#[test]
fn failed_selection_returns_to_selector_then_plays_and_cancels() {
    let dir = broken_chart_directory("selection");
    std::fs::write(
        dir.join("playable.bms"),
        "#TITLE Playable\n#BPM 120\n#00111:01\n",
    )
    .unwrap();
    let charts = tapline::select::scan(&dir);
    assert_eq!(charts.len(), 2);
    let mut choices = charts
        .into_iter()
        .map(|meta| ChartChoice::Selected(Box::new(meta)))
        .chain(std::iter::once(ChartChoice::Cancelled));
    let mut selections = 0;
    let mut plays = 0;
    let mut errors = Vec::new();
    run_session(
        &mut stdout(),
        &Args::parse_from(["tapline", "--no-audio"]),
        |_, _| {
            selections += 1;
            Ok(choices.next().expect("unexpected selector restart"))
        },
        |_, _, chart, _| {
            assert_eq!(chart.title, "Playable");
            plays += 1;
            Ok(())
        },
        |_, error| {
            errors.push(error.to_string());
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(selections, 3);
    assert_eq!(plays, 1);
    assert_eq!(errors, ["Layered notes have unequal lengths"]);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn explicit_invalid_file_returns_error_without_playback_or_recovery() {
    let dir = broken_chart_directory("explicit");
    let path = dir.join("broken.bmson");
    let args = Args::parse_from(["tapline", "--file", path.to_str().unwrap(), "--no-audio"]);
    let error = run_session(
        &mut stdout(),
        &args,
        cli::choose_chart,
        |_, _, _, _| panic!("invalid chart must not play"),
        |_, _| panic!("explicit file must not recover"),
    )
    .unwrap_err();
    assert_eq!(error.to_string(), "Layered notes have unequal lengths");
    std::fs::remove_dir_all(dir).unwrap();
}

fn broken_chart_directory(name: &str) -> std::path::PathBuf {
    let dir =
        std::env::temp_dir().join(format!("tapline-load-error-{name}-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("broken.bmson"), r#"{"version":"1.0.0","info":{"title":"Broken","artist":"test","init_bpm":120,"mode_hint":"generic-4keys"},"sound_channels":[{"name":"first.wav","notes":[{"x":1,"y":0}]},{"name":"second.wav","notes":[{"x":1,"y":0,"l":240}]}]}"#).unwrap();
    dir
}
