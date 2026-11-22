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
fn unavailable_audio_backend_skips_decoding_and_allows_silent_play() {
    let mut chart = crate::chart::built_in(120.0, 0.0);
    chart.notes[0].keysound = Some(1);
    chart.wav_paths.insert(1, Path::new("key.wav").into());
    let audio = ChartAudio::load(&chart, false);
    let calls = AtomicUsize::new(0);
    let bank = audio.start_attempt_with(
        || None,
        |_| {
            calls.fetch_add(1, Ordering::Relaxed);
            anyhow::bail!("unexpected decode")
        },
    );

    assert_eq!(calls.into_inner(), 0);
    assert!(!bank.enabled);
    assert_eq!(bank.sample_count(), 0);
    assert_eq!(bank.decode_failures, 0);
    let duration = bank.chart_duration_ms(&chart, false, false, 0.0);
    assert_eq!(duration, chart.duration_ms);
    assert_eq!(
        crate::render::format_audio_badge(
            crate::render::AudioMode::Samples,
            bank.enabled,
            bank.sample_count(),
            bank.decode_failures,
        ),
        "silent (audio backend unavailable)"
    );

    let note = chart.notes[0];
    let mut game = crate::game::Game::new(chart.clone());
    let keysound = game.hit(note.lane, note.time_ms);
    assert_eq!(keysound, Some(1));
    assert_eq!(game.perfect, 1);
    assert!(bank.start_hit(note.lane, keysound, false, 1.0).is_none());
}

