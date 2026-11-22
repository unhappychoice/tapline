use super::*;
use crate::cli::{load_session, Args, ChartChoice};
use crate::game::Game;
use crate::results::Store;
use clap::Parser;
use std::cell::Cell;

thread_local! {
    static READS: Cell<usize> = const { Cell::new(0) };
    static PARSES: Cell<usize> = const { Cell::new(0) };
}

pub(crate) fn count_content_read() {
    READS.with(|count| count.set(count.get() + 1));
}

pub(crate) fn count_metadata_parse() {
    PARSES.with(|count| count.set(count.get() + 1));
}

fn counts() -> (usize, usize) {
    (READS.get(), PARSES.get())
}

#[test]
fn visits_reuse_unchanged_metadata_and_refresh_library() {
    let dir = super::super::tests::tempdir();
    let bms = dir.join("static.bms");
    let bmson = dir.join("static.bmson");
    let preview = dir.join("preview.ogg");
    std::fs::write(
        &bms,
        "#RANDOM 1\n#IF 1\n#TITLE Static\n#PREVIEW preview.ogg\n#ENDIF\n#00111:01\n",
    )
    .unwrap();
    std::fs::copy("tests/fixtures/minimal.bmson", &bmson).unwrap();
    let mut state = crate::select::SelectorState::default();

    let charts = state.scan(&dir);
    assert_eq!(charts.len(), 2);
    let records = dir.join("records.json");
    assert_eq!(progress(&charts, &records), ["[UNPLAYED]", "[UNPLAYED]"]);
    assert_eq!(counts(), (2, 2));
    let selected = charts.into_iter().find(|meta| meta.path == bms).unwrap();
    save_result(selected, &records);
    assert_eq!(counts(), (3, 2));
    std::fs::write(&preview, []).unwrap();
    let charts = state.scan(&dir);
    assert_eq!(charts.len(), 2);
    assert_eq!(
        charts.iter().find(|meta| meta.path == bms).unwrap().preview,
        Some(preview)
    );
    assert!(progress(&charts, &records)
        .iter()
        .any(|row| row.starts_with("[BEST ")));
    assert_eq!(counts(), (3, 2));

    let added = dir.join("added.bms");
    std::fs::write(&added, "#TITLE Added").unwrap();
    std::fs::write(&bms, "#TITLE Updated chart").unwrap();
    std::fs::remove_file(&bmson).unwrap();
    let charts = state.scan(&dir);
    let titles: Vec<_> = charts.iter().map(|meta| meta.title.as_str()).collect();
    assert_eq!(titles, ["Added", "Updated chart"]);
    assert_eq!(progress(&charts, &records), ["[UNPLAYED]", "[UNPLAYED]"]);
    assert_eq!(counts(), (5, 4));
    assert!(!state.discovery.metadata.contains_key(&bmson));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn bmson_preview_is_resolved_relative_to_the_chart_on_each_visit() {
    let dir = super::super::tests::tempdir();
    let song = dir.join("song");
    std::fs::create_dir(&song).unwrap();
    let chart = song.join("chart.bmson");
    let preview = song.join("tone.wav");
    let mut value: serde_json::Value =
        serde_json::from_str(include_str!("../../../tests/fixtures/minimal.bmson")).unwrap();
    value["info"]["preview_music"] = serde_json::json!("tone.wav");
    std::fs::write(&chart, value.to_string()).unwrap();
    let mut state = crate::select::SelectorState::default();

    std::fs::copy("tests/fixtures/tone.wav", dir.join("tone.wav")).unwrap();
    assert_eq!(state.scan(&dir)[0].preview, None);
    std::fs::copy("tests/fixtures/tone.wav", &preview).unwrap();
    assert_eq!(state.scan(&dir)[0].preview.as_ref(), Some(&preview));
    assert_eq!(counts(), (1, 1));
    assert_eq!(
        crate::bmson::read_meta(&chart).unwrap().preview.as_ref(),
        Some(&preview)
    );

    std::fs::remove_file(&preview).unwrap();
    assert_eq!(state.scan(&dir)[0].preview, None);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn progress_uses_the_displayed_revision_after_chart_replacement() {
    let dir = super::super::tests::tempdir();
    let path = dir.join("chart.bms");
    let records = dir.join("records.json");
    let mut state = crate::select::SelectorState::default();
    std::fs::write(&path, "#TITLE Original\n#00111:01\n").unwrap();
    let original = state.scan(&dir);

    std::fs::write(&path, "#TITLE Replacement\n#00111:01\n").unwrap();
    let replacement = state.scan(&dir);
    save_result(replacement[0].clone(), &records);

    let before = counts();
    assert_eq!(original[0].title, "Original");
    assert_eq!(progress(&original, &records), ["[UNPLAYED]"]);
    assert!(progress(&replacement, &records)[0].starts_with("[BEST "));
    assert_eq!(counts(), before);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[cfg(unix)]
fn rescan_skips_an_oversized_symlink_target_after_a_raced_read() {
    use std::{fs, os::unix::fs::symlink};

    let dir = super::super::tests::tempdir();
    let chart = dir.join("linked.bms");
    let small = dir.join("small.txt");
    let large = dir.join("large.txt");
    fs::write(&small, "#TITLE Linked chart").unwrap();
    fs::File::create(&large)
        .unwrap()
        .set_len(chart_file::MAX_CHART_BYTES + 1)
        .unwrap();
    symlink(&large, &chart).unwrap();

    let mut discovery = Discovery::default();
    let seed = || 0;
    let first = discovery.chart(&chart, &seed, |path| {
        fs::remove_file(path)?;
        symlink(&small, path)?;
        let bytes = chart_file::read(path)?;
        fs::remove_file(path)?;
        symlink(&large, path)?;
        Ok(bytes)
    });

    assert_eq!(first.unwrap().title, "Linked chart");
    let was_cached = discovery.metadata.contains_key(&chart);
    assert!(discovery.scan(&dir, &seed).is_empty());
    assert!(!was_cached);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn seeded_branches_are_redrawn_and_match_gameplay_each_visit() {
    let dir = super::super::tests::tempdir();
    std::fs::write(
        dir.join("branch.bms"),
        "\u{feff}#rAnDoM 2\n#IF 1\n#TITLE First\n#ELSE\n#TITLE Second\n#ENDIF\n#00111:01\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("switch.bms"),
        "#SWITCH 2\n#CASE 1\n#TITLE One\n#SKIP\n#CASE 2\n#TITLE Two\n#ENDSW\n#00111:01\n",
    )
    .unwrap();
    let random = dir.join("random.bms");
    std::fs::write(
        &random,
        include_bytes!("../../../tests/fixtures/random.bms"),
    )
    .unwrap();
    let args = Args::parse_from(["tapline"]);
    let mut discovery = Discovery::default();
    let mut previous_titles = HashMap::new();
    for seed in [0, 1] {
        let before = counts();
        let charts = discovery.scan(&dir, &|| seed);
        assert_eq!(charts.len(), 3);
        assert_eq!(counts(), (before.0 + 3, before.1 + 3));
        for meta in charts {
            assert_eq!(meta.random_seed, seed);
            let (chart, identity) =
                load_session(&ChartChoice::Selected(Box::new(meta.clone())), &args)
                    .unwrap()
                    .unwrap();
            assert_eq!(chart.title, meta.title);
            assert_eq!(chart.bpm, meta.bpm);
            assert_eq!(chart.lane_count, meta.lane_count);
            assert_eq!(identity.unwrap().random_seed, Some(seed));
            if meta.path == random {
                let replay = bms::load_with_seed(&random, args.countdown_ms, seed).unwrap();
                assert_eq!(chart.notes[0].keysound, replay.notes[0].keysound);
            }
            if let Some(previous_title) =
                previous_titles.insert(meta.path.clone(), meta.title.clone())
            {
                assert_ne!(previous_title, meta.title);
            }
        }
    }
    let repeated = discovery.scan(&dir, &|| 1);
    assert_eq!(repeated.len(), previous_titles.len());
    for entry in repeated {
        assert_eq!(entry.random_seed, 1);
        assert_eq!(previous_titles.get(&entry.path), Some(&entry.title));
    }
    std::fs::remove_dir_all(dir).unwrap();
}

fn progress(charts: &[ChartEntry], path: &Path) -> Vec<String> {
    super::super::load_progress(charts, Some(path))
}

fn save_result(meta: ChartEntry, path: &Path) {
    let args = Args::parse_from(["tapline"]);
    let (chart, identity) = load_session(&ChartChoice::Selected(Box::new(meta)), &args)
        .unwrap()
        .unwrap();
    let mut game = Game::new(chart);
    game.completed = true;
    let mut store = Store::default();
    store.record(&identity.unwrap(), &game, false);
    store.save(path).unwrap();
}
