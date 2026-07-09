use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use tapline::bms;

static COUNTER: AtomicU32 = AtomicU32::new(0);

fn tempdir() -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("tapline-bms-edge-{}-{}", std::process::id(), n));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_bytes(bytes: &[u8]) -> PathBuf {
    let dir = tempdir();
    let path = dir.join("song.bms");
    std::fs::write(&path, bytes).unwrap();
    path
}

fn write_text(text: &str) -> PathBuf {
    write_bytes(text.as_bytes())
}

#[test]
fn empty_file_loads_with_defaults_and_no_notes() {
    let path = write_text("");
    let chart = bms::load(&path, 0.0).unwrap();
    assert_eq!(chart.title, "");
    assert_eq!(chart.artist, "");
    assert_eq!(chart.bpm, 130.0);
    assert!(chart.notes.is_empty());
    assert!(chart.bgm.is_empty());
    assert!(chart.playlevel.is_none());
    assert!(chart.difficulty.is_none());
}

#[test]
fn header_only_file_still_reports_metadata() {
    let path = write_text(
        "\
#TITLE Header Only
#ARTIST Someone
#BPM 200
#PLAYLEVEL 8
#DIFFICULTY 4
",
    );
    let m = bms::read_meta(&path).unwrap();
    assert_eq!(m.title, "Header Only");
    assert_eq!(m.artist, "Someone");
    assert_eq!(m.bpm, 200.0);
    assert_eq!(m.playlevel, Some(8));
    assert_eq!(m.difficulty, Some(4));
}

#[test]
fn utf8_bom_is_stripped_before_parsing() {
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice(b"#TITLE BOM Test\n#BPM 140\n#00111:0100\n");
    let path = write_bytes(&bytes);
    let m = bms::read_meta(&path).unwrap();
    assert_eq!(m.title, "BOM Test", "BOM should not leak into the title");
    assert_eq!(m.bpm, 140.0);
}

#[test]
fn shift_jis_bytes_decode_into_japanese_headers() {
    // "#TITLE 挑戦\n#ARTIST 太郎\n" in Shift-JIS
    let title_kanji = &[0x92, 0xA7, 0x90, 0xED]; // 挑戦
    let artist_kanji = &[0x91, 0xBE, 0x98, 0x59]; // 太郎
    let mut bytes: Vec<u8> = Vec::new();
    bytes.extend_from_slice(b"#TITLE ");
    bytes.extend_from_slice(title_kanji);
    bytes.push(b'\n');
    bytes.extend_from_slice(b"#ARTIST ");
    bytes.extend_from_slice(artist_kanji);
    bytes.push(b'\n');
    bytes.extend_from_slice(b"#BPM 145\n");
    let path = write_bytes(&bytes);
    let m = bms::read_meta(&path).unwrap();
    assert_eq!(m.title, "挑戦");
    assert_eq!(m.artist, "太郎");
    assert_eq!(m.bpm, 145.0);
}

#[test]
fn lines_not_starting_with_hash_are_treated_as_comments() {
    let path = write_text(
        "\
This is a comment.
So is this.
#TITLE Comments Are Fine
    // more chatter
#BPM 128
#00111:0100
",
    );
    let m = bms::read_meta(&path).unwrap();
    assert_eq!(m.title, "Comments Are Fine");
    assert_eq!(m.bpm, 128.0);
}

#[test]
fn duplicate_channel_lines_stack_their_notes() {
    let path = write_text(
        "\
#TITLE Dup
#BPM 60
#WAV01 kick.wav
#00111:0100
#00111:0001
",
    );
    let chart = bms::load(&path, 0.0).unwrap();
    // Two note events, one from each line, at different offsets.
    assert_eq!(chart.notes.len(), 2);
    assert!(
        chart.notes[0].time_ms < chart.notes[1].time_ms,
        "duplicate lines should still be materialised in time order"
    );
}

#[test]
fn single_slot_data_places_a_note_at_the_measure_start() {
    let path = write_text(
        "\
#TITLE Single
#BPM 60
#WAV01 kick.wav
#00111:01
",
    );
    let chart = bms::load(&path, 0.0).unwrap();
    assert_eq!(chart.notes.len(), 1);
    // BPM 60, measure = 4000 ms, slot 0 → t = 4000 * 1 + 0 = 4000
    assert!(
        (chart.notes[0].time_ms - 4000.0).abs() < 1e-6,
        "expected t=4000ms, got {}",
        chart.notes[0].time_ms
    );
}

#[test]
fn wav_id_zero_is_ignored_in_channel_data() {
    let path = write_text(
        "\
#TITLE Zero Slots
#BPM 60
#WAV01 kick.wav
#00111:00000000
",
    );
    let chart = bms::load(&path, 0.0).unwrap();
    assert!(chart.notes.is_empty());
}

#[test]
fn zero_bpm_does_not_panic_but_yields_infinite_duration() {
    let path = write_text(
        "\
#TITLE Zero BPM
#BPM 0
#WAV01 kick.wav
#00111:0100
",
    );
    let chart = bms::load(&path, 0.0).unwrap();
    // measure_ms = 4 * 60_000 / 0.0 = +∞ → duration and note times are infinite.
    assert!(chart.duration_ms.is_infinite() || chart.duration_ms.is_nan());
}

#[test]
fn bgm_channel_01_populates_the_bgm_list_not_the_note_list() {
    let path = write_text(
        "\
#TITLE BGM
#BPM 60
#WAV0A hat.wav
#00101:0A000A00
#00111:0100
",
    );
    let chart = bms::load(&path, 0.0).unwrap();
    assert_eq!(chart.bgm.len(), 2);
    assert_eq!(chart.notes.len(), 1);
    assert_eq!(chart.bgm[0].keysound, 10);
}

#[test]
fn wav_references_that_do_not_exist_are_dropped_from_wav_paths() {
    let path = write_text(
        "\
#TITLE Missing WAVs
#BPM 60
#WAV01 kick.wav
#WAV02 snare.wav
#00111:0100
",
    );
    let chart = bms::load(&path, 0.0).unwrap();
    // Neither kick.wav nor snare.wav exists next to the .bms file.
    assert!(chart.wav_paths.is_empty());
}
