use crate::game::Game;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug)]
pub struct Identity {
    pub key: String,
    pub random_seed: Option<u64>,
}

impl Identity {
    pub fn file(path: &Path, random_seed: Option<u64>) -> anyhow::Result<Self> {
        let bytes = crate::chart_file::read(path)?;
        let extension = path
            .extension()
            .unwrap_or_default()
            .to_string_lossy()
            .to_ascii_lowercase();
        Ok(Self {
            key: format!("{extension}:{:x}", Sha256::digest(bytes)),
            random_seed,
        })
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
}

impl Store {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        if path.exists() && !path.metadata()?.is_file() {
            anyhow::bail!("results must be a regular file");
        }
        match std::fs::read(path) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error.into()),
        }
    }

    pub fn get(&self, identity: &Identity) -> Option<&Record> {
        self.records.get(&identity.key)
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

    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        use std::io::Write;
        std::fs::create_dir_all(path.parent().unwrap_or(Path::new(".")))?;
        let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        let result = (|| -> anyhow::Result<()> {
            file.write_all(&serde_json::to_vec_pretty(self)?)?;
            file.sync_all()?;
            drop(file);
            std::fs::rename(&temporary, path)?;
            Ok(())
        })();
        let _ = std::fs::remove_file(temporary);
        result
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
