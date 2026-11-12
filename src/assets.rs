use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

pub(crate) struct AssetIndex {
    exact: HashMap<PathBuf, PathBuf>,
    folded: HashMap<String, PathBuf>,
}

impl AssetIndex {
    pub(crate) fn new(dir: &Path) -> Self {
        let mut assets = Self {
            exact: HashMap::new(),
            folded: HashMap::new(),
        };
        assets.scan(dir, Path::new(""));
        assets
    }

    pub(crate) fn resolve(
        &self,
        defs: &HashMap<u32, String>,
        extensions: &[&str],
    ) -> HashMap<u32, PathBuf> {
        defs.iter()
            .filter_map(|(id, name)| {
                self.resolve_path(name, extensions)
                    .map(|path| (*id, path.clone()))
            })
            .collect()
    }

    pub(crate) fn resolve_path(&self, name: &str, extensions: &[&str]) -> Option<&PathBuf> {
        let normalized = name.replace('\\', "/");
        let path: PathBuf = Path::new(&normalized)
            .components()
            .filter(|component| *component != std::path::Component::CurDir)
            .collect();
        self.find(&path).or_else(|| {
            extensions
                .iter()
                .find_map(|ext| self.find(&path.with_extension(ext)))
        })
    }

    fn find(&self, path: &Path) -> Option<&PathBuf> {
        self.exact
            .get(path)
            .or_else(|| self.folded.get(&path.to_string_lossy().to_lowercase()))
    }

    fn scan(&mut self, dir: &Path, relative: &Path) {
        let Ok(entries) = std::fs::read_dir(dir.join(relative)) else {
            return;
        };
        let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = relative.join(entry.file_name());
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                self.scan(dir, &path);
            } else if entry.path().is_file() {
                self.folded
                    .entry(path.to_string_lossy().to_lowercase())
                    .or_insert_with(|| entry.path());
                self.exact.insert(path, entry.path());
            }
        }
    }
}
