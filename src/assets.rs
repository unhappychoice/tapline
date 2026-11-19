use std::{
    cell::RefCell,
    collections::HashMap,
    fs::{self, Metadata},
    path::{Component, Path, PathBuf},
    rc::Rc,
};

type DirectoryEntries = HashMap<String, Vec<Entry>>;

pub(crate) struct AssetIndex {
    root: PathBuf,
    directories: RefCell<HashMap<PathBuf, Rc<DirectoryEntries>>>,
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
        let path = normalized_asset_path(name)?;
        self.find(&path).or_else(|| {
            extensions
                .iter()
                .find_map(|ext| self.find(&path.with_extension(ext)))
        })
    }

    fn find(&self, path: &Path) -> Option<PathBuf> {
        let components: Vec<_> = path.components().collect();
        let mut candidates = vec![PathBuf::new()];
        for (index, component) in components.iter().enumerate() {
            let directory = index + 1 < components.len();
            let name = normalize_filename(&component.as_os_str().to_string_lossy());
            candidates = candidates
                .iter()
                .flat_map(|parent| {
                    self.entries(parent)
                        .get(&name)
                        .into_iter()
                        .flatten()
                        .inspect(|_| {
                            #[cfg(test)]
                            MATCHING_WORK.with(|count| count.set(count.get() + 1));
                        })
                        .filter(|entry| entry.directory == directory)
                        .filter(|entry| self.matches_type(entry, directory))
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

    fn matches_type(&self, entry: &Entry, directory: bool) -> bool {
        // Recheck cached entries without following links.
        fs::symlink_metadata(self.root.join(&entry.path)).is_ok_and(|metadata| {
            !is_link(&metadata)
                && if directory {
                    metadata.is_dir()
                } else {
                    metadata.is_file()
                }
        })
    }

    fn entries(&self, relative: &Path) -> Rc<DirectoryEntries> {
        self.directories
            .borrow_mut()
            .entry(relative.to_path_buf())
            .or_insert_with(|| {
                #[cfg(test)]
                DIRECTORY_VISITS.with(|count| count.set(count.get() + 1));
                let dir = self.root.join(relative);
                let dir = if dir.as_os_str().is_empty() {
                    Path::new(".")
                } else {
                    &dir
                };
                let mut entries: Vec<_> = fs::read_dir(dir)
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
                let mut by_name: HashMap<String, Vec<Entry>> = HashMap::new();
                entries.into_iter().for_each(|entry| {
                    let name =
                        normalize_filename(&entry.path.file_name().unwrap().to_string_lossy());
                    by_name.entry(name).or_default().push(entry);
                });
                Rc::new(by_name)
            })
            .clone()
    }
}

pub(crate) fn normalized_asset_path(name: &str) -> Option<PathBuf> {
    let normalized = name.replace('\\', "/");
    if normalized.contains([':', '\0']) {
        return None;
    }
    let path: PathBuf = Path::new(&normalized)
        .components()
        .filter(|component| *component != Component::CurDir)
        .collect();
    (!path.as_os_str().is_empty() && path.components().all(|c| matches!(c, Component::Normal(_))))
        .then_some(path)
}

pub(crate) fn is_link(metadata: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // Reject reparse points, including directory junctions.
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    metadata.is_symlink()
}

fn normalize_filename(name: &str) -> String {
    #[cfg(test)]
    MATCHING_WORK.with(|count| count.set(count.get() + 1));
    name.to_lowercase()
}

#[cfg(test)]
thread_local! {
    static MATCHING_WORK: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(crate) static DIRECTORY_VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
mod tests;
