use super::*;

struct Fixture(PathBuf);

impl Fixture {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("tapline-assets-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn write(&self, name: &str) {
        let path = self.0.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"").unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn empty_definitions_never_enumerate_directories() {
    let fixture = Fixture::new("empty");
    fixture.write("unrelated/deep/file.wav");
    let assets = AssetIndex::new(&fixture.0);
    assert!(assets.resolve(&HashMap::new(), &["wav"]).is_empty());
    assert!(assets.directories.borrow().is_empty());
}

#[test]
fn directory_visits_stay_bounded_as_unrelated_subtrees_grow() {
    let fixture = Fixture::new("bounded");
    fixture.write("Audio/Drums/Kick.OGG");
    [0, 100].into_iter().for_each(|size| {
        (0..size).for_each(|index| {
            fixture.write(&format!("unrelated/{index}/deep/file.wav"));
            fixture.write(&format!("Audio/unrelated/{index}/file.wav"));
        });
        let assets = AssetIndex::new(&fixture.0);
        DIRECTORY_VISITS.with(|count| count.set(0));
        (0..3).for_each(|_| {
            assert_eq!(
                assets.resolve_path("./audio\\drums/kick.wav", &["wav", "ogg"]),
                Some(fixture.0.join("Audio/Drums/Kick.OGG"))
            );
            assert!(assets
                .resolve_path("Audio/Drums/missing.wav", &["wav", "ogg"])
                .is_none());
        });
        let directories = assets.directories.borrow();
        assert_eq!(directories.len(), 3);
        DIRECTORY_VISITS.with(|count| assert_eq!(count.get(), 3));
        ["", "Audio", "Audio/Drums"].iter().for_each(|path| {
            assert!(directories.contains_key(Path::new(path)));
        });
    });
}

#[test]
fn exact_whole_path_precedes_folded_matches_across_directory_collisions() {
    let fixture = Fixture::new("collisions");
    fixture.write("Audio/Kick.wav");
    if fixture.0.join("audio").exists() {
        return;
    }
    fixture.write("audio/kick.wav");
    fixture.write("Audio/snare.wav");
    let assets = AssetIndex::new(&fixture.0);
    assert_eq!(
        assets.resolve_path("audio/kick.wav", &[]),
        Some(fixture.0.join("audio/kick.wav"))
    );
    assert_eq!(
        assets.resolve_path("AUDIO/KICK.WAV", &[]),
        Some(fixture.0.join("Audio/Kick.wav"))
    );
    assert_eq!(
        assets.resolve_path("audio/SNARE.wav", &[]),
        Some(fixture.0.join("Audio/snare.wav"))
    );
}

#[test]
fn references_cannot_escape_root_or_follow_directory_symlinks() {
    let fixture = Fixture::new("paths");
    fixture.write("nested/kick.wav");
    let assets = AssetIndex::new(&fixture.0);
    assert!(assets
        .resolve_path("nested/../nested/kick.wav", &[])
        .is_none());
    assert!(assets
        .resolve_path(fixture.0.join("nested/kick.wav").to_str().unwrap(), &[])
        .is_none());
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(fixture.0.join("nested"), fixture.0.join("link")).unwrap();
        assert!(assets.resolve_path("link/kick.wav", &[]).is_none());
    }
}

#[test]
fn chart_without_asset_definitions_never_visits_asset_directories() {
    let fixture = Fixture::new("chart");
    fixture.write("unrelated/deep/file.wav");
    let path = fixture.0.join("chart.bms");
    std::fs::write(&path, "#BPM 120\n#00111:01\n").unwrap();
    DIRECTORY_VISITS.with(|count| count.set(0));
    let chart = crate::bms::load(&path, 0.0).unwrap();
    assert!(chart.wav_paths.is_empty());
    assert!(chart.bmp_paths.is_empty());
    DIRECTORY_VISITS.with(|count| assert_eq!(count.get(), 0));
}

#[test]
fn matching_work_grows_linearly_with_files_and_references() {
    let work: Vec<_> = [100, 200]
        .into_iter()
        .map(|size| {
            let fixture = Fixture::new(&format!("matching-{size}"));
            (0..size).for_each(|index| fixture.write(&format!("sample-{index}.ogg")));
            let assets = AssetIndex::new(&fixture.0);
            MATCHING_WORK.with(|count| count.set(0));
            (0..size).for_each(|index| {
                assert_eq!(
                    assets.resolve_path(&format!("SAMPLE-{index}.wav"), &["wav", "ogg"]),
                    Some(fixture.0.join(format!("sample-{index}.ogg")))
                );
            });
            let initial = MATCHING_WORK.with(|count| count.get());
            // One normalization per cached file, three candidate names, and one match.
            assert_eq!(initial, size * 5);
            MATCHING_WORK.with(|count| count.set(0));
            (0..size).for_each(|index| {
                assert!(assets
                    .resolve_path(&format!("sample-{index}.ogg"), &[])
                    .is_some());
            });
            // Warm exact-case lookups normalize only the reference and inspect its bucket.
            MATCHING_WORK.with(|count| assert_eq!(count.get(), size * 2));
            initial
        })
        .collect();
    assert_eq!(work[1], work[0] * 2);
}

#[test]
fn filename_collisions_preserve_exact_case_and_sorted_fallback() {
    let fixture = Fixture::new("filename-collisions");
    fixture.write("Kick.wav");
    if fixture.0.join("kick.wav").exists() {
        return;
    }
    fixture.write("kick.wav");
    let assets = AssetIndex::new(&fixture.0);
    assert_eq!(
        assets.resolve_path("kick.wav", &[]),
        Some(fixture.0.join("kick.wav"))
    );
    assert_eq!(
        assets.resolve_path("KICK.WAV", &[]),
        Some(fixture.0.join("Kick.wav"))
    );
}

#[test]
fn cached_entries_keep_types_and_revalidate_regular_files() {
    let fixture = Fixture::new("entry-types");
    fixture.write("sample.wav/nested.ogg");
    fixture.write("sample.ogg");
    fixture.write("removed.wav");
    let assets = AssetIndex::new(&fixture.0);
    assert_eq!(
        assets.resolve_path("sample.wav", &["ogg"]),
        Some(fixture.0.join("sample.ogg"))
    );
    assert!(assets.resolve_path("sample.ogg/nested.ogg", &[]).is_none());
    std::fs::remove_file(fixture.0.join("removed.wav")).unwrap();
    assert!(assets.resolve_path("removed.wav", &[]).is_none());
}
