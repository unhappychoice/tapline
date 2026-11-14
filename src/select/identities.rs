use crate::results::Identity;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::SystemTime,
};

#[derive(Default)]
pub(super) struct Identities(HashMap<PathBuf, (Stamp, Identity)>);

#[derive(PartialEq, Eq)]
struct Stamp {
    length: u64,
    modified: SystemTime,
    #[cfg(unix)]
    changed: (i64, i64, u64, u64),
}

impl Identities {
    pub(super) fn get(&mut self, path: &Path, bytes: Option<&[u8]>) -> Option<Identity> {
        self.get_with(path, || match bytes {
            Some(bytes) => Ok(Identity::from_bytes(path, bytes, None)),
            None => Identity::file(path, None),
        })
    }

    fn get_with(
        &mut self,
        path: &Path,
        read: impl FnOnce() -> anyhow::Result<Identity>,
    ) -> Option<Identity> {
        let current = stamp(path);
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

fn stamp(path: &Path) -> Option<Stamp> {
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

    #[test]
    fn selector_visits_reuse_reads_and_refresh_records_after_changes() {
        let dir = std::env::temp_dir().join(format!("tapline-identities-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("chart.bms");
        let results = dir.join("results.json");
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
        assert!(cache.get(&path, None).is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn discovery_bytes_supply_identity_without_another_read() {
        let path = Path::new("tests/fixtures/minimal.bmson");
        let bytes = crate::chart_file::read(path).unwrap();
        let mut cache = Identities::default();
        let identity = cache.get(path, Some(&bytes)).unwrap();
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
