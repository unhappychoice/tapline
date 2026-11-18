use super::*;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

fn tempdir() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir =
        std::env::temp_dir().join(format!("tapline-audio-loading-{}-{id}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn chart(dir: &Path, name: &str) -> Chart {
    let path = dir.join("chart.bmson");
    let data = serde_json::json!({
        "version": "1.0.0",
        "info": {"title": "Audio test", "artist": "tapline", "init_bpm": 120,
                 "mode_hint": "generic-4keys"},
        "sound_channels": [{"name": name, "notes": [{"x": 1, "y": 0}]}]
    });
    std::fs::write(&path, data.to_string()).unwrap();
    crate::bmson::load(&path, 0.0).unwrap()
}

#[test]
fn bmson_decodes_regular_audio_and_extension_fallback_without_a_device() {
    let dir = tempdir();
    std::fs::create_dir(dir.join("Audio")).unwrap();
    let wav = dir.join("Audio/Tone.wav");
    std::fs::write(&wav, include_bytes!("../../tests/fixtures/tone.wav")).unwrap();
    for name in ["Audio/Tone.wav", "audio/tone.ogg"] {
        let chart = chart(&dir, name);
        assert_eq!(chart.wav_paths[&0], wav);
        let (samples, failures) = decode_all(&chart.wav_paths);
        assert_eq!(failures, 0);
        let source = sample_source(&samples[&0], chart.sample_slices.get(&0));
        assert!(source.count() > 0);
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(unix)]
#[test]
fn chart_audio_skips_file_and_directory_links_to_other_directories() {
    use std::os::unix::fs::symlink;

    let root = tempdir();
    let song = root.join("song");
    std::fs::create_dir(&song).unwrap();
    let outside = root.join("tone.wav");
    std::fs::write(&outside, include_bytes!("../../tests/fixtures/tone.wav")).unwrap();
    symlink(&outside, song.join("tone.wav")).unwrap();
    symlink(&root, song.join("linked")).unwrap();

    for name in ["tone.wav", "linked/tone.wav"] {
        let bms_path = song.join("chart.bms");
        std::fs::write(&bms_path, format!("#BPM 120\n#WAV01 {name}\n#00111:01\n")).unwrap();
        for chart in [
            chart(&song, name),
            crate::bms::load(&bms_path, 0.0).unwrap(),
        ] {
            assert_eq!(chart.notes.len(), 1);
            assert!(chart.wav_paths.is_empty(), "{name}");
            let (samples, failures) = decode_all(&chart.wav_paths);
            assert!(samples.is_empty());
            assert_eq!(failures, 0);
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
pub(super) fn with_fifo(check: impl FnOnce(&Path) + Send + 'static) {
    let dir = tempdir();
    let fifo = dir.join("blocked.wav");
    let name = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    let (sender, receiver) = std::sync::mpsc::channel();
    let worker_dir = dir.clone();
    let worker = std::thread::spawn(move || {
        check(&fifo);
        std::fs::remove_dir_all(worker_dir).unwrap();
        sender.send(()).unwrap();
    });
    receiver
        .recv_timeout(Duration::from_secs(5))
        .expect("FIFO audio load timed out");
    worker.join().unwrap();
}

#[cfg(unix)]
#[test]
fn bmson_rejects_fifo_and_symlink_audio_without_a_device() {
    with_fifo(|fifo| {
        let dir = fifo.parent().unwrap();
        std::os::unix::fs::symlink(fifo, dir.join("linked.wav")).unwrap();
        ["blocked.wav", "linked.wav"].iter().for_each(|name| {
            let chart = chart(dir, name);
            assert!(chart.wav_paths.is_empty());
            let (samples, failures) = decode_all(&chart.wav_paths);
            assert!(samples.is_empty());
            assert_eq!(failures, 0);
        });
        std::fs::copy("tests/fixtures/tone.wav", dir.join("blocked.ogg")).unwrap();
        let chart = chart(dir, "blocked.wav");
        let (samples, failures) = decode_all(&chart.wav_paths);
        assert_eq!(samples.len(), 1);
        assert_eq!(failures, 0);
    });
}

#[test]
fn decoding_rejects_directories() {
    let dir = tempdir();
    assert!(decode(&dir).is_err());
    std::fs::remove_dir_all(dir).unwrap();
}
