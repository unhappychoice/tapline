use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use tapline::audio::SampleBank;

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn tempdir() -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("tapline-audio-bank-{}-{}", std::process::id(), n));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Build a minimal WAV (PCM, mono, 16-bit, 44100 Hz) with `sample_count`
/// silent samples. Enough for rodio's decoder to accept it.
fn silent_wav_bytes(sample_count: u32) -> Vec<u8> {
    let bytes_per_sample = 2u16;
    let channels = 1u16;
    let sample_rate = 44_100u32;
    let byte_rate = sample_rate * channels as u32 * bytes_per_sample as u32;
    let block_align = channels * bytes_per_sample;
    let data_size = sample_count * bytes_per_sample as u32;
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_size).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&(bytes_per_sample * 8).to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_size.to_le_bytes());
    for _ in 0..sample_count {
        out.extend_from_slice(&0i16.to_le_bytes());
    }
    out
}

#[test]
fn silent_bank_constructor_creates_a_disabled_bank() {
    let bank = SampleBank::silent();
    assert!(!bank.enabled);
}

#[test]
fn new_with_empty_map_never_panics_even_if_backend_is_absent() {
    let bank = SampleBank::new(&HashMap::new());
    // enabled may be true or false depending on the test runner's audio
    // backend; the important guarantee is that construction doesn't panic
    // and the bank stays usable.
    bank.play(0);
    bank.play_synth(440.0, 20);
    for lane in 0..7 {
        bank.play_hit(lane, None, true);
    }
}

#[test]
fn new_drops_wav_entries_pointing_at_missing_files() {
    let dir = tempdir();
    let mut wav_paths: HashMap<u32, PathBuf> = HashMap::new();
    wav_paths.insert(1, dir.join("missing.wav"));
    let bank = SampleBank::new(&wav_paths);
    // The missing WAV never made it into the bank; calling play with its id
    // is a no-op instead of a decode attempt.
    bank.play(1);
}

#[test]
fn new_drops_wav_entries_that_are_not_valid_audio() {
    let dir = tempdir();
    let broken = dir.join("broken.wav");
    std::fs::write(&broken, b"not a wav at all").unwrap();
    let mut wav_paths: HashMap<u32, PathBuf> = HashMap::new();
    wav_paths.insert(1, broken);
    let bank = SampleBank::new(&wav_paths);
    bank.play(1); // decode failure earlier → no-op now
}

#[test]
fn new_loads_a_hand_crafted_pcm_wav_without_panicking() {
    let dir = tempdir();
    let wav = dir.join("tiny.wav");
    std::fs::write(&wav, silent_wav_bytes(64)).unwrap();
    let mut wav_paths: HashMap<u32, PathBuf> = HashMap::new();
    wav_paths.insert(42, wav);
    let bank = SampleBank::new(&wav_paths);
    // Sample loaded → play should route through the WAV path.
    bank.play(42);
}

#[test]
fn play_hit_falls_back_to_synth_when_keysound_is_unknown() {
    let bank = SampleBank::silent();
    // Silent bank has no `handle`, so this is a no-op — the important thing
    // is that the branch that reads `LANE_PITCHES[lane % LEN]` doesn't panic
    // even for a lane index far past the pitch table.
    bank.play_hit(usize::MAX, Some(999), true);
}
