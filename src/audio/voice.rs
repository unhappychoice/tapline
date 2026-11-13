use rodio::Source;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Handle that cuts off the voice it was created with.
#[derive(Default)]
pub(crate) struct VoiceHandle(Arc<AtomicBool>);

impl VoiceHandle {
    pub(crate) fn stop(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

/// A source that ends as soon as its handle is stopped or its samples run
/// out, so finished voices leave the mixer instead of idling as silence.
pub(crate) struct Voice<S> {
    inner: S,
    stopped: Arc<AtomicBool>,
}

pub(crate) fn voice<S: Source<Item = f32>>(inner: S) -> (Voice<S>, VoiceHandle) {
    let handle = VoiceHandle::default();
    let stopped = Arc::clone(&handle.0);
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
    fn current_frame_len(&self) -> Option<usize> {
        self.inner.current_frame_len()
    }
    fn channels(&self) -> u16 {
        self.inner.channels()
    }
    fn sample_rate(&self) -> u32 {
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
        let (source, _handle) = voice(SamplesBuffer::new(1, 1000, vec![0.5, 0.25]));
        assert_eq!(source.collect::<Vec<_>>(), [0.5, 0.25]);
    }

    #[test]
    fn stopping_the_handle_ends_the_voice_immediately() {
        let (mut source, handle) = voice(SamplesBuffer::new(2, 1000, vec![0.5; 8]));
        assert_eq!(source.next(), Some(0.5));
        handle.stop();
        assert_eq!(source.next(), None);
        assert_eq!((source.channels(), source.sample_rate()), (2, 1000));
    }
}
