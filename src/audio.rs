use rayon::prelude::*;
use rodio::buffer::SamplesBuffer;
use rodio::{Decoder, OutputStream, OutputStreamHandle, Source};
use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone)]
struct DecodedSample {
    channels: u16,
    sample_rate: u32,
    data: Arc<Vec<f32>>,
}

/// Zero-copy source over a shared sample buffer. Cloning it (and thus the
/// backing Arc) is O(1), which lets us stream the same keysound to the mixer
/// many times per song without cloning the PCM data.
struct SharedSamples {
    data: Arc<Vec<f32>>,
    pos: usize,
    channels: u16,
    sample_rate: u32,
}

impl Iterator for SharedSamples {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        let s = *self.data.get(self.pos)?;
        self.pos += 1;
        Some(s)
    }
}

impl Source for SharedSamples {
    fn current_frame_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> u16 {
        self.channels
    }
    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

pub struct SampleBank {
    _stream: Option<OutputStream>,
    handle: Option<OutputStreamHandle>,
    samples: HashMap<u32, DecodedSample>,
    pub enabled: bool,
    /// Number of `wav_paths` entries that failed to decode. Surfaced on the
    /// intro screen so charts that end up unexpectedly silent are diagnosable.
    pub decode_failures: usize,
}

impl SampleBank {
    pub fn new(wav_paths: &HashMap<u32, PathBuf>) -> Self {
        let (stream, handle, enabled) = silence_stderr(|| match OutputStream::try_default() {
            Ok((s, h)) => (Some(s), Some(h), true),
            Err(_) => (None, None, false),
        });
        let (samples, failures) = decode_all(wav_paths);
        Self {
            _stream: stream,
            handle,
            samples,
            enabled,
            decode_failures: failures,
        }
    }

    pub fn with_settings(
        wav_paths: &HashMap<u32, PathBuf>,
        settings: &HashMap<u32, crate::sample_settings::SampleSettings>,
    ) -> Self {
        let mut bank = Self::new(wav_paths);
        bank.samples.iter_mut().for_each(|(id, sample)| {
            if let Some(settings) = settings.get(id) {
                apply_settings(sample, settings);
            }
        });
        bank
    }

    pub fn silent() -> Self {
        Self {
            _stream: None,
            handle: None,
            samples: HashMap::new(),
            enabled: false,
            decode_failures: 0,
        }
    }

    pub fn chart_duration_ms(&self, chart: &crate::chart::Chart, auto_ks: bool) -> f64 {
        if !self.enabled {
            return chart.duration_ms;
        }
        let notes = chart.notes.iter().map(|note| {
            let delay = if auto_ks {
                0.0
            } else {
                crate::game::WINDOW_GOOD
            };
            (note.time_ms + delay, note.keysound)
        });
        let bgm = chart
            .bgm
            .iter()
            .map(|event| (event.time_ms, Some(event.keysound)));
        let audio_end = notes
            .chain(bgm)
            .map(|(time, id)| {
                id.and_then(|id| self.samples.get(&id))
                    .map(|sample| {
                        time + sample.data.len() as f64 * 1000.0
                            / sample.channels as f64
                            / sample.sample_rate as f64
                            + 32.0
                    })
                    .unwrap_or(chart.duration_ms)
            })
            .reduce(f64::max);
        let judgment_end = chart
            .notes
            .iter()
            .map(|note| note.end_ms.unwrap_or(note.time_ms) + crate::game::MISS_AFTER)
            .chain(
                chart
                    .mines
                    .iter()
                    .map(|mine| mine.time_ms + crate::game::MINE_WINDOW),
            )
            .fold(0.0, f64::max);
        // Leave one frame after the strict miss boundary for judgment processing.
        audio_end
            .map(|end| end.max(judgment_end + 32.0))
            .unwrap_or(chart.duration_ms)
    }

    pub fn sample_count(&self) -> usize {
        self.samples.len()
    }

    pub fn play(&self, id: u32) {
        let Some(handle) = &self.handle else {
            return;
        };
        let Some(s) = self.samples.get(&id) else {
            return;
        };
        let source = SharedSamples {
            data: Arc::clone(&s.data),
            pos: 0,
            channels: s.channels,
            sample_rate: s.sample_rate,
        };
        let _ = handle.play_raw(source);
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
        match self.hit_sound(keysound, synth_mode) {
            HitSound::Synth => {
                let f = LANE_PITCHES[lane % LANE_PITCHES.len()];
                self.play_synth(f, 80);
            }
            HitSound::Sample(id) => self.play(id),
            HitSound::Silent => {}
        }
    }

