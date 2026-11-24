use crate::fs_guard;
use rodio::{Decoder, MixerDeviceSink, Player, Source};
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

pub(crate) struct SelectionPreview<T> {
    selected: Option<usize>,
    sink: Option<Player>,
    work: Arc<(Mutex<Work<T>>, Condvar)>,
}

/// Upper bound on the number of decoded samples a preview may feed into
/// `repeat_infinite`. In rodio 0.23.0, `repeat_infinite` buffers its source
/// internally and that buffer retains every decoded sample for the next lap,
/// so capping the source here caps how much of a long preview stays resident
/// in memory for the lifetime of the loop.
const MAX_PREVIEW_SAMPLES: usize = crate::audio::MAX_DECODED_SAMPLES;

impl SelectionPreview<rodio::source::Repeat<BoundedPreview<Decoder<BufReader<File>>>>> {
    pub(crate) fn audio() -> Option<Self> {
        Self::new(|path| {
            bounded_source(path, MAX_PREVIEW_SAMPLES).map(Source::repeat_infinite)
        })
        .ok()
    }
}

impl<T: Send + 'static> SelectionPreview<T> {
    pub(crate) fn new(
        load: impl FnMut(&Path) -> Option<T> + Send + 'static,
    ) -> std::io::Result<Self> {
        let work = Arc::new((Mutex::new(Work::default()), Condvar::new()));
        let worker = Arc::clone(&work);
        std::thread::Builder::new()
            .name("tapline-preview".into())
            .spawn(move || load_previews(&worker, load))?;
        Ok(Self {
            selected: None,
            sink: None,
            work,
        })
    }

    pub(crate) fn update(
        &mut self,
        selected: Option<usize>,
        path: Option<&Path>,
        play: impl FnOnce(T) -> Player,
    ) {
        let (lock, wake) = &*self.work;
        let changed = self.selected != selected;
        if changed {
            self.sink.take().inspect(Player::stop);
            self.selected = selected;
        }
        let mut work = lock.lock().unwrap();
        if changed {
            work.generation += 1;
            work.ready = None;
            work.pending = selected.and(path).map(Path::to_path_buf);
            wake.notify_one();
        }
        let ready = work.ready.take();
        drop(work);
        if let Some(source) = ready {
            self.sink = Some(play(source));
        }
    }
}

impl<T> Drop for SelectionPreview<T> {
    fn drop(&mut self) {
        let (lock, wake) = &*self.work;
        let mut work = lock.lock().unwrap();
        work.closed = true;
        work.pending = None;
        work.ready = None;
        wake.notify_one();
        // The worker exits after any in-flight read returns.
    }
}

struct Work<T> {
    generation: u64,
    pending: Option<PathBuf>,
    ready: Option<T>,
    closed: bool,
}

impl<T> Default for Work<T> {
    fn default() -> Self {
        Self {
            generation: 0,
            pending: None,
            ready: None,
            closed: false,
        }
    }
}

pub(crate) fn play<T: Source + Send + 'static>(handle: &MixerDeviceSink, source: T) -> Player {
    let sink = Player::connect_new(handle.mixer());
    sink.append(source);
    sink
}

fn load_previews<T>(
    (lock, wake): &(Mutex<Work<T>>, Condvar),
    mut load: impl FnMut(&Path) -> Option<T>,
) {
    loop {
        let mut work = wake
            .wait_while(lock.lock().unwrap(), |work| {
                !work.closed && work.pending.is_none()
            })
            .unwrap();
        let Some(path) = work.pending.take() else {
            return;
        };
        let generation = work.generation;
        drop(work);

        let source = load(&path);
        let mut work = lock.lock().unwrap();
        if !work.closed && work.generation == generation {
            work.ready = source;
        }
    }
}

fn load_source(path: &Path) -> Option<Decoder<BufReader<File>>> {
    let file = fs_guard::open(path).ok()?;
    Decoder::try_from(file).ok()
}

fn bounded_source(
    path: &Path,
    max_samples: usize,
) -> Option<BoundedPreview<Decoder<BufReader<File>>>> {
    load_source(path).map(|source| BoundedPreview::new(source, max_samples))
}

/// Caps a source at a fixed number of samples before it is handed to
/// `repeat_infinite`, so the loop buffer it builds can never grow past that
/// budget no matter how long the underlying file is.
struct BoundedPreview<S> {
    inner: S,
    remaining: usize,
}

