use std::{
    cell::RefCell,
    collections::HashMap,
    path::{Component, Path, PathBuf},
    rc::Rc,
};

pub(crate) struct AssetIndex {
    root: PathBuf,
    directories: RefCell<HashMap<PathBuf, Rc<Vec<Entry>>>>,
}

struct Entry {
    path: PathBuf,
    directory: bool,
}

impl AssetIndex {
    pub(crate) fn new(dir: &Path) -> Self {
        Self {
            root: dir.to_path_buf(),
            directories: RefCell::new(HashMap::new()),
        }
    }

    pub(crate) fn resolve(
        &self,
        defs: &HashMap<u32, String>,
        extensions: &[&str],
    ) -> HashMap<u32, PathBuf> {
        defs.iter()
            .filter_map(|(id, name)| self.resolve_path(name, extensions).map(|path| (*id, path)))
            .collect()
    }

    pub(crate) fn resolve_path(&self, name: &str, extensions: &[&str]) -> Option<PathBuf> {
        let normalized = name.replace('\\', "/");
        let path: PathBuf = Path::new(&normalized)
            .components()
            .filter(|component| *component != Component::CurDir)
            .collect();
        self.find(&path).or_else(|| {
            extensions
                .iter()
                .find_map(|ext| self.find(&path.with_extension(ext)))
        })
    }

    fn find(&self, path: &Path) -> Option<PathBuf> {
        let components: Vec<_> = path.components().collect();
        if components.is_empty()
            || components
                .iter()
                .any(|c| !matches!(c, Component::Normal(_)))
        {
            return None;
        }
        let mut candidates = vec![PathBuf::new()];
        for (index, component) in components.iter().enumerate() {
            let directory = index + 1 < components.len();
            let name = component.as_os_str().to_string_lossy().to_lowercase();
            candidates = candidates
                .iter()
                .flat_map(|parent| {
                    self.entries(parent)
                        .iter()
                        .filter(|entry| entry.directory == directory)
                        .filter(|entry| {
                            entry
                                .path
                                .file_name()
                                .unwrap()
                                .to_string_lossy()
                                .to_lowercase()
                                == name
                        })
                        .filter(|entry| directory || self.root.join(&entry.path).is_file())
                        .map(|entry| entry.path.clone())
                        .collect::<Vec<_>>()
                })
                .collect();
        }
        candidates
            .iter()
            .find(|candidate| *candidate == path)
            .or_else(|| candidates.first())
            .map(|path| self.root.join(path))
    }

    fn entries(&self, relative: &Path) -> Rc<Vec<Entry>> {
        self.directories
            .borrow_mut()
            .entry(relative.to_path_buf())
            .or_insert_with(|| {
                #[cfg(test)]
                DIRECTORY_VISITS.with(|count| count.set(count.get() + 1));
                let mut entries: Vec<_> = std::fs::read_dir(self.root.join(relative))
                    .into_iter()
                    .flatten()
                    .filter_map(Result::ok)
                    .filter_map(|entry| {
                        Some(Entry {
                            path: relative.join(entry.file_name()),
                            directory: entry.file_type().ok()?.is_dir(),
                        })
                    })
                    .collect();
                entries.sort_by(|a, b| a.path.cmp(&b.path));
                Rc::new(entries)
            })
            .clone()
    }
}

#[cfg(test)]
thread_local! {
    static DIRECTORY_VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
mod tests;