    fn hit_sound(&self, keysound: Option<u32>, synth_mode: bool) -> HitSound {
        if synth_mode || self.samples.is_empty() {
            return HitSound::Synth;
        }
        match keysound.filter(|id| self.samples.contains_key(id)) {
            Some(id) => HitSound::Sample(id),
            None => HitSound::Silent,
        }
    }
}

#[derive(Debug, PartialEq)]
enum HitSound {
    Synth,
    Sample(u32),
    Silent,
}

fn decode_all(wav_paths: &HashMap<u32, PathBuf>) -> (HashMap<u32, DecodedSample>, usize) {
    decode_all_with(wav_paths, decode)
}

fn decode_all_with(
    wav_paths: &HashMap<u32, PathBuf>,
    decode_sample: impl Fn(&PathBuf) -> anyhow::Result<DecodedSample> + Sync,
) -> (HashMap<u32, DecodedSample>, usize) {
    let mut ids_by_path: HashMap<&PathBuf, Vec<u32>> = HashMap::new();
    wav_paths.iter().for_each(|(&id, path)| {
        ids_by_path.entry(path).or_default().push(id);
    });
    ids_by_path
        .par_iter()
        .map(|(path, ids)| (ids, decode_sample(path)))
        .collect::<Vec<_>>()
        .into_iter()
        .fold(
            (HashMap::new(), 0),
            |(mut samples, failures), (ids, result)| match result {
                Ok(sample) => {
                    samples.extend(ids.iter().map(|&id| (id, sample.clone())));
                    (samples, failures)
                }
                Err(_) => (samples, failures + ids.len()),
            },
        )
}

fn apply_settings(sample: &mut DecodedSample, settings: &crate::sample_settings::SampleSettings) {
    let rate = settings.frequency.unwrap_or(sample.sample_rate) as f64
        * 2.0_f64.powf((settings.pitch.unwrap_or(60) as f64 - 60.0) / 12.0);
    sample.sample_rate = rate.round().clamp(1.0, u32::MAX as f64) as u32;
    let (left, right) = settings.gains();
    if left != 1.0 || right != 1.0 {
        let channels = sample.channels as usize;
        let data = sample
            .data
            .chunks(channels)
            .flat_map(|frame| {
                if channels == 1 {
                    vec![frame[0] * left, frame[0] * right]
                } else {
                    frame
                        .iter()
                        .enumerate()
                        .map(|(i, value)| value * if i % 2 == 0 { left } else { right })
                        .collect()
                }
            })
            .collect();
        sample.data = Arc::new(data);
        sample.channels = sample.channels.max(2);
    }
    if let Some(length) = settings.length.filter(|&length| length > 0) {
        // WAVCMD time units are half milliseconds; truncate only on frame boundaries.
        let frames = u64::from(sample.sample_rate) * u64::from(length) / 2000;
        let count = frames.saturating_mul(u64::from(sample.channels));
        if count < sample.data.len() as u64 {
            sample.data = Arc::new(sample.data[..count as usize].to_vec());
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
        data: Arc::new(data),
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

    fn duration_bank() -> SampleBank {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("tapline-duration-{}-{id}.wav", std::process::id()));
        let mut wav = b"RIFF".to_vec();
        let data_size = 8000u32 * 2 * 2 * 10;
        wav.extend((36 + data_size).to_le_bytes());
        wav.extend(b"WAVEfmt ");
        wav.extend(16u32.to_le_bytes());
        wav.extend(1u16.to_le_bytes());
        wav.extend(2u16.to_le_bytes());
        wav.extend(8000u32.to_le_bytes());
        wav.extend(32000u32.to_le_bytes());
        wav.extend(4u16.to_le_bytes());
        wav.extend(16u16.to_le_bytes());
        wav.extend(b"data");
        wav.extend(data_size.to_le_bytes());
        wav.resize(44 + data_size as usize, 0);
        std::fs::write(&path, wav).unwrap();
        let sample = decode(&path).unwrap();
        std::fs::remove_file(path).unwrap();
        SampleBank {
            enabled: true,
            samples: HashMap::from([(1, sample)]),
            ..SampleBank::silent()
        }
    }

    #[test]
    fn duration_waits_for_decoded_stereo_bgm_and_removes_empty_measures() {
        let bank = duration_bank();
        let path =
            std::env::temp_dir().join(format!("tapline-duration-{}.bms", std::process::id()));
        std::fs::write(&path, "#BPM 60\n#00101:01\n#02011:00\n").unwrap();
        let mut chart = crate::bms::load(&path, 0.0).unwrap();
        std::fs::remove_file(path).unwrap();
        assert!(chart.duration_ms > 14032.0);
        assert_eq!(bank.chart_duration_ms(&chart, false), 14032.0);
        chart.duration_ms = 60000.0;
        assert_eq!(bank.chart_duration_ms(&chart, false), 14032.0);
    }

    #[test]
    fn duration_covers_earlier_samples_late_hits_and_long_note_judgments() {
        let mut bank = duration_bank();
        let mut chart = crate::chart::Chart {
            duration_ms: 30000.0,
            notes: vec![crate::chart::Note {
                time_ms: 4000.0,
                keysound: Some(1),
                ..Default::default()
            }],
            bgm: vec![crate::chart::BgmEvent {
                time_ms: 0.0,
                keysound: 1,
            }],
            ..Default::default()
        };
        assert_eq!(bank.chart_duration_ms(&chart, false), 14172.0);
        assert_eq!(bank.chart_duration_ms(&chart, true), 14032.0);
        chart.notes[0].end_ms = Some(20000.0);
        assert_eq!(bank.chart_duration_ms(&chart, true), 20192.0);
        chart.mines.push(crate::chart::Mine {
            time_ms: 25000.0,
            ..Default::default()
        });
        assert_eq!(bank.chart_duration_ms(&chart, true), 25072.0);
        chart.notes[0].time_ms = 0.0;
        chart.notes[0].end_ms = None;
        chart.mines.clear();
        chart.bgm[0].time_ms = 1000.0;
        assert_eq!(bank.chart_duration_ms(&chart, true), 11032.0);
        bank.samples.insert(
            2,
            DecodedSample {
                channels: 1,
                sample_rate: 8000,
                data: Arc::new(vec![0.0; 800]),
            },
        );
        chart.notes[0].time_ms = 9000.0;
        chart.notes[0].keysound = Some(2);
        assert_eq!(bank.chart_duration_ms(&chart, true), 11032.0);
    }

    #[test]
    fn duration_falls_back_for_disabled_missing_or_unused_samples() {
        let mut bank = duration_bank();
        let mut chart = crate::chart::Chart {
            duration_ms: 30000.0,
            ..Default::default()
        };
        assert_eq!(bank.chart_duration_ms(&chart, false), 30000.0);
        chart.bgm.push(crate::chart::BgmEvent {
            time_ms: 4000.0,
            keysound: 1,
        });
        bank.enabled = false;
        assert_eq!(bank.chart_duration_ms(&chart, false), 30000.0);
        bank.enabled = true;
        chart.bgm[0].keysound = 2;
        assert_eq!(bank.chart_duration_ms(&chart, false), 30000.0);
        chart.notes.push(crate::chart::Note {
            time_ms: 4000.0,
            keysound: Some(1),
            ..Default::default()
        });
        assert_eq!(bank.chart_duration_ms(&chart, false), 30000.0);
        chart.notes[0].keysound = None;
        assert_eq!(bank.chart_duration_ms(&chart, false), 30000.0);
        bank.samples.clear();
        assert_eq!(bank.chart_duration_ms(&chart, false), 30000.0);
    }

    #[test]
    fn settings_apply_pan_volume_frequency_pitch_and_frame_aligned_length() {
        let mut sample = DecodedSample {
            channels: 1,
            sample_rate: 1000,
            data: Arc::new(vec![1.0; 20]),
        };
        let settings = crate::sample_settings::SampleSettings {
            pan: 10000,
            attenuation: -2000,
            frequency: Some(2000),
            pitch: Some(72),
            volume: Some(50),
            length: Some(2),
        };
        apply_settings(&mut sample, &settings);
        assert_eq!(sample.sample_rate, 4000);
        assert_eq!(sample.channels, 2);
        assert_eq!(sample.data.len(), 8);
        sample.data.chunks(2).for_each(|frame| {
            assert_eq!(frame[0], 0.0);
            assert!((frame[1] - 0.05).abs() < 1e-6);
        });
    }

    #[test]
    fn duration_uses_processed_sample_rate_channels_and_length() {
        let mut bank = duration_bank();
        apply_settings(
            bank.samples.get_mut(&1).unwrap(),
            &crate::sample_settings::SampleSettings {
                frequency: Some(16000),
                pitch: Some(72),
                length: Some(200),
                ..Default::default()
            },
        );
        let chart = crate::chart::Chart {
            duration_ms: 30000.0,
            bgm: vec![crate::chart::BgmEvent {
                time_ms: 1000.0,
                keysound: 1,
            }],
            ..Default::default()
        };
        assert_eq!(bank.chart_duration_ms(&chart, true), 1132.0);
    }

    #[test]
    fn stereo_pan_preserves_channels_and_does_not_modify_shared_original() {
        let original = Arc::new(vec![0.5, 0.25]);
        let mut sample = DecodedSample {
            channels: 2,
            sample_rate: 44100,
            data: original.clone(),
        };
        let settings = crate::sample_settings::SampleSettings {
            pan: -2000,
            ..Default::default()
        };
        apply_settings(&mut sample, &settings);
        assert_eq!(sample.data[0], 0.5);
        assert!((sample.data[1] - 0.025).abs() < 1e-6);
        assert_eq!(*original, vec![0.5, 0.25]);
        let settings = crate::sample_settings::SampleSettings {
            attenuation: -10000,
            ..Default::default()
        };
        apply_settings(&mut sample, &settings);
        assert_eq!(*sample.data, vec![0.0, 0.0]);
    }

    #[test]
    fn hit_sound_selection_uses_decoded_samples_and_synth_override() {
        let dir = std::env::temp_dir().join(format!("tapline-hit-sound-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let valid = dir.join("valid.wav");
        let broken = dir.join("broken.wav");
        let mut wav = b"RIFF".to_vec();
        wav.extend_from_slice(&38u32.to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&44100u32.to_le_bytes());
        wav.extend_from_slice(&88200u32.to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&2u32.to_le_bytes());
        wav.extend_from_slice(&0i16.to_le_bytes());
        std::fs::write(&valid, wav).unwrap();
        std::fs::write(&broken, b"not audio").unwrap();

        [
            vec![],
            vec![(1, broken.clone())],
            vec![(1, valid.clone()), (2, broken)],
        ]
        .into_iter()
        .for_each(|paths| {
            let paths = paths.into_iter().collect();
            let (samples, failures) = decode_all(&paths);
            assert_eq!(samples.len() + failures, paths.len());
            let loaded = samples.contains_key(&1);
            assert_eq!(loaded, paths.len() == 2);
            let bank = SampleBank {
                samples,
                ..SampleBank::silent()
            };
            [None, Some(1), Some(999)].into_iter().for_each(|keysound| {
                assert_eq!(bank.hit_sound(keysound, true), HitSound::Synth);
                let expected = match (loaded, keysound) {
                    (false, _) => HitSound::Synth,
                    (true, Some(1)) => HitSound::Sample(1),
                    _ => HitSound::Silent,
                };
                assert_eq!(bank.hit_sound(keysound, false), expected);
            });
        });
        std::fs::remove_dir_all(dir).unwrap();
    }

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
    fn duplicate_paths_decode_once_and_share_pcm_across_all_ids() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let paths = (1..=10)
            .map(|id| (id, PathBuf::from("sample.wav")))
            .collect();
        let calls = AtomicUsize::new(0);
        let (samples, failures) = decode_all_with(&paths, |path| {
            assert_eq!(path, &PathBuf::from("sample.wav"));
            calls.fetch_add(1, Ordering::Relaxed);
            Ok(DecodedSample {
                channels: 2,
                sample_rate: 44_100,
                data: Arc::new(vec![0.25, -0.25]),
            })
        });

        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert_eq!(failures, 0);
        assert_eq!(samples.len(), 10);
        (1..=10).for_each(|id| {
            let sample = &samples[&id];
            assert!(Arc::ptr_eq(&sample.data, &samples[&1].data));
            let source = SharedSamples {
                data: Arc::clone(&sample.data),
                pos: 0,
                channels: sample.channels,
                sample_rate: sample.sample_rate,
            };
            assert_eq!(source.channels(), 2);
            assert_eq!(source.sample_rate(), 44_100);
            assert_eq!(source.collect::<Vec<_>>(), vec![0.25, -0.25]);
        });
    }

    #[test]
    fn duplicate_failures_are_counted_per_id_and_other_paths_still_load() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let paths = HashMap::from([
            (1, PathBuf::from("missing.wav")),
            (2, PathBuf::from("missing.wav")),
            (3, PathBuf::from("valid.wav")),
        ]);
        let calls = AtomicUsize::new(0);
        let (samples, failures) = decode_all_with(&paths, |path| {
            calls.fetch_add(1, Ordering::Relaxed);
            anyhow::ensure!(path == &PathBuf::from("valid.wav"), "missing sample");
            Ok(DecodedSample {
                channels: 1,
                sample_rate: 44_100,
                data: Arc::new(vec![0.5]),
            })
        });

        assert_eq!(calls.load(Ordering::Relaxed), 2);
        assert_eq!(failures, 2);
        assert_eq!(samples.len(), 1);
        assert_eq!(*samples[&3].data, vec![0.5]);
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
