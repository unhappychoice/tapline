use rodio::Source;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

/// Handle that cuts off the voice it was created with.
pub(crate) struct VoiceHandle(Weak<AtomicBool>);

impl VoiceHandle {
    pub(crate) fn stop(&self) {
        if let Some(stopped) = self.0.upgrade() {
            stopped.store(true, Ordering::Relaxed);
        }
    }

    pub(crate) fn is_live(&self) -> bool {
        self.0.strong_count() > 0
    }
}

/// A source that ends as soon as its handle is stopped or its samples run
/// out, so finished voices leave the mixer instead of idling as silence.
pub(crate) struct Voice<S> {
    inner: S,
    stopped: Arc<AtomicBool>,
}

pub(crate) fn voice<S: Source<Item = f32>>(inner: S) -> (Voice<S>, VoiceHandle) {
    let stopped = Arc::new(AtomicBool::new(false));
    let handle = VoiceHandle(Arc::downgrade(&stopped));
    (Voice { inner, stopped }, handle)
}

impl<S: Source<Item = f32>> Iterator for Voice<S> {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if self.stopped.load(Ordering::Relaxed) {
            return None;
        }
        self.inner.next()
    }
}

impl<S: Source<Item = f32>> Source for Voice<S> {
    fn current_span_len(&self) -> Option<usize> {
        self.inner.current_span_len()
    }
    fn channels(&self) -> rodio::ChannelCount {
        self.inner.channels()
    }
    fn sample_rate(&self) -> rodio::SampleRate {
        self.inner.sample_rate()
    }
    fn total_duration(&self) -> Option<Duration> {
        self.inner.total_duration()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rodio::buffer::SamplesBuffer;

    #[test]
    fn voice_ends_with_its_samples() {
        let (source, _handle) = voice(SamplesBuffer::new(
            rodio::nz!(1),
            rodio::nz!(1000),
            vec![0.5, 0.25],
        ));
        assert_eq!(source.collect::<Vec<_>>(), [0.5, 0.25]);
    }

    #[test]
    fn stopping_the_handle_ends_the_voice_immediately() {
        let (mut source, handle) = voice(SamplesBuffer::new(
            rodio::nz!(2),
            rodio::nz!(1000),
            vec![0.5; 8],
        ));
        assert_eq!(source.next(), Some(0.5));
        handle.stop();
        assert_eq!(source.next(), None);
        assert_eq!(
            (source.channels().get(), source.sample_rate().get()),
            (2, 1000)
        );
    }
}
