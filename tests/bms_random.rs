use clap::Parser;
use std::path::PathBuf;
use tapline::{bms, cli};

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/random.bms")
}

#[test]
fn injected_play_seeds_change_branches_and_preserve_selector_metadata() {
    let args = cli::Args::parse_from(["tapline", "--no-audio"]);
    let titles: std::collections::HashSet<_> = (0..32)
        .map(|seed| {
            let meta = bms::read_meta_with_seed(&fixture(), seed).unwrap();
            let chart = cli::load_chart(&cli::ChartChoice::Selected(Box::new(meta.clone())), &args)
                .unwrap()
                .unwrap();
            assert_eq!(meta.random_seed, seed);
            assert_eq!(meta.title, chart.title);
            assert_eq!(meta.bpm, chart.bpm);
            assert_eq!(meta.lane_count, chart.lane_count);
            let replay = bms::load_with_seed(&fixture(), args.countdown_ms, seed).unwrap();
            assert_eq!(chart.title, replay.title);
            assert_eq!(chart.notes[0].keysound, replay.notes[0].keysound);
            meta.title
        })
        .collect();
    assert_eq!(titles.len(), 2);
}

#[test]
fn seed_environment_is_applied_without_mutating_the_test_process() {
    for seed in [Some("42"), Some("invalid"), None] {
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", "seed_environment_child"])
            .env_remove("TAPLINE_RANDOM_SEED")
            .env("TAPLINE_RANDOM_TEST_CHILD", "1");
        if let Some(seed) = seed {
            command.env("TAPLINE_RANDOM_SEED", seed);
        }
        let status = command.status().unwrap();
        assert!(status.success());
    }
}

#[test]
fn seed_environment_child() {
    if std::env::var_os("TAPLINE_RANDOM_TEST_CHILD").is_none() {
        return;
    }
    let metas: Vec<_> = (0..8)
        .map(|_| bms::read_meta(&fixture()).unwrap())
        .collect();
    if let Ok(seed) = std::env::var("TAPLINE_RANDOM_SEED")
        .unwrap_or_default()
        .parse::<u64>()
    {
        let expected = bms::load_with_seed(&fixture(), 0.0, seed).unwrap();
        metas.iter().for_each(|meta| {
            assert_eq!(meta.random_seed, seed);
            assert_eq!(meta.title, expected.title);
            assert_eq!(meta.lane_count, expected.lane_count);
            assert_eq!(bms::load(&fixture(), 0.0).unwrap().title, expected.title);
        });
    } else {
        let seeds: std::collections::HashSet<_> =
            metas.iter().map(|meta| meta.random_seed).collect();
        assert!(seeds.len() > 1);
    }
}
