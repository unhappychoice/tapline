use super::ChartEntry;
use crate::{assets::AssetIndex, bms, chart_file, results::Identity};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Default)]
pub(super) struct Discovery {
    metadata: HashMap<PathBuf, (Stamp, CachedMeta)>,
}

struct CachedMeta {
    entry: ChartEntry,
    preview: Option<String>,
}

impl Discovery {
    pub(super) fn scan(&mut self, root: &Path, seed: &impl Fn() -> u64) -> Vec<ChartEntry> {
        let mut charts = Vec::new();
        self.walk(root, 0, &mut charts, seed);
        let paths: HashSet<_> = charts.iter().map(|chart| chart.path.as_path()).collect();
        self.metadata
            .retain(|path, _| paths.contains(path.as_path()));
        let mut keyed_charts: Vec<_> = charts
            .into_iter()
            .map(|chart| {
                #[cfg(test)]
                tests::TITLE_SORT_KEYS.with(|count| count.set(count.get() + 1));
                (chart.title.to_ascii_lowercase(), chart)
            })
            .collect();
        keyed_charts.sort_unstable_by(|(a_key, a), (b_key, b)| {
            a_key.cmp(b_key).then_with(|| a.path.cmp(&b.path))
        });
        keyed_charts.into_iter().map(|(_, chart)| chart).collect()
    }

    pub(super) fn invalidate(&mut self, path: &Path) {
        self.metadata.remove(path);
    }

    fn walk(
        &mut self,
        dir: &Path,
        depth: usize,
        out: &mut Vec<ChartEntry>,
        seed: &impl Fn() -> u64,
    ) {
        if depth > 5 {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let assets = AssetIndex::new(dir);
        for entry in entries.flatten() {
            let path = entry.path();
            if entry.file_type().is_ok_and(|ty| ty.is_dir()) {
                self.walk(&path, depth + 1, out, seed);
            } else if is_bms(&path) {
                out.extend(self.chart(&path, &assets, seed, chart_file::read));
            }
        }
    }

    fn chart(
        &mut self,
        path: &Path,
        assets: &AssetIndex,
        seed: &impl Fn() -> u64,
        read: impl FnOnce(&Path) -> anyhow::Result<Vec<u8>>,
    ) -> Option<ChartEntry> {
        let before = stamp(path);
        let is_bmson = crate::bmson::is_bmson(path);
        if let Some((stamp, cached)) = self.metadata.get(path) {
            if before.as_ref() == Some(stamp) {
                let mut entry = cached.for_visit(assets);
                if !is_bmson {
                    let seed = seed();
                    entry.meta.random_seed = seed;
                    entry.identity.random_seed = Some(seed);
                }
                return Some(entry);
            }
        }
        self.metadata.remove(path);
        let bytes = read(path).ok()?;
        let (meta, preview, draws_random) = if is_bmson {
            let (meta, preview) = crate::bmson::parse_meta(path, &bytes).ok()?;
            (meta, preview, false)
        } else {
            bms::parse_meta(path, &bytes, seed()).ok()?
        };
        let identity = Identity::from_bytes(path, &bytes, (!is_bmson).then_some(meta.random_seed));
        let cached = CachedMeta {
            entry: ChartEntry { meta, identity },
            preview,
        };
        let entry = cached.for_visit(assets);
        if let Some(stamp) =
            before.filter(|before| !draws_random && Some(before) == stamp(path).as_ref())
        {
            self.metadata.insert(path.to_path_buf(), (stamp, cached));
        }
        Some(entry)
    }
}

impl CachedMeta {
    fn for_visit(&self, assets: &AssetIndex) -> ChartEntry {
        let mut entry = self.entry.clone();
        entry.meta.preview = self
            .preview
            .as_deref()
            .and_then(|name| bms::resolve_indexed_preview(assets, name));
        entry
    }
}

#[derive(Clone, PartialEq, Eq)]
struct Stamp {
    length: u64,
    modified: SystemTime,
    #[cfg(unix)]
    changed: (i64, i64, u64, u64),
}

fn stamp(path: &Path) -> Option<Stamp> {
    let meta = path
        .metadata()
        .ok()
        .filter(|meta| meta.is_file() && meta.len() <= crate::chart_file::MAX_CHART_BYTES)?;
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

const EXTS: [&str; 5] = ["bms", "bml", "bme", "pms", "bmson"];

pub(super) fn is_bms(p: &Path) -> bool {
    p.extension()
        .and_then(|s| s.to_str())
        .map(|e| EXTS.iter().any(|x| x.eq_ignore_ascii_case(e)))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) use tests::{count_content_read, count_metadata_parse};
