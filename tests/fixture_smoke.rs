use std::path::PathBuf;
use tapline::bms;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

#[test]
fn sample_bms_reads_metadata_correctly() {
    let m = bms::read_meta(&fixture("sample.bms")).unwrap();
    assert_eq!(m.title, "Sample Song");
    assert_eq!(m.artist, "tapline test");
    assert_eq!(m.bpm, 140.0);
    assert_eq!(m.playlevel, Some(3));
}

#[test]
fn sample_bms_loads_notes_and_bgm() {
    let chart = bms::load(&fixture("sample.bms"), 0.0).unwrap();
    assert!(!chart.notes.is_empty(), "sample chart should have notes");
    assert!(
        !chart.bgm.is_empty(),
        "sample chart declares channel 01 BGM"
    );
    for pair in chart.notes.windows(2) {
        assert!(
            pair[0].time_ms <= pair[1].time_ms,
            "notes must be time-sorted"
        );
    }
}

#[test]
fn sample_bms_lands_on_a_multi_lane_layout() {
    let chart = bms::load(&fixture("sample.bms"), 0.0).unwrap();
    // The fixture touches channels 11, 12, 13, 14, 15, 18 → lane_count 7.
    assert!(chart.lane_count >= 5, "expected 5- or 7-lane layout");
    for note in &chart.notes {
        assert!(note.lane < chart.lane_count);
    }
}

#[test]
fn sample_bms_wav_declarations_get_dropped_when_files_are_missing() {
    let chart = bms::load(&fixture("sample.bms"), 0.0).unwrap();
    // The sample declares #WAV01/02/03 but ships no actual WAV files.
    assert!(
        chart.wav_paths.is_empty(),
        "expected no WAV paths since fixture ships no audio"
    );
}
