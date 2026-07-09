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

#[cfg(not(unix))]
fn silence_stderr<F, R>(f: F) -> R
where
    F: FnOnce() -> R,
{
    f()
}
