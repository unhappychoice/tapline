use super::*;

fn directory() -> PathBuf {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "tapline-results-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn game(score: u32, combo: u32, perfect: u32, miss: u32) -> Game {
    let mut game = Game::new(crate::chart::built_in(120.0, 0.0));
    game.completed = true;
    game.score = score;
    game.max_combo = combo;
    game.perfect = perfect;
    game.miss = miss;
    game
}

#[test]
fn identity_tracks_contents_and_format_but_not_location_or_seed() {
    let dir = directory();
    let first = dir.join("first.bms");
    let moved = dir.join("moved.BMS");
    std::fs::write(&first, "#TITLE One").unwrap();
    std::fs::copy(&first, &moved).unwrap();
    let identity = Identity::file(&first, Some(1)).unwrap();
    assert_eq!(identity.key, Identity::file(&moved, Some(2)).unwrap().key);
    std::fs::write(&first, "#TITLE Two").unwrap();
    assert_ne!(identity.key, Identity::file(&first, Some(1)).unwrap().key);
    assert_ne!(Identity::built_in(120.0).key, Identity::built_in(150.0).key);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn identity_key_hex_encodes_the_sha256_digest() {
    let identity = Identity::from_bytes(Path::new("file.BMS"), b"abc", None);
    assert_eq!(
        identity.key,
        "bms:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn best_values_improve_independently_and_latest_keeps_provenance() {
    let mut store = Store::default();
    let mut id = Identity::built_in(120.0);
    id.random_seed = Some(42);
    store.record(&id, &game(1000, 10, 1, 1), false);
    id.random_seed = Some(99);
    store.record(&id, &game(900, 20, 3, 1), true);
    let record = store.get(&id).unwrap();
    assert_eq!(record.best.score, 1000);
    assert_eq!(record.best.max_combo, 20);
    assert_eq!(record.best.accuracy, 75.0);
    assert_eq!(record.latest.score, 900);
    assert_eq!(record.latest.random_seed, Some(99));
    assert!(record.latest.auto_keysound);
    assert!(
        crate::render::format_progress(&game(1100, 21, 4, 0), Some(record)).contains("score +100")
    );
    assert!(
        crate::render::format_progress(&game(1100, 21, 4, 0), Some(record))
            .contains("accuracy +25.00pp")
    );
}

#[test]
fn interrupted_plays_do_not_create_or_overwrite_records() {
    let mut store = Store::default();
    let id = Identity::built_in(120.0);
    let mut interrupted = game(1000, 10, 1, 0);
    interrupted.completed = false;
    store.record(&id, &interrupted, false);
    assert!(store.get(&id).is_none());
    store.record(&id, &game(100, 1, 1, 1), false);
    let previous = store.get(&id).unwrap().clone();
    store.record(&id, &interrupted, false);
    assert_eq!(store.get(&id).unwrap(), &previous);
    assert!(crate::render::format_progress(&interrupted, Some(&previous)).contains("not saved"));
}

#[test]
fn clear_achievement_survives_a_later_failed_play() {
    let mut store = Store::default();
    let id = Identity::built_in(120.0);
    let mut cleared = game(1000, 10, 1, 0);
    (0..1000).for_each(|_| cleared.gauge.apply(crate::game::Judgment::Perfect));
    assert!(cleared.gauge.is_clear());
    assert!(crate::render::format_progress(&cleared, None).contains("First completed"));
    store.record(&id, &cleared, false);
    store.record(&id, &game(0, 0, 0, 1), false);
    assert!(store.get(&id).unwrap().best.clear);
    assert!(!store.get(&id).unwrap().latest.clear);
}

#[test]
fn persistence_round_trip_replaces_file_and_rejects_corrupt_storage() {
    let dir = directory();
    let path = dir.join("data/results.json");
    let id = Identity::built_in(120.0);
    assert!(update(&path, &id, &game(100, 2, 2, 1), false)
        .unwrap()
        .is_none());
    let first = Store::load(&path).unwrap().get(&id).unwrap().clone();
    let second = game(200, 3, 3, 1);
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    fs::write(&temporary, "blocked").unwrap();
    assert!(matches!(
        update(&path, &id, &second, true),
        Err(UpdateError::Write(Some(previous))) if previous == first
    ));
    fs::remove_file(temporary).unwrap();
    assert_eq!(update(&path, &id, &second, true).unwrap(), Some(first));
    assert_eq!(
        Store::load(&path).unwrap().get(&id).unwrap().best.score,
        200
    );
    std::fs::write(&path, "broken").unwrap();
    assert!(matches!(
        update(&path, &id, &second, true),
        Err(UpdateError::Read)
    ));
    assert_eq!(fs::read_to_string(&path).unwrap(), "broken");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn concurrent_processes_preserve_distinct_charts_and_shared_bests() {
    use std::process::{Command, Stdio};

    let dir = directory();
    let path = dir.join("results.json");
    let lock = lock_results(&path).unwrap();
    let workers: Vec<_> = (1..=8)
        .map(|index| {
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "results::tests::result_update_worker",
                    "--ignored",
                ])
                .env("TAPLINE_TEST_RESULTS_PATH", &path)
                .env("TAPLINE_TEST_RESULT_INDEX", index.to_string())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap()
        })
        .collect();
    drop(lock);
    workers.into_iter().for_each(|worker| {
        let output = worker.wait_with_output().unwrap();
        assert!(output.status.success(), "{output:?}");
        assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
    });
    let store = Store::load(&path).unwrap();
    (1..=8).for_each(|index| {
        let id = Identity::built_in(f64::from(index));
        assert_eq!(store.get(&id).unwrap().best.score, index * 100);
    });
    let best = &store.get(&Identity::built_in(120.0)).unwrap().best;
    assert_eq!(best.score, 800);
    assert_eq!(best.max_combo, 8);
    assert_eq!(best.accuracy, 100.0);
    assert!(best.clear);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
#[ignore = "subprocess entry point for concurrent result updates"]
fn result_update_worker() {
    let Some(path) = std::env::var_os("TAPLINE_TEST_RESULTS_PATH").map(PathBuf::from) else {
        return;
    };
    let index: u32 = std::env::var("TAPLINE_TEST_RESULT_INDEX")
        .unwrap()
        .parse()
        .unwrap();
    let mut run = game(index * 100, 9 - index, 1, index - 1);
    if index == 1 {
        (0..1000).for_each(|_| run.gauge.apply(crate::game::Judgment::Perfect));
    }
    let id = Identity::built_in(f64::from(index));
    assert!(update(&path, &id, &run, false).unwrap().is_none());
    update(&path, &Identity::built_in(120.0), &run, false).unwrap();
}

#[cfg(unix)]
#[test]
fn loading_a_replacement_fifo_does_not_block() {
    let dir = directory();
    let path = dir.join("results.json");
    std::fs::write(&path, b"{}").unwrap();
    assert!(path.metadata().unwrap().is_file());
    std::fs::remove_file(&path).unwrap();
    let name = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    let (sender, receiver) = std::sync::mpsc::channel();
    let worker_path = path.clone();
    std::thread::spawn(move || {
        sender.send(Store::load(&worker_path).is_err()).ok();
    });
    let result = receiver.recv_timeout(std::time::Duration::from_secs(5));
    std::fs::remove_dir_all(dir).unwrap();
    assert!(result.expect("replacement FIFO must not block Store::load"));
}
