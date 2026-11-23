use std::{
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};
use tapline::{
    chart,
    game::Game,
    results::{Identity, Store},
};

#[test]
fn stale_temporary_file_does_not_block_repeated_saves() {
    let dir = directory();
    let path = dir.join("results.json");
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    let identity = Identity::built_in(120.0);
    let mut store = recorded_store(&identity, 100);
    store.save(&path).unwrap();
    std::fs::write(&temporary, b"interrupted write").unwrap();

    (2..=3).for_each(|run| {
        record_score(&mut store, &identity, run * 100);
        store.save(&path).unwrap();
        assert_eq!(
            Store::load(&path).unwrap().get(&identity),
            store.get(&identity)
        );
        assert_eq!(std::fs::read(&temporary).unwrap(), b"interrupted write");
    });
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn failed_replacement_removes_temporary_file_and_allows_retry() {
    let dir = directory();
    let path = dir.join("results.json");
    std::fs::create_dir(&path).unwrap();
    let marker = path.join("keep.txt");
    std::fs::write(&marker, b"existing data").unwrap();
    let identity = Identity::built_in(120.0);
    let store = recorded_store(&identity, 100);

    let result = store.save(&path);
    assert!(result.is_err());
    assert_eq!(std::fs::read(&marker).unwrap(), b"existing data");
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);

    std::fs::remove_dir_all(&path).unwrap();
    store.save(&path).unwrap();
    assert_eq!(
        Store::load(&path).unwrap().get(&identity),
        store.get(&identity)
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(unix)]
#[test]
fn open_reader_keeps_the_previous_snapshot_after_replacement() {
    use std::io::Read;

    let dir = directory();
    let path = dir.join("results.json");
    let identity = Identity::built_in(120.0);
    let mut store = recorded_store(&identity, 100);
    store.save(&path).unwrap();
    let mut reader = std::fs::File::open(&path).unwrap();
    let original = std::fs::read(&path).unwrap();

    record_score(&mut store, &identity, 200);
    store.save(&path).unwrap();

    let mut snapshot = Vec::new();
    reader.read_to_end(&mut snapshot).unwrap();
    assert_eq!(snapshot, original);
    assert_eq!(
        Store::load(&path).unwrap().get(&identity),
        store.get(&identity)
    );
    drop(reader);
    std::fs::remove_dir_all(dir).unwrap();
}

fn recorded_store(identity: &Identity, score: u32) -> Store {
    let mut store = Store::default();
    record_score(&mut store, identity, score);
    store
}

fn record_score(store: &mut Store, identity: &Identity, score: u32) {
    let mut game = Game::new(chart::built_in(120.0, 0.0));
    game.completed = true;
    game.score = score;
    store.record(identity, &game, false);
}

fn directory() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "tapline-results-persistence-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&dir).unwrap();
    dir
}
