use crate::results::Identity;
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    time::SystemTime,
};

#[derive(Default)]
pub(super) struct Identities(HashMap<PathBuf, (Stamp, Identity)>);

#[derive(Clone, PartialEq, Eq)]
pub(super) struct Stamp {
    length: u64,
    modified: SystemTime,
    #[cfg(unix)]
    changed: (i64, i64, u64, u64),
}

impl Identities {
    pub(super) fn get(&mut self, path: &Path) -> Option<Identity> {
        self.get_with(path, || Identity::file(path, None))
    }

    pub(super) fn discover(
        &mut self,
        path: &Path,
        bytes: &[u8],
        before: Option<Stamp>,
    ) -> Option<Stamp> {
        let stable = before.filter(|before| Some(before) == stamp(path).as_ref());
        if let Some(stamp) = &stable {
            let entry = self
                .0
                .entry(path.to_path_buf())
                .or_insert_with(|| (stamp.clone(), Identity::from_bytes(path, bytes, None)));
            if entry.0 != *stamp {
                *entry = (stamp.clone(), Identity::from_bytes(path, bytes, None));
            }
        } else {
            self.0.remove(path);
        }
        stable
    }

    pub(super) fn retain(&mut self, paths: &HashSet<&Path>) {
        self.0.retain(|path, _| paths.contains(path.as_path()));
    }

    fn get_with(
        &mut self,
        path: &Path,
        read: impl FnOnce() -> anyhow::Result<Identity>,
    ) -> Option<Identity> {
        self.get_at(path, stamp(path), read)
    }

    fn get_at(
        &mut self,
        path: &Path,
        current: Option<Stamp>,
        read: impl FnOnce() -> anyhow::Result<Identity>,
    ) -> Option<Identity> {
        if let Some((old, identity)) = self.0.get(path) {
            if current.as_ref() == Some(old) {
                return Some(identity.clone());
            }
        }
        self.0.remove(path);
        let identity = read().ok()?;
        if let Some(stamp) = current.filter(|before| Some(before) == stamp(path).as_ref()) {
            self.0.insert(path.to_path_buf(), (stamp, identity.clone()));
        }
        Some(identity)
    }
}

pub(super) fn stamp(path: &Path) -> Option<Stamp> {
    let meta = path.metadata().ok().filter(|meta| meta.is_file())?;
    Some(Stamp {
        length: meta.len(),
        modified: meta.modified().ok()?,
        #[cfg(unix)]
        changed: {
            use std::os::unix::fs::MetadataExt;
            (meta.ctime(), meta.ctime_nsec(), meta.dev(), meta.ino())
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            (0..)
                .find_map(|index| {
                    let path = std::env::temp_dir()
                        .join(format!("tapline-identities-{}-{index}", std::process::id()));
                    match std::fs::create_dir(&path) {
                        Ok(()) => Some(Self(path)),
                        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => None,
                        Err(error) => panic!("failed to create test directory: {error}"),
                    }
                })
                .unwrap()
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn test_directories_are_isolated_and_removed_on_drop() {
        let first = TestDir::new();
        let second = TestDir::new();
        std::fs::write(first.0.join("results.json"), b"stale results").unwrap();
        assert_ne!(first.0, second.0);
        assert!(!second.0.join("results.json").exists());
        let path = first.0.clone();
        drop(first);
        assert!(!path.exists());
        assert!(second.0.is_dir());
    }

    #[test]
    fn selector_visits_reuse_reads_and_refresh_records_after_changes() {
        let dir = TestDir::new();
        let path = dir.0.join("chart.bms");
        let results = dir.0.join("results.json");
        std::fs::write(&path, b"#TITLE Original").unwrap();
        let mut cache = Identities::default();
        let mut reads = 0;
        let mut visit = |cache: &mut Identities| {
            cache
                .get_with(&path, || {
                    reads += 1;
                    Identity::file(&path, None)
                })
                .unwrap()
        };
        let first = visit(&mut cache);
        assert!(crate::results::Store::load(&results)
            .unwrap()
            .get(&first)
            .is_none());
        let record = serde_json::json!({"best": {"score": 300, "accuracy": 100.0, "max_combo": 1, "clear": true},
            "latest": {"score": 300, "accuracy": 100.0, "max_combo": 1, "clear": true, "random_seed": null, "auto_keysound": false}});
        std::fs::write(
            &results,
            serde_json::to_vec(&serde_json::json!({"records": {first.key.clone(): record}}))
                .unwrap(),
        )
        .unwrap();
        let second = visit(&mut cache);
        assert_eq!(first.key, second.key);
        assert!(crate::results::Store::load(&results)
            .unwrap()
            .get(&second)
            .is_some());
        std::fs::write(&path, b"#TITLE Changed chart").unwrap();
        let changed = visit(&mut cache);
        assert_ne!(first.key, changed.key);
        assert!(crate::results::Store::load(&results)
            .unwrap()
            .get(&changed)
            .is_none());
        visit(&mut cache);
        assert_eq!(reads, 2);
        std::fs::remove_file(&path).unwrap();
        assert!(cache.get(&path).is_none());
    }

    #[test]
    fn discovery_changes_do_not_cache_stale_bytes() {
        let dir = TestDir::new();
        let path = dir.0.join("chart.bms");
        let mut cache = Identities::default();
        std::fs::write(&path, b"#TITLE Original").unwrap();
        let original = cache.get(&path).unwrap();
        let before = stamp(&path);
        let bytes = crate::chart_file::read(&path).unwrap();
        std::fs::write(&path, b"#TITLE Changed during parsing").unwrap();
        cache.discover(&path, &bytes, before);
        assert!(!cache.0.contains_key(&path));
        let changed = cache.get(&path).unwrap();
        assert_ne!(original.key, changed.key);
        assert_eq!(changed.key, Identity::file(&path, None).unwrap().key);
        assert_eq!(
            cache
                .get_with(&path, || panic!("unchanged chart reread"))
                .unwrap()
                .key,
            changed.key
        );
    }

    #[test]
    fn discovery_bytes_supply_identity_without_another_read() {
        let path = Path::new("tests/fixtures/minimal.bmson");
        let before = stamp(path);
        let bytes = crate::chart_file::read(path).unwrap();
        let mut cache = Identities::default();
        cache.discover(path, &bytes, before);
        let identity = cache
            .get_with(path, || panic!("discovery bytes reread"))
            .unwrap();
        assert_eq!(identity.key, Identity::file(path, None).unwrap().key);
        assert_eq!(
            cache
                .get_with(path, || panic!("unchanged chart reread"))
                .unwrap()
                .key,
            identity.key
        );
    }
}
