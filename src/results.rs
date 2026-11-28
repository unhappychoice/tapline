use crate::game::Game;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions, TryLockError},
    io,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

// Instrumented coverage binaries spawn and schedule subprocesses much slower than a
// release build, so this needs enough headroom for 8 concurrent workers to each take
// their turn with the lock under cargo-llvm-cov.
const LOCK_WAIT_LIMIT: Duration = Duration::from_secs(10);
const LOCK_RETRY_INTERVAL: Duration = Duration::from_millis(10);

#[derive(Clone, Debug)]
pub struct Identity {
    pub key: String,
    pub random_seed: Option<u64>,
}

impl Identity {
    pub fn file(path: &Path, random_seed: Option<u64>) -> anyhow::Result<Self> {
        let bytes = crate::chart_file::read(path)?;
        Ok(Self::from_bytes(path, &bytes, random_seed))
    }

    pub(crate) fn from_bytes(path: &Path, bytes: &[u8], random_seed: Option<u64>) -> Self {
        let extension = path
            .extension()
            .unwrap_or_default()
            .to_string_lossy()
            .to_ascii_lowercase();
        let digest = Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        Self {
            key: format!("{extension}:{digest}"),
            random_seed,
        }
    }

    pub fn built_in(bpm: f64) -> Self {
        Self {
            key: format!("built-in-v1:{bpm}"),
            random_seed: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ResultRecord {
    pub score: u32,
    pub accuracy: f64,
    pub max_combo: u32,
    pub clear: bool,
    pub random_seed: Option<u64>,
    pub auto_keysound: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Best {
    pub score: u32,
    pub accuracy: f64,
    pub max_combo: u32,
    pub clear: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Record {
    pub best: Best,
    pub latest: ResultRecord,
}

#[derive(Default, Serialize, Deserialize)]
pub struct Store {
    records: BTreeMap<String, Record>,
    #[serde(skip)]
    cached_file: Option<CachedFile>,
}

#[derive(Debug)]
pub enum UpdateError {
    Read,
    Lock,
    Write(Option<Record>),
}

pub fn update(
    path: &Path,
    identity: &Identity,
    game: &Game,
    auto_keysound: bool,
) -> Result<Option<Record>, UpdateError> {
    let _lock = game
        .completed
        .then(|| lock_results(path))
        .transpose()
        .map_err(|_| UpdateError::Lock)?;
    let mut store = Store::load(path).map_err(|_| UpdateError::Read)?;
    let previous = store.get(identity).cloned();
    if game.completed {
        store.record(identity, game, auto_keysound);
        if store.save(path).is_err() {
            return Err(UpdateError::Write(previous));
        }
    }
    Ok(previous)
}

/// Like [`update`], but merges into a `Store` the caller already has loaded
/// (e.g. the one the selector read to show progress) instead of rereading
/// and reparsing the whole file just to change one entry, as long as nothing
/// else has written to the file since. Falls back to a fresh [`Store::load`]
/// under the lock when the file no longer matches the cached snapshot.
pub fn update_cached(
    store: &mut Store,
    path: &Path,
    identity: &Identity,
    game: &Game,
    auto_keysound: bool,
) -> Result<Option<Record>, UpdateError> {
    if !game.completed {
        return Ok(store.get(identity).cloned());
    }
    let _lock = lock_results(path).map_err(|_| UpdateError::Lock)?;
    store.refresh(path).map_err(|_| UpdateError::Read)?;
    let previous = store.get(identity).cloned();
    store.record(identity, game, auto_keysound);
    match store.save_snapshot(path) {
        Ok(file) => store.cached_file = CachedFile::new(file.metadata(), file),
        Err(_) => {
            store.restore(identity, previous.clone());
            return Err(UpdateError::Write(previous));
        }
    }
    Ok(previous)
}

struct CachedFile {
    #[cfg(unix)]
    // Holding the original inode prevents its reuse while this snapshot is cached.
    _file: File,
    #[cfg(unix)]
    metadata: fs::Metadata,
}

impl CachedFile {
    fn new(metadata: io::Result<fs::Metadata>, file: File) -> Option<Self> {
        #[cfg(unix)]
        {
            Some(Self {
                _file: file,
                metadata: metadata.ok()?,
            })
        }
        #[cfg(not(unix))]
        {
            // A retained destination handle can prevent replacement on Windows.
            drop((metadata, file));
            None
        }
    }

    fn matches(&self, file: &File) -> bool {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let cached = &self.metadata;
            file.metadata().is_ok_and(|current| {
                current.dev() == cached.dev()
                    && current.ino() == cached.ino()
                    && current.len() == cached.len()
                    && current.mtime() == cached.mtime()
                    && current.mtime_nsec() == cached.mtime_nsec()
                    && current.ctime() == cached.ctime()
                    && current.ctime_nsec() == cached.ctime_nsec()
            })
        }
        #[cfg(not(unix))]
        {
            let _ = file;
            false
        }
    }
}

fn lock_results(path: &Path) -> io::Result<File> {
    fs::create_dir_all(path.parent().unwrap_or(Path::new(".")))?;
    // Keep the lock file in place so replacing the results cannot split the lock.
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path.with_added_extension("lock"))?;
    let deadline = Instant::now() + LOCK_WAIT_LIMIT;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(TryLockError::Error(error)) => return Err(error),
            Err(TryLockError::WouldBlock) => {}
        }
        thread::sleep(LOCK_RETRY_INTERVAL.min(deadline.saturating_duration_since(Instant::now())));
        if Instant::now() >= deadline {
            return Err(io::ErrorKind::TimedOut.into());
        }
    }
}

impl Store {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let mut store = Self::default();
        store.refresh(path)?;
        Ok(store)
    }

