use rodio::buffer::SamplesBuffer;
use rodio::{Decoder, OutputStream, OutputStreamHandle, Source};
use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;

struct DecodedSample {
    channels: u16,
    sample_rate: u32,
    data: Vec<f32>,
}

pub struct SampleBank {
    _stream: Option<OutputStream>,
    handle: Option<OutputStreamHandle>,
    samples: HashMap<u32, DecodedSample>,
    pub enabled: bool,
}

impl SampleBank {
    pub fn new(wav_paths: &HashMap<u32, PathBuf>) -> Self {
        let (stream, handle, enabled) = silence_stderr(|| match OutputStream::try_default() {
            Ok((s, h)) => (Some(s), Some(h), true),
            Err(_) => (None, None, false),
        });
        let samples = wav_paths
            .iter()
            .filter_map(|(id, path)| decode(path).ok().map(|s| (*id, s)))
            .collect();
        Self {
            _stream: stream,
            handle,
            samples,
            enabled,
        }
    }

    pub fn silent() -> Self {
        Self {
            _stream: None,
            handle: None,
            samples: HashMap::new(),
            enabled: false,
        }
    }

    pub fn play(&self, id: u32) {
        let Some(handle) = &self.handle else {
            return;
        };
        let Some(s) = self.samples.get(&id) else {
            return;
        };
        let buf = SamplesBuffer::new(s.channels, s.sample_rate, s.data.clone());
        let _ = handle.play_raw(buf);
    }

    pub fn play_synth(&self, freq: f32, duration_ms: u32) {
        let Some(handle) = &self.handle else {
            return;
        };
        let sr: u32 = 44_100;
        let data = synth_ping(freq, duration_ms, sr);
        let buf = SamplesBuffer::new(1, sr, data);
        let _ = handle.play_raw(buf);
    }

    pub fn play_hit(&self, lane: usize, keysound: Option<u32>, synth_mode: bool) {
        if let Some(id) = keysound {
            if self.samples.contains_key(&id) {
                self.play(id);
                return;
            }
        }
        if synth_mode {
            let f = LANE_PITCHES[lane % LANE_PITCHES.len()];
            self.play_synth(f, 80);
        }
    }
}

const LANE_PITCHES: [f32; 7] = [261.63, 329.63, 392.00, 523.25, 659.25, 783.99, 1046.50];

fn synth_ping(freq: f32, duration_ms: u32, sample_rate: u32) -> Vec<f32> {
    let total = (sample_rate * duration_ms / 1000) as usize;
    let sr = sample_rate as f32;
    (0..total)
        .map(|i| {
            let t = i as f32 / sr;
            let env = (-t * 18.0).exp();
            let s1 = (2.0 * std::f32::consts::PI * freq * t).sin();
            let s2 = (2.0 * std::f32::consts::PI * freq * 2.0 * t).sin() * 0.3;
            (s1 + s2) * env * 0.35
        })
        .collect()
}

fn decode(path: &PathBuf) -> anyhow::Result<DecodedSample> {
    let file = BufReader::new(File::open(path)?);
    let decoder = Decoder::new(file)?;
    let channels = decoder.channels();
    let sample_rate = decoder.sample_rate();
    let data: Vec<f32> = decoder.convert_samples::<f32>().collect();
    Ok(DecodedSample {
        channels,
        sample_rate,
        data,
    })
}

#[cfg(unix)]
fn silence_stderr<F, R>(f: F) -> R
where
    F: FnOnce() -> R,
{
    use std::os::fd::AsRawFd;
    let saved = unsafe { libc::dup(2) };
    let devnull = std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/null")
        .ok();
    if let Some(ref d) = devnull {
        if saved >= 0 {
            unsafe {
                libc::dup2(d.as_raw_fd(), 2);
            }
        }
    }
    let out = f();
    if saved >= 0 {
        unsafe {
            libc::dup2(saved, 2);
            libc::close(saved);
        }
    }
    drop(devnull);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silent_bank_is_disabled() {
        let bank = SampleBank::silent();
        assert!(!bank.enabled);
    }

    #[test]
    fn silent_bank_play_is_a_noop_and_does_not_panic() {
        let bank = SampleBank::silent();
        bank.play(1);
        bank.play(999);
    }

    #[test]
    fn silent_bank_play_synth_is_a_noop_and_does_not_panic() {
        let bank = SampleBank::silent();
        bank.play_synth(440.0, 60);
    }

    #[test]
    fn silent_bank_play_hit_is_a_noop_across_lanes() {
        let bank = SampleBank::silent();
        for lane in 0..7 {
            bank.play_hit(lane, Some(1), true);
            bank.play_hit(lane, None, true);
            bank.play_hit(lane, None, false);
        }
    }

    #[test]
    fn synth_ping_produces_expected_sample_count() {
        let sr = 44_100;
        let dur = 100u32;
        let samples = synth_ping(440.0, dur, sr);
        let expected = (sr * dur / 1000) as usize;
        assert_eq!(samples.len(), expected);
    }

    #[test]
    fn synth_ping_starts_near_zero_and_decays_below_a_hundredth() {
        let sr = 44_100;
        let samples = synth_ping(440.0, 200, sr);
        assert_eq!(samples.len(), (sr * 200 / 1000) as usize);
        assert!(samples[0].abs() < 0.5);
        let tail = samples.last().copied().unwrap().abs();
        assert!(
            tail < 0.05,
            "expected the tail of the exponential decay to be well below the head; got {}",
            tail
        );
    }

    #[test]
    fn synth_ping_stays_within_the_headroom() {
        let samples = synth_ping(440.0, 100, 44_100);
        for s in samples {
            assert!(
                s.abs() < 1.0,
                "synth_ping should stay in [-1, 1); got {}",
                s
            );
        }
    }

    #[test]
    fn synth_ping_zero_duration_is_empty() {
        assert!(synth_ping(440.0, 0, 44_100).is_empty());
    }

    #[test]
    fn lane_pitches_span_seven_lanes() {
        assert_eq!(LANE_PITCHES.len(), 7);
        for pair in LANE_PITCHES.windows(2) {
            assert!(pair[1] > pair[0], "lane pitches should climb monotonically");
        }
    }

    #[test]
    fn decode_reports_error_for_missing_file() {
        let missing = PathBuf::from("/nonexistent-tapline-audio-test-xyz.wav");
        assert!(decode(&missing).is_err());
    }
}

#[cfg(not(unix))]
fn silence_stderr<F, R>(f: F) -> R
where
    F: FnOnce() -> R,
{
    f()
}
