use rodio::mixer::{self, Mixer};
use rodio::source::Zero;
use rodio::Source;
use std::time::Duration;

pub(crate) type MasterBus = Mixer;

const CHANNELS: rodio::ChannelCount = rodio::nz!(2);
const SAMPLE_RATE: rodio::SampleRate = rodio::nz!(44_100);
const CEILING: f32 = 0.98;
const RELEASE_SECS: f32 = 0.08;

/// Routes every voice through one mixer followed by a peak limiter, so dense
/// keysound charts whose summed signal exceeds full scale do not hard-clip.
pub(crate) fn master_bus(output: &Mixer) -> MasterBus {
    let (controller, mixer) = mixer::mixer(CHANNELS, SAMPLE_RATE);
    // The mixer ends once it has no sources; keep it alive with silence.
    controller.add(Zero::new(CHANNELS, SAMPLE_RATE));
    output.add(Limiter::new(mixer));
    controller
}

pub(crate) struct Limiter<S> {
    inner: S,
    gain: f32,
    release: f32,
}

impl<S: Source<Item = f32>> Limiter<S> {
    pub(crate) fn new(inner: S) -> Self {
        let samples_per_sec = inner.sample_rate().get() as f32 * inner.channels().get() as f32;
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
mod tests;
