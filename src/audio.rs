use crate::chart::{Chart, VolumeTarget};
use rayon::prelude::*;
use rodio::buffer::SamplesBuffer;
use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink, Source};
use std::cell::RefCell;
use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

pub(crate) fn preview_output(enabled: bool) -> Option<(OutputStream, OutputStreamHandle)> {
    enabled
        .then(|| silence_stderr(OutputStream::try_default))
        .and_then(Result::ok)
}

pub fn playback_gain(chart: &Chart, target: VolumeTarget, time_ms: f64) -> f32 {
    let volume = chart
        .volume_events
        .iter()
        .rev()
        .find(|event| event.target == target && event.time_ms <= time_ms)
        .map_or(255, |event| event.volume);
    chart.vol_wav.unwrap_or(100) as f32 / 100.0 * volume as f32 / 255.0
}

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
    end: usize,
    channels: u16,
    sample_rate: u32,
}

impl Iterator for SharedSamples {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        if self.pos >= self.end {
            return None;
        }
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
    layers: HashMap<u32, Vec<u32>>,
    playing: RefCell<HashMap<u32, Sink>>,
    slices: HashMap<u32, crate::chart::SampleSlice>,
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
            layers: HashMap::new(),
            playing: RefCell::new(HashMap::new()),
            slices: HashMap::new(),
            _stream: stream,
            handle,
            samples,
            enabled,
            decode_failures: failures,
        }
    }

    pub fn with_slices(mut self, slices: &HashMap<u32, crate::chart::SampleSlice>) -> Self {
        self.slices = slices.clone();
        self
    }

    pub fn with_layers(mut self, layers: &HashMap<u32, Vec<u32>>) -> Self {
        self.layers = layers.clone();
        self
    }

    pub fn silent() -> Self {
        Self {
            layers: HashMap::new(),
            playing: RefCell::new(HashMap::new()),
            slices: HashMap::new(),
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
        let invisible_end = chart
            .invisible_notes
            .iter()
            .map(|note| note.time_ms + 2500.0)
            .fold(0.0, f64::max);
        audio_end
            .map(|end| end.max(judgment_end + 32.0).max(invisible_end))
            .unwrap_or(chart.duration_ms)
    }

    pub fn sample_count(&self) -> usize {
        self.samples.len()
    }

    pub fn play(&self, id: u32) {
        self.play_with_gain(id, 1.0);
    }

    pub fn play_with_gain(&self, id: u32, gain: f32) {
        self.sound_ids(&id)
            .iter()
            .for_each(|id| self.play_sample(*id, gain));
    }

    pub fn play_synth(&self, freq: f32, duration_ms: u32) {
        self.play_synth_with_gain(freq, duration_ms, 1.0);
    }

    fn play_synth_with_gain(&self, freq: f32, duration_ms: u32, gain: f32) {
        let Some(handle) = &self.handle else {
            return;
        };
        let sr: u32 = 44_100;
        let data = synth_ping(freq, duration_ms, sr);
        let buf = SamplesBuffer::new(1, sr, data);
        let _ = handle.play_raw(buf.amplify(gain));
    }

    pub fn play_hit(&self, lane: usize, keysound: Option<u32>, synth_mode: bool) {
        self.play_hit_with_gain(lane, keysound, synth_mode, 1.0);
    }

    pub fn play_hit_with_gain(
        &self,
        lane: usize,
        keysound: Option<u32>,
        synth_mode: bool,
        gain: f32,
    ) {
        match self.hit_sound(keysound, synth_mode) {
            HitSound::Synth => {
                let f = LANE_PITCHES[lane % LANE_PITCHES.len()];
                self.play_synth_with_gain(f, 80, gain);
            }
            HitSound::Sample(id) => self.play_with_gain(id, gain),
            HitSound::Silent => {}
        }
    }

    fn sound_ids<'a>(&'a self, id: &'a u32) -> &'a [u32] {
        self.layers
            .get(id)
            .map_or(std::slice::from_ref(id), Vec::as_slice)
    }

    fn play_sample(&self, id: u32, gain: f32) {
        let Some(handle) = &self.handle else {
            return;
        };
        if let Ok(sink) = Sink::try_new(handle) {
            self.append_sample(id, gain, sink);
        }
    }

    fn append_sample(&self, id: u32, gain: f32, sink: Sink) {
        if let Some(source) = self.sample_with_gain(id, gain) {
            sink.append(source);
            self.replace_sample(id, sink);
        }
    }

    fn sample_with_gain(&self, id: u32, gain: f32) -> Option<impl Source<Item = f32> + Send> {
        self.samples
            .get(&id)
            .map(|sample| sample_source(sample, self.slices.get(&id)).amplify(gain))
    }

    fn replace_sample(&self, id: u32, sink: Sink) {
        if let Some(previous) = self.playing.borrow_mut().insert(id, sink) {
            previous.stop();
        }
    }

    fn hit_sound(&self, keysound: Option<u32>, synth_mode: bool) -> HitSound {
        if synth_mode || self.samples.is_empty() {
            return HitSound::Synth;
        }
        match keysound.filter(|id| {
            self.sound_ids(id)
                .iter()
                .any(|id| self.samples.contains_key(id))
        }) {
            Some(id) => HitSound::Sample(id),
            None => HitSound::Silent,
        }
    }
}

