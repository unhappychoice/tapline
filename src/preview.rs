use rodio::{Decoder, OutputStreamHandle, Sink, Source};
use std::fs::File;
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
    let source = Decoder::new(BufReader::new(File::open(path).ok()?)).ok()?;
    let sink = Sink::try_new(handle).ok()?;
    sink.append(source.repeat_infinite());
    Some(sink)
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
    fn disabled_audio_does_not_open_output_or_play_preview() {
        assert!(crate::audio::preview_output(false).is_none());
        assert!(play(None, Path::new("missing.wav")).is_none());
    }
}
