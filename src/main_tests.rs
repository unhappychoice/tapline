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
        |_, _, _| {
            plays += 1;
            Ok(())
        },
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
        |_, _, _| {
            plays += 1;
            // Both completion and interruption return Ok after closing results.
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(plays, 1);
}