fn sample_source(s: &DecodedSample, slice: Option<&crate::chart::SampleSlice>) -> SharedSamples {
    let frame = |ms: f64| (ms / 1000.0 * s.sample_rate as f64).round() as usize;
    let start_frame = slice.map_or(0, |slice| frame(slice.start_ms));
    let end_frame = slice.and_then(|slice| slice.duration_ms.map(|d| frame(slice.start_ms + d)));
    SharedSamples {
        data: Arc::clone(&s.data),
        pos: start_frame
            .saturating_mul(s.channels as usize)
            .min(s.data.len()),
        end: end_frame.map_or(s.data.len(), |f| {
            f.saturating_mul(s.channels as usize).min(s.data.len())
        }),
        channels: s.channels,
        sample_rate: s.sample_rate,
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
    fn duration_leaves_time_for_empty_presses_after_invisible_notes() {
        let bank = duration_bank();
        let chart = crate::chart::Chart {
            bgm: vec![crate::chart::BgmEvent {
                time_ms: 0.0,
                keysound: 1,
            }],
            invisible_notes: vec![crate::chart::InvisibleNote {
                time_ms: 20000.0,
                lane: 0,
                keysound: 1,
            }],
            ..Default::default()
        };
        assert_eq!(bank.chart_duration_ms(&chart, false), 22500.0);
        assert_eq!(bank.chart_duration_ms(&chart, true), 22500.0);
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
    fn playback_gain_defaults_to_unity_and_applies_volwav() {
        let mut chart = Chart::default();
        assert_eq!(playback_gain(&chart, VolumeTarget::Bgm, 0.0), 1.0);
        chart.vol_wav = Some(50);
        [VolumeTarget::Bgm, VolumeTarget::Key]
            .into_iter()
            .for_each(|target| {
                assert_eq!(playback_gain(&chart, target, 0.0), 0.5);
            });
        chart.vol_wav = Some(0);
        assert_eq!(playback_gain(&chart, VolumeTarget::Key, 0.0), 0.0);
    }

    #[test]
    fn parsed_volume_events_change_each_target_from_the_event_time() {
        let path = std::env::temp_dir().join(format!("tapline-volume-{}.bms", std::process::id()));
        std::fs::write(
            &path,
            "#BPM 120\n#VOLWAV 50\n#00197:0040\n#00198:0080\n#00297:FF\n#00398:FF\n",
        )
        .unwrap();
        let chart = crate::bms::load(&path, 100.0).unwrap();
        std::fs::remove_file(path).unwrap();
        [VolumeTarget::Bgm, VolumeTarget::Key]
            .into_iter()
            .for_each(|target| {
                assert_eq!(playback_gain(&chart, target, 3099.0), 0.5);
                let level = if target == VolumeTarget::Bgm {
                    64.0
                } else {
                    128.0
                };
                assert_eq!(playback_gain(&chart, target, 3100.0), 0.5 * level / 255.0);
                assert_eq!(playback_gain(&chart, target, 4000.0), 0.5 * level / 255.0);
            });
        assert_eq!(playback_gain(&chart, VolumeTarget::Bgm, 4100.0), 0.5);
        assert_eq!(
            playback_gain(&chart, VolumeTarget::Key, 4100.0),
            0.5 * 128.0 / 255.0
        );
        assert_eq!(playback_gain(&chart, VolumeTarget::Key, 6100.0), 0.5);
    }

    #[test]
    fn layers_resolve_all_available_slices_even_if_one_is_missing() {
        let (samples, _) = decode_all(&HashMap::from([
            (1, PathBuf::from("tests/fixtures/tone.wav")),
            (2, PathBuf::from("tests/fixtures/tone.wav")),
        ]));
        let bank = SampleBank {
            samples,
            ..SampleBank::silent()
        }
        .with_layers(&HashMap::from([(3, vec![0, 1, 2])]));
        assert_eq!(bank.sound_ids(&3), &[0, 1, 2]);
        assert_eq!(bank.hit_sound(Some(3), false), HitSound::Sample(3));
        assert_eq!(bank.hit_sound(Some(3), true), HitSound::Synth);
        assert_eq!(bank.sound_ids(&1), &[1]);
    }

    #[test]
    fn layered_slices_apply_gain_and_preserve_slice_boundaries() {
        let mut bank = SampleBank::silent()
            .with_layers(&HashMap::from([(3, vec![1, 2])]))
            .with_slices(&HashMap::from([(
                1,
                crate::chart::SampleSlice {
                    start_ms: 1.0,
                    duration_ms: Some(2.0),
                },
            )]));
        [1, 2].into_iter().for_each(|id| {
            bank.samples.insert(
                id,
                DecodedSample {
                    channels: 1,
                    sample_rate: 1000,
                    data: Arc::new(vec![0.0, 0.4, 0.8, 1.0]),
                },
            );
        });
        let output: Vec<Vec<f32>> = bank
            .sound_ids(&3)
            .iter()
            .map(|id| bank.sample_with_gain(*id, 0.5).unwrap().collect())
            .collect();
        assert_eq!(output, [vec![0.2, 0.4], vec![0.0, 0.2, 0.4, 0.5]]);
        assert!(bank.sample_with_gain(99, 0.5).is_none());
    }

    #[test]
    fn shared_slices_replace_playback_but_other_slices_keep_playing() {
        let bank = SampleBank::silent();
        let (first, mut first_output) = Sink::new_idle();
        first.append(SamplesBuffer::new(1, 44100, vec![1.0; 1000]));
        bank.replace_sample(1, first);
        let (other, _other_output) = Sink::new_idle();
        other.append(SamplesBuffer::new(1, 44100, vec![1.0; 1000]));
        bank.replace_sample(2, other);
        let (replacement, _replacement_output) = Sink::new_idle();
        replacement.append(SamplesBuffer::new(1, 44100, vec![0.5; 1000]));
        bank.replace_sample(1, replacement);
        assert_eq!(first_output.next(), None);
        let playing = bank.playing.borrow();
        assert_eq!(playing.len(), 2);
        assert!(!playing[&1].empty());
        assert!(!playing[&2].empty());
    }

    #[test]
    fn slice_source_preserves_stereo_frames_and_clamps_to_audio_length() {
        let sample = DecodedSample {
            channels: 2,
            sample_rate: 1000,
            data: Arc::new((0..20).map(|n| n as f32).collect()),
        };
        let slice = crate::chart::SampleSlice {
            start_ms: 2.0,
            duration_ms: Some(3.0),
        };
        assert_eq!(
            sample_source(&sample, Some(&slice)).collect::<Vec<_>>(),
            (4..10).map(|n| n as f32).collect::<Vec<_>>()
        );
        let tail = crate::chart::SampleSlice {
            start_ms: 8.0,
            duration_ms: None,
        };
        assert_eq!(
            sample_source(&sample, Some(&tail)).collect::<Vec<_>>(),
            vec![16.0, 17.0, 18.0, 19.0]
        );
        let beyond = crate::chart::SampleSlice {
            start_ms: 100.0,
            duration_ms: Some(1.0),
        };
        assert_eq!(sample_source(&sample, Some(&beyond)).count(), 0);
        let clipped = crate::chart::SampleSlice {
            start_ms: 8.0,
            duration_ms: Some(100.0),
        };
        assert_eq!(sample_source(&sample, Some(&clipped)).count(), 4);
        assert_eq!(sample_source(&sample, None).count(), 20);
    }

    #[test]
    fn bmson_fixture_decodes_shared_audio_for_all_slices() {
        let chart =
            crate::bmson::load(std::path::Path::new("tests/fixtures/minimal.bmson"), 0.0).unwrap();
        let (samples, failures) = decode_all(&chart.wav_paths);
        assert_eq!(failures, 0);
        assert_eq!(samples.len(), 3);
        assert!(Arc::ptr_eq(&samples[&0].data, &samples[&1].data));
        let lengths: Vec<_> = (0..3)
            .map(|id| sample_source(&samples[&id], chart.sample_slices.get(&id)).count())
            .collect();
        assert_eq!(lengths, [4000, 2000, 8000]);
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

    fn sustained_sample() -> DecodedSample {
        DecodedSample {
            channels: 1,
            sample_rate: 44_100,
            data: Arc::new(vec![0.5; 44_100]),
        }
    }

    #[test]
    fn replaying_same_id_stops_previous_sink_and_restarts_sample() {
        let mut bank = SampleBank::silent();
        let sample = sustained_sample();
        bank.samples.insert(1, sample);
        let (first, mut first_output) = Sink::new_idle();
        bank.append_sample(1, 1.0, first);
        assert_eq!(first_output.next(), Some(0.5));

        let (second, mut second_output) = Sink::new_idle();
        bank.append_sample(1, 0.5, second);

        assert!(first_output.skip(1000).take(100).all(|s| s == 0.0));
        assert_eq!(second_output.next(), Some(0.25));
        assert_eq!(bank.playing.borrow().len(), 1);
    }

    #[test]
    fn different_ids_keep_playing_even_when_they_share_sample_data() {
        let mut bank = SampleBank::silent();
        let sample = sustained_sample();
        bank.samples.insert(1, sample.clone());
        bank.samples.insert(2, sample);
        let (first, mut first_output) = Sink::new_idle();
        let (second, mut second_output) = Sink::new_idle();

        bank.append_sample(1, 1.0, first);
        bank.append_sample(2, 1.0, second);

        assert_eq!(first_output.next(), Some(0.5));
        assert_eq!(second_output.next(), Some(0.5));
        assert_eq!(bank.playing.borrow().len(), 2);
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
                end: usize::MAX,
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