    fn refresh(&mut self, path: &Path) -> anyhow::Result<()> {
        let file = match crate::fs_guard::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                *self = Self::default();
                return Ok(());
            }
            Err(error) => return Err(error.into()),
        };
        if !self
            .cached_file
            .as_ref()
            .is_some_and(|cached| cached.matches(&file))
        {
            *self = Self::read_snapshot(file)?;
        }
        Ok(())
    }

    fn read_snapshot(mut file: File) -> anyhow::Result<Self> {
        use std::io::Read;

        let metadata = file.metadata();
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        let mut store: Self = serde_json::from_slice(&bytes)?;
        store.cached_file = CachedFile::new(metadata, file);
        Ok(store)
    }

    pub fn get(&self, identity: &Identity) -> Option<&Record> {
        self.records.get(&identity.key)
    }

    /// Reverts a merged-in record after a failed save, so a `Store` that is
    /// retained across attempts (see [`update_cached`]) does not drift away
    /// from what is actually on disk.
    fn restore(&mut self, identity: &Identity, previous: Option<Record>) {
        match previous {
            Some(record) => {
                self.records.insert(identity.key.clone(), record);
            }
            None => {
                self.records.remove(&identity.key);
            }
        }
    }

    pub fn record(&mut self, identity: &Identity, game: &Game, auto_keysound: bool) {
        if !game.completed {
            return;
        }
        let latest = ResultRecord {
            score: game.score,
            accuracy: game.accuracy(),
            max_combo: game.max_combo,
            clear: game.gauge.is_clear(),
            random_seed: identity.random_seed,
            auto_keysound,
        };
        self.records
            .entry(identity.key.clone())
            .and_modify(|record| {
                record.best.score = record.best.score.max(latest.score);
                record.best.accuracy = record.best.accuracy.max(latest.accuracy);
                record.best.max_combo = record.best.max_combo.max(latest.max_combo);
                record.best.clear |= latest.clear;
                record.latest = latest.clone();
            })
            .or_insert_with(|| Record {
                best: Best {
                    score: latest.score,
                    accuracy: latest.accuracy,
                    max_combo: latest.max_combo,
                    clear: latest.clear,
                },
                latest,
            });
    }

    /// Replaces the file with this snapshot. Use [`update`] to merge a completed run.
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        self.save_snapshot(path).map(drop)
    }

    fn save_snapshot(&self, path: &Path) -> anyhow::Result<File> {
        use std::io::Write;
        let directory = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        fs::create_dir_all(directory)?;
        let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
        temporary.write_all(&serde_json::to_vec_pretty(self)?)?;
        temporary.as_file().sync_all()?;
        Ok(temporary.persist(path).map_err(|error| error.error)?)
    }
}

pub fn data_path() -> Option<PathBuf> {
    let base = std::env::var_os("TAPLINE_DATA_DIR").map(PathBuf::from);
    base.or_else(platform_directory)
        .map(|dir| dir.join("results.json"))
}

fn platform_directory() -> Option<PathBuf> {
    if cfg!(target_os = "windows") {
        return std::env::var_os("LOCALAPPDATA").map(|p| PathBuf::from(p).join("tapline"));
    }
    if cfg!(target_os = "macos") {
        return std::env::var_os("HOME")
            .map(|p| PathBuf::from(p).join("Library/Application Support/tapline"));
    }
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".local/share")))
        .map(|p| p.join("tapline"))
}

#[cfg(test)]
mod tests;
