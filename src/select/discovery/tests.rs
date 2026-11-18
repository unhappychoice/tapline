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
    assert_eq!(
        progress(&mut state, &charts, &records),
        ["[UNPLAYED]", "[UNPLAYED]"]
    );
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
    assert!(progress(&mut state, &charts, &records)
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
    assert_eq!(
        progress(&mut state, &charts, &records),
        ["[UNPLAYED]", "[UNPLAYED]"]
    );
    assert_eq!(counts(), (5, 4));
    assert!(!state.discovery.metadata.contains_key(&bmson));
    std::fs::remove_dir_all(dir).unwrap();
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
    let args = Args::parse_from(["tapline"]);
    let mut discovery = Discovery::default();
    let mut first_titles = HashMap::new();
    for seed in [0, 1] {
        let before = counts();
        let charts = discovery.scan(&dir, &|| seed);
        assert_eq!(charts.len(), 2);
        assert_eq!(counts(), (before.0 + 2, before.1 + 2));
        for meta in charts {
            assert_eq!(meta.random_seed, seed);
            let (chart, identity) =
                load_session(&ChartChoice::Selected(Box::new(meta.clone())), &args)
                    .unwrap()
                    .unwrap();
            assert_eq!(chart.title, meta.title);
            assert_eq!(identity.unwrap().random_seed, Some(seed));
            if let Some(first_title) = first_titles.insert(meta.path, meta.title.clone()) {
                assert_ne!(first_title, meta.title);
            }
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}

fn progress(
    state: &mut crate::select::SelectorState,
    charts: &[ChartMeta],
    path: &Path,
) -> Vec<String> {
    super::super::load_progress(charts, &mut state.discovery.identities, Some(path))
}

fn save_result(meta: ChartMeta, path: &Path) {
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