#[test]
fn gameplay_decodes_only_referenced_bms_wavs() {
    let dir = tempdir();
    let wav = include_bytes!("../../tests/fixtures/tone.wav");
    for name in ["shared.wav", "invisible.wav", "unused.wav"] {
        std::fs::write(dir.join(name), wav).unwrap();
    }
    std::fs::write(dir.join("broken.wav"), b"invalid audio").unwrap();
    let path = dir.join("chart.bms");
    std::fs::write(
        &path,
        "#BPM 120\n#WAV01 shared.wav\n#WAV02 shared.wav\n#WAV03 invisible.wav\n\
         #WAV04 unused.wav\n#WAV05 broken.wav\n#00101:01\n#00111:02\n\
         #00132:03\n#00201:05\n",
    )
    .unwrap();
    let chart = crate::bms::load(&path, 0.0).unwrap();
    assert_eq!(chart.wav_paths[&4].as_ref(), dir.join("unused.wav"));
    let calls = AtomicUsize::new(0);
    let audio = ChartAudio::load(&chart, false);
    let (bank, _output) = test_playback(&audio, |path| {
        assert_ne!(path, chart.wav_paths[&4].as_ref());
        calls.fetch_add(1, Ordering::Relaxed);
        decode(path)
    });
    assert_eq!(calls.load(Ordering::Relaxed), 3);
    assert_eq!(bank.decode_failures, 1);
    assert_eq!(bank.sample_count(), 3);
    assert!(Arc::ptr_eq(
        &bank.data.samples[&1].data,
        &bank.data.samples[&2].data
    ));

    assert!(bank.start_sound(chart.bgm[0].keysound, 1.0).is_some());
    assert!(bank.start_sound(chart.bgm[1].keysound, 1.0).is_none());
    let note = chart.notes[0];
    assert!(bank
        .start_hit(note.lane, note.keysound, false, 1.0)
        .is_some());
    let invisible = chart.invisible_notes[0];
    let mut game = crate::game::Game::new(chart);
    let keysound = game.hit(note.lane, note.time_ms);
    assert_eq!(keysound, note.keysound);
    assert!(bank.start_hit(note.lane, keysound, false, 1.0).is_some());
    let keysound = game.hit(invisible.lane, invisible.time_ms);
    assert_eq!(keysound, Some(invisible.keysound));
    assert!(bank
        .start_hit(invisible.lane, keysound, false, 1.0)
        .is_some());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn gameplay_loads_only_reachable_layer_members_and_preserves_slices() {
    let mut chart = crate::chart::built_in(120.0, 0.0);
    chart.notes[0].keysound = Some(10);
    chart.sound_layers = HashMap::from([(10, vec![1, 2]), (20, vec![3])]);
    chart.wav_paths = HashMap::from([
        (1, Path::new("shared.wav").into()),
        (2, Path::new("shared.wav").into()),
        (3, Path::new("unused.wav").into()),
    ]);
    chart.sample_slices.insert(
        2,
        crate::chart::SampleSlice {
            start_ms: 2.0,
            duration_ms: Some(3.0),
        },
    );
    let audio = ChartAudio::load(&chart, false);
    let (bank, _output) = test_playback(&audio, |path| {
        assert_eq!(path, chart.wav_paths[&1].as_ref());
        Ok(DecodedSample::new(
            1,
            1000,
            Arc::new((0..10).map(|i| i as f32).collect()),
        ))
    });
    assert_eq!(bank.sample_count(), 2);
    assert_eq!(bank.decode_failures, 0);
    let slice = bank.sample_with_gain(2, 1.0).unwrap();
    assert_eq!(slice.collect::<Vec<_>>(), [2.0, 3.0, 4.0]);

    let note = chart.notes[0];
    assert_eq!(
        bank.start_hit(note.lane, note.keysound, false, 1.0),
        Some(Duration::from_millis(10))
    );
    assert!(bank.playing.borrow().contains_key(&1));
    assert!(bank.playing.borrow().contains_key(&2));
}

#[test]
fn bmson_decodes_regular_audio_and_extension_fallback_without_a_device() {
    let dir = tempdir();
    std::fs::create_dir(dir.join("Audio")).unwrap();
    let wav = dir.join("Audio/Tone.wav");
    std::fs::write(&wav, include_bytes!("../../tests/fixtures/tone.wav")).unwrap();
    for name in ["Audio/Tone.wav", "audio/tone.ogg"] {
        let chart = chart(&dir, name);
        assert_eq!(chart.wav_paths[&0].as_ref(), wav);
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

fn test_playback(
    audio: &ChartAudio<'_>,
    decode_sample: impl Fn(&Path) -> anyhow::Result<DecodedSample> + Sync,
) -> (SampleBank, rodio::mixer::MixerSource) {
    let (mixer, output) = rodio::mixer::mixer(rodio::nz!(2), rodio::nz!(44_100));
    (
        audio.start_attempt_with(|| Some((None, master::master_bus(&mixer))), decode_sample),
        output,
    )
}

#[test]
fn retry_reuses_decoded_sources_and_starts_with_fresh_voices() {
    let mut chart = crate::chart::built_in(120.0, 0.0);
    chart.notes[0].keysound = Some(1);
    chart.bgm.push(crate::chart::BgmEvent {
        time_ms: 0.0,
        keysound: 2,
    });
    chart.wav_paths = HashMap::from([
        (1, Path::new("key.wav").into()),
        (2, Path::new("bgm.wav").into()),
    ]);
    chart.sample_settings.insert(
        2,
        crate::sample_settings::SampleSettings {
            volume: Some(50),
            ..Default::default()
        },
    );
    let calls = [AtomicUsize::new(0), AtomicUsize::new(0)];
    let audio = ChartAudio::load(&chart, false);
    let decode_sample = |path: &Path| {
        let index = usize::from(path == chart.wav_paths[&2].as_ref());
        calls[index].fetch_add(1, Ordering::Relaxed);
        let mut samples = vec![0.5; 4096];
        samples[..2].fill(0.25);
        Ok(DecodedSample::new(2, 44_100, Arc::new(samples)))
    };

    let silent = audio.start_attempt_with(|| None, decode_sample);
    assert!(!silent.enabled);
    assert!(audio.data.get().is_none());

    for _ in 0..2 {
        let (bank, mut output) = test_playback(&audio, decode_sample);
        assert!(bank.playing.borrow().is_empty());
        assert!(bank.start_sound(2, 1.0).is_some());
        assert_eq!(
            output.by_ref().take(2048).find(|&sample| sample != 0.0),
            Some(0.125)
        );
        assert_eq!(output.next(), Some(0.125));
        assert!(output
            .by_ref()
            .take(1024)
            .any(|sample| (sample - 0.25).abs() < 1e-6));
        assert!(bank.start_hit(0, Some(1), false, 1.0).is_some());
        drop(bank);
        assert!(output.skip(1000).take(100).all(|sample| sample == 0.0));
    }
    assert_eq!(calls.map(AtomicUsize::into_inner), [1, 1]);
}
