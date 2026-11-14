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
    let mut store = Store::load(&path).unwrap();
    store.record(&id, &game(100, 2, 2, 1), false);
    store.save(&path).unwrap();
    store.record(&id, &game(200, 3, 3, 1), true);
    store.save(&path).unwrap();
    assert_eq!(Store::load(&path).unwrap().get(&id), store.get(&id));
    std::fs::write(&path, "broken").unwrap();
    assert!(Store::load(&path).is_err());
    assert!(store.save(&dir).is_err());
    std::fs::remove_dir_all(dir).unwrap();
}
