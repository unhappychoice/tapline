use rodio::{Decoder, OutputStreamHandle, Sink, Source};
use std::fs::{File, OpenOptions};
use std::io::BufReader;
use std::path::Path;

#[derive(Default)]
pub(crate) struct SelectionPreview {
    selected: Option<usize>,
    sink: Option<Sink>,
}

impl SelectionPreview {
    pub(crate) fn update(
        &mut self,
        selected: usize,
        path: Option<&Path>,
        play: impl FnOnce(&Path) -> Option<Sink>,
    ) {
        if self.selected == Some(selected) {
            return;
        }
        self.sink.take().inspect(Sink::stop);
        self.selected = Some(selected);
        self.sink = path.and_then(play);
    }
}

pub(crate) fn play(handle: Option<&OutputStreamHandle>, path: &Path) -> Option<Sink> {
    let handle = handle?;
    let source = load_source(path)?;
    let sink = Sink::try_new(handle).ok()?;
    sink.append(source.repeat_infinite());
    Some(sink)
}

fn load_source(path: &Path) -> Option<Decoder<BufReader<File>>> {
    if !path.metadata().ok()?.is_file() {
        return None;
    }
    let mut options = OpenOptions::new();
    options.read(true);
    // Avoid blocking if a regular file is replaced with a FIFO before opening.
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = options.open(path).ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    Decoder::new(BufReader::new(file)).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_changes_stop_previous_audio_and_unchanged_selection_keeps_playing() {
        let mut preview = SelectionPreview::default();
        let path = Some(Path::new("preview.wav"));
        let (sink, mut queue) = Sink::new_idle();
        sink.append(rodio::source::SineWave::new(440.0).repeat_infinite());
        preview.update(0, path, |_| Some(sink));
        assert!(queue.next().is_some());
        preview.update(0, path, |_| panic!("must not restart"));
        assert!(queue.next().is_some());
        preview.update(1, None, |_| panic!("no preview"));
        assert!(queue.skip(512).take(1024).all(|sample| sample == 0.0));
        assert!(preview.sink.is_none());
    }

    #[test]
    fn selection_replaces_preview_and_stops_on_drop() {
        let mut preview = SelectionPreview::default();
        let (sink, mut queue) = Sink::new_idle();
        sink.append(rodio::source::SineWave::new(440.0).repeat_infinite());
        preview.update(0, Some(Path::new("first.wav")), |_| None);
        preview.update(1, Some(Path::new("second.wav")), |_| Some(sink));
        assert!(queue.next().is_some());
        drop(preview);
        assert!(queue.skip(512).take(1024).all(|sample| sample == 0.0));
    }

    #[test]
    fn regular_audio_decodes_and_repeats() {
        let path = Path::new("tests/fixtures/tone.wav");
        let samples: Vec<_> = load_source(path).unwrap().collect();
        assert!(!samples.is_empty());
        let repeated: Vec<_> = load_source(path)
            .unwrap()
            .repeat_infinite()
            .take(samples.len() * 3)
            .collect();
        assert_eq!(repeated, samples.repeat(3));
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
        assert!(play(None, Path::new("missing.wav")).is_none());
    }
}
