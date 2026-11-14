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