impl<S> BoundedPreview<S> {
    fn new(inner: S, max_samples: usize) -> Self {
        Self {
            inner,
            remaining: max_samples,
        }
    }
}

impl<S: Source<Item = f32>> Iterator for BoundedPreview<S> {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if self.remaining == 0 {
            return None;
        }
        let sample = self.inner.next()?;
        self.remaining -= 1;
        Some(sample)
    }
}

impl<S: Source<Item = f32>> Source for BoundedPreview<S> {
    fn current_span_len(&self) -> Option<usize> {
        self.inner
            .current_span_len()
            .map(|len| len.min(self.remaining))
    }
    fn channels(&self) -> rodio::ChannelCount {
        self.inner.channels()
    }
    fn sample_rate(&self) -> rodio::SampleRate {
        self.inner.sample_rate()
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_latest_pending_preview_is_loaded_and_played() {
        use std::{sync::mpsc, time::Duration};

        let (started, loads) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        let mut preview = SelectionPreview::new(move |path| {
            started.send(path.to_path_buf()).unwrap();
            wait.recv().ok()?;
            Some(path.to_path_buf())
        })
        .unwrap();
        let timeout = Duration::from_secs(2);
        let update = |preview: &mut SelectionPreview<_>, index, path: Option<&str>| {
            preview.update(index, path.map(Path::new), |_| {
                panic!("stale preview played")
            });
        };
        update(&mut preview, Some(0), Some("first"));
        assert_eq!(loads.recv_timeout(timeout).unwrap(), Path::new("first"));
        update(&mut preview, Some(1), Some("skipped"));
        update(&mut preview, Some(2), Some("latest"));
        release.send(()).unwrap();
        assert_eq!(loads.recv_timeout(timeout).unwrap(), Path::new("latest"));
        update(&mut preview, Some(2), Some("latest"));
        release.send(()).unwrap();

        let (sink, mut queue) = Player::new();
        sink.append(rodio::source::SineWave::new(440.0).repeat_infinite());
        let mut sink = Some(sink);
        let deadline = std::time::Instant::now() + timeout;
        while preview.sink.is_none() {
            preview.update(Some(2), Some(Path::new("latest")), |path| {
                assert_eq!(path, Path::new("latest"));
                sink.take().unwrap()
            });
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        update(&mut preview, Some(2), Some("latest"));
        assert!(queue.next().is_some());
        update(&mut preview, None, None);
        assert!(queue.skip(512).take(1024).all(|sample| sample == 0.0));
    }

    #[test]
    fn regular_audio_decodes_and_repeats() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tone.wav");
        let samples: Vec<_> = load_source(&path).unwrap().collect();
        assert!(!samples.is_empty());
        let repeated: Vec<_> = load_source(&path)
            .unwrap()
            .repeat_infinite()
            .take(samples.len() * 3)
            .collect();
        assert_eq!(repeated, samples.repeat(3));
    }

    #[test]
    fn oversized_preview_cannot_retain_more_than_the_configured_sample_budget() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tone.wav");
        let full: Vec<_> = load_source(&path).unwrap().collect();
        let budget = full.len() / 3;
        assert!(budget > 0 && budget < full.len());

        let looped: Vec<_> = bounded_source(&path, budget)
            .unwrap()
            .repeat_infinite()
            .take(budget * 4)
            .collect();
        let expected: Vec<_> = full[..budget].repeat(4);
        assert_eq!(looped, expected);
    }

    #[test]
    fn missing_invalid_and_directory_previews_are_skipped() {
        assert!(load_source(Path::new("missing-preview.wav")).is_none());
        assert!(load_source(Path::new("Cargo.toml")).is_none());
        assert!(load_source(Path::new(".")).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn fifo_and_symlink_previews_return_without_a_writer() {
        use std::{os::unix::fs::symlink, sync::mpsc, time::Duration};

        let dir = std::env::temp_dir().join(format!("tapline-preview-fifo-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fifo = dir.join("preview.wav");
        let name = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        let link = dir.join("linked.wav");
        symlink(&fifo, &link).unwrap();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            sender
                .send([fifo, link].iter().all(|path| load_source(path).is_none()))
                .unwrap();
        });
        let result = receiver.recv_timeout(Duration::from_secs(2));
        std::fs::remove_dir_all(dir).unwrap();
        assert!(result.expect("special-file previews must not block"));
    }

    #[test]
    fn disabled_audio_does_not_open_output_or_play_preview() {
        assert!(crate::audio::preview_output(false).is_none());
    }
}
