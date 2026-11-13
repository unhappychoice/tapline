use rodio::dynamic_mixer::{self, DynamicMixerController};
use rodio::source::Zero;
use rodio::{OutputStreamHandle, Source};
use std::sync::Arc;
use std::time::Duration;

pub(crate) type MasterBus = Arc<DynamicMixerController<f32>>;

const CHANNELS: u16 = 2;
const SAMPLE_RATE: u32 = 44_100;
const CEILING: f32 = 0.98;
const RELEASE_SECS: f32 = 0.08;

/// Routes every voice through one mixer followed by a peak limiter, so dense
/// keysound charts whose summed signal exceeds full scale do not hard-clip.
pub(crate) fn master_bus(handle: &OutputStreamHandle) -> Option<MasterBus> {
    let (controller, mixer) = dynamic_mixer::mixer::<f32>(CHANNELS, SAMPLE_RATE);
    // The mixer ends once it has no sources; keep it alive with silence.
    controller.add(Zero::<f32>::new(CHANNELS, SAMPLE_RATE));
    handle.play_raw(Limiter::new(mixer)).ok()?;
    Some(controller)
}

pub(crate) struct Limiter<S> {
    inner: S,
    gain: f32,
    release: f32,
}

impl<S: Source<Item = f32>> Limiter<S> {
    pub(crate) fn new(inner: S) -> Self {
        let samples_per_sec = inner.sample_rate() as f32 * inner.channels() as f32;
        Self {
            inner,
            gain: 1.0,
            release: 1.0 - (-1.0 / (RELEASE_SECS * samples_per_sec)).exp(),
        }
    }

    fn follow(&mut self, peak: f32) {
        let target = if peak > CEILING { CEILING / peak } else { 1.0 };
        self.gain = if target < self.gain {
            target
        } else {
            self.gain + (target - self.gain) * self.release
        };
    }
}

impl<S: Source<Item = f32>> Iterator for Limiter<S> {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        let sample = self.inner.next()?;
        self.follow(sample.abs());
        Some(sample * self.gain)
    }
}

impl<S: Source<Item = f32>> Source for Limiter<S> {
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
mod tests;
