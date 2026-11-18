use super::identities::{self, Identities, Stamp};
use crate::bms::{self, ChartMeta};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Default)]
pub(super) struct Discovery {
    pub(super) identities: Identities,
    metadata: HashMap<PathBuf, (Stamp, CachedMeta)>,
}

struct CachedMeta {
    meta: ChartMeta,
    preview: Option<String>,
}

impl Discovery {
    pub(super) fn scan(&mut self, root: &Path, seed: &impl Fn() -> u64) -> Vec<ChartMeta> {
        let mut charts = Vec::new();
        self.walk(root, 0, &mut charts, seed);
        let paths: HashSet<_> = charts.iter().map(|chart| chart.path.as_path()).collect();
        self.metadata
            .retain(|path, _| paths.contains(path.as_path()));
        self.identities.retain(&paths);
        charts.sort_by(|a, b| {
            a.title
                .to_ascii_lowercase()
                .cmp(&b.title.to_ascii_lowercase())
                .then_with(|| a.path.cmp(&b.path))
        });
        charts
    }

    fn walk(
        &mut self,
        dir: &Path,
        depth: usize,
        out: &mut Vec<ChartMeta>,
        seed: &impl Fn() -> u64,
    ) {
        if depth > 5 {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if entry.file_type().is_ok_and(|ty| ty.is_dir()) {
                self.walk(&path, depth + 1, out, seed);
            } else if is_bms(&path) {
                out.extend(self.chart(&path, seed));
            }
        }
    }

    fn chart(&mut self, path: &Path, seed: &impl Fn() -> u64) -> Option<ChartMeta> {
        let before = identities::stamp(path);
        let is_bmson = crate::bmson::is_bmson(path);
        if let Some((stamp, cached)) = self.metadata.get(path) {
            if before.as_ref() == Some(stamp) {
                let mut meta = cached.for_visit();
                if !is_bmson {
                    meta.random_seed = seed();
                }
                return Some(meta);
            }
        }
        self.metadata.remove(path);
        let bytes = crate::chart_file::read(path).ok()?;
        let (meta, preview, draws_random) = if is_bmson {
            (
                crate::bmson::read_meta_bytes(path, &bytes).ok()?,
                None,
                false,
            )
        } else {
            bms::parse_meta(path, &bytes, seed()).ok()?
        };
        let cached = CachedMeta { meta, preview };
        let meta = cached.for_visit();
        let stable_stamp = self.identities.discover(path, &bytes, before);
        if let Some(stamp) = stable_stamp.filter(|_| !draws_random) {
            self.metadata.insert(path.to_path_buf(), (stamp, cached));
        }
        Some(meta)
    }
}

impl CachedMeta {
    fn for_visit(&self) -> ChartMeta {
        let mut meta = self.meta.clone();
        meta.preview = bms::resolve_preview(&meta.path, self.preview.as_deref());
        meta
    }
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
