use super::*;
use cli::ChartChoice;

#[test]
fn explicit_built_in_retries_then_exits() {
    assert_retry_then_exit(&Args::parse_from(["tapline", "--built-in", "--no-audio"]));
}

#[test]
fn empty_directory_fallback_retries_then_exits() {
    let dir = std::env::temp_dir().join(format!("tapline-main-test-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let args = Args::parse_from(["tapline", "--dir", dir.to_str().unwrap(), "--no-audio"]);
    assert_retry_then_exit(&args);
    std::fs::remove_dir(dir).unwrap();
}

#[test]
fn explicit_file_retries_then_exits() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/sample.bms");
    assert_retry_then_exit(&Args::parse_from(["tapline", "--file", path, "--no-audio"]));
}

#[test]
fn selected_chart_retries_the_same_random_pattern_before_returning_to_selector() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let entry = tapline::select::scan(&root)
        .into_iter()
        .find(|entry| entry.path == root.join("random.bms"))
        .unwrap();
    let seed = entry.random_seed;
    let mut choices = [
        ChartChoice::Selected(Box::new(entry)),
        ChartChoice::Cancelled,
    ]
    .into_iter();
    let mut selections = 0;
    let mut attempts = Vec::new();
    run_session(
        &mut stdout(),
        &Args::parse_from(["tapline", "--no-audio"]),
        &cli::Keymap::default(),
        |_, _, _| {
            selections += 1;
            Ok(choices.next().expect("unexpected selector restart"))
        },
        |_, _, chart, _, identity, destination, _| {
            assert_eq!(destination, ResultDestination::Selector);
            let identity = identity.unwrap();
            assert_eq!(identity.random_seed, Some(seed));
            let note = chart.notes[0];
            attempts.push((identity.key.clone(), note.time_ms, note.lane, note.keysound));
            Ok(match attempts.len() {
                1 => runtime::ResultAction::Retry,
                2 => runtime::ResultAction::Dismiss,
                _ => panic!("unexpected retry"),
            })
        },
        |_, error| panic!("unexpected load error: {error}"),
    )
    .unwrap();
    assert_eq!(selections, 2);
    assert_eq!(attempts.len(), 2);
    assert_eq!(attempts[0], attempts[1]);
}

#[test]
fn replaced_chart_requires_a_new_selection_before_playback() {
    let dir = std::env::temp_dir().join(format!("tapline-stale-selection-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let path = dir.join("chart.bms");
    std::fs::write(&path, "#TITLE Original\n#00111:01\n").unwrap();

    let mut selections = 0;
    let mut played = Vec::new();
    run_session(
        &mut stdout(),
        &Args::parse_from(["tapline", "--no-audio"]),
        &cli::Keymap::default(),
        |_, _, state| {
            selections += 1;
            let entry = match selections {
                1 => {
                    let entry = state.scan(&dir).remove(0);
                    assert_eq!(entry.title, "Original");
                    std::fs::write(&path, "#TITLE Replacement\n#00111:01\n").unwrap();
                    entry
                }
                2 => {
                    let entry = state.scan(&dir).remove(0);
                    assert_eq!(entry.title, "Replacement");
                    entry
                }
                3 => return Ok(ChartChoice::Cancelled),
                _ => panic!("unexpected selector restart"),
            };
            Ok(ChartChoice::Selected(Box::new(entry)))
        },
        |_, _, chart, _, _, destination, _| {
            assert_eq!(destination, ResultDestination::Selector);
            played.push(chart.title.clone());
            Ok(runtime::ResultAction::Dismiss)
        },
        |_, error| panic!("unexpected load error: {error}"),
    )
    .unwrap();
    assert_eq!(selections, 3);
    assert_eq!(played, ["Replacement"]);
    std::fs::remove_dir_all(dir).unwrap();
}

fn assert_retry_then_exit(args: &Args) {
    let mut selections = 0;
    let mut plays = 0;
    run_session(
        &mut stdout(),
        args,
        &cli::Keymap::default(),
        |out, args, state| {
            selections += 1;
            assert_eq!(selections, 1, "chart selection ran again");
            cli::choose_chart_with_selector(out, args, state)
        },
        |_, _, _, _, _, destination, _| {
            assert_eq!(destination, ResultDestination::Exit);
            plays += 1;
            Ok(match plays {
                1 => runtime::ResultAction::Retry,
                2 => runtime::ResultAction::Dismiss,
                _ => panic!("unexpected retry"),
            })
        },
        |_, error| panic!("unexpected load error: {error}"),
    )
    .unwrap();
    assert_eq!(plays, 2);
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
        .map(|entry| ChartChoice::Selected(Box::new(entry)))
        .chain(std::iter::once(ChartChoice::Cancelled));
    let mut selections = 0;
    let mut plays = 0;
    let mut errors = Vec::new();
    run_session(
        &mut stdout(),
        &Args::parse_from(["tapline", "--no-audio"]),
        &cli::Keymap::default(),
        |_, _, _| {
            selections += 1;
            Ok(choices.next().expect("unexpected selector restart"))
        },
        |_, _, chart, _, _, destination, _| {
            assert_eq!(destination, ResultDestination::Selector);
            assert_eq!(chart.title, "Playable");
            plays += 1;
            Ok(runtime::ResultAction::Dismiss)
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
        &cli::Keymap::default(),
        cli::choose_chart_with_selector,
        |_, _, _, _, _, _, _| panic!("invalid chart must not play"),
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
