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
fn duplicate_channel_lines_preserve_notes_at_different_positions() {
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

#[test]
fn lntype_two_merges_runs_and_preserves_the_first_keysound_on_both_sides() {
    let path = write_text("#BPM 60\n#LNTYPE 2\n#00151:0102030000040000\n#00161:0102030000040000\n");
    let chart = bms::load(&path, 0.0).unwrap();
    for notes in [&chart.notes, &chart.p2_notes] {
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[0].time_ms, 4000.0);
        assert_eq!(notes[0].end_ms, Some(5500.0));
        assert_eq!(notes[0].keysound, Some(1));
        assert_eq!(notes[1].time_ms, 6500.0);
        assert_eq!(notes[1].end_ms, Some(7000.0));
        assert_eq!(notes[1].keysound, Some(4));
    }
}

#[test]
fn lntype_two_preserves_base62_keysounds_on_both_sides() {
    let path = write_text(
        "#BASE 62\n#BPM 60\n#LNTYPE 2\n#00151:0A000a00\n#00161:0A000a00\n#00197:0a\n#00198:FF\n",
    );
    let chart = bms::load(&path, 0.0).unwrap();
    for notes in [&chart.notes, &chart.p2_notes] {
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[0].keysound, Some(10));
        assert_eq!(notes[0].time_ms, 4000.0);
        assert_eq!(notes[0].end_ms, Some(5000.0));
        assert_eq!(notes[1].keysound, Some(36));
        assert_eq!(notes[1].time_ms, 6000.0);
        assert_eq!(notes[1].end_ms, Some(7000.0));
    }
    assert_eq!(chart.volume_events.len(), 2);
    assert_eq!(chart.volume_events[0].volume, 10);
    assert_eq!(chart.volume_events[1].volume, 255);
}

#[test]
fn lntype_two_merges_across_measures_with_different_resolutions_in_time_order() {
    let path = write_text("#BPM 60\n#LNTYPE 2\n#00251:02020300\n#00151:0001\n#00351:04\n");
    let chart = bms::load(&path, 250.0).unwrap();
    assert_eq!(chart.notes.len(), 2);
    assert_eq!(chart.notes[0].time_ms, 6250.0);
    assert_eq!(chart.notes[0].end_ms, Some(11250.0));
    assert_eq!(chart.notes[0].keysound, Some(1));
    assert_eq!(chart.notes[1].time_ms, 12250.0);
    assert_eq!(chart.notes[1].end_ms, Some(16250.0));
}

#[test]
fn lntype_two_does_not_bridge_missing_measures_or_lanes() {
    let path = write_text("#BPM 60\n#LNTYPE 2\n#00151:01\n#00351:02\n#00252:03\n#00451:\n");
    let chart = bms::load(&path, 0.0).unwrap();
    let spans: Vec<_> = chart
        .notes
        .iter()
        .map(|n| (n.lane, n.time_ms, n.end_ms))
        .collect();
    assert_eq!(
        spans,
        vec![
            (0, 4000.0, Some(8000.0)),
            (1, 8000.0, Some(12000.0)),
            (0, 12000.0, Some(16000.0))
        ]
    );
}

#[test]
fn lntype_two_endpoints_follow_measure_scales_bpm_changes_and_stops() {
    let path = write_text(
        "#BPM 60\n#LNTYPE 2\n#BPM01 120\n#STOP01 48\n#00102:0.5\n#00108:00010000\n#00109:00000100\n#00151:01020300\n",
    );
    let chart = bms::load(&path, 0.0).unwrap();
    assert_eq!(chart.notes.len(), 1);
    assert_eq!(chart.notes[0].time_ms, 4000.0);
    assert_eq!(chart.notes[0].end_ms, Some(5500.0));
}

#[test]
fn default_and_explicit_lntype_one_keep_pairing_slots_and_ignore_unmatched_starts() {
    for header in ["", "#LNTYPE 1\n"] {
        let path = write_text(&format!("#BPM 60\n{header}#00151:010203040500\n"));
        let chart = bms::load(&path, 0.0).unwrap();
        assert_eq!(chart.notes.len(), 2);
        assert_eq!(chart.notes[0].time_ms, 4000.0);
        assert!((chart.notes[0].end_ms.unwrap() - (4000.0 + 4000.0 / 6.0)).abs() < 1e-6);
        assert_eq!(chart.notes[1].end_ms, Some(6000.0));
        assert_eq!(chart.notes[1].keysound, Some(3));
    }
}

#[test]
fn duplicate_normal_notes_use_the_last_keysound_on_both_sides() {
    for channel in ["11", "21"] {
        let path = write_text(&format!("#BPM 60\n#001{channel}:01\n#001{channel}:02\n"));
        let chart = bms::load(&path, 0.0).unwrap();
        let notes = if channel == "11" {
            &chart.notes
        } else {
            &chart.p2_notes
        };
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].keysound, Some(2));
        assert_eq!(notes[0].time_ms, 4000.0);
    }
}

#[test]
fn duplicate_positions_merge_across_different_slot_counts() {
    let path = write_text("#BPM 60\n#00111:010203\n#00111:000004000500\n#00112:06\n#00211:07\n");
    let chart = bms::load(&path, 0.0).unwrap();
    let keysounds: Vec<_> = chart
        .notes
        .iter()
        .map(|note| note.keysound.unwrap())
        .collect();
    assert_eq!(keysounds, vec![1, 6, 4, 5, 7]);
}

#[test]
fn duplicate_long_note_endpoints_merge_before_pairing_on_both_sides() {
    for channel in ["51", "61"] {
        let path = write_text(&format!(
            "#BPM 60\n#001{channel}:0101\n#001{channel}:02000200\n"
        ));
        let chart = bms::load(&path, 0.0).unwrap();
        let notes = if channel == "51" {
            &chart.notes
        } else {
            &chart.p2_notes
        };
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].keysound, Some(2));
        assert_eq!(notes[0].time_ms, 4000.0);
        assert_eq!(notes[0].end_ms, Some(6000.0));
    }
}

#[test]
fn duplicate_landmines_use_the_last_hex_damage_on_both_sides() {
    for channel in ["D1", "E1"] {
        let path = write_text(&format!("#BPM 60\n#001{channel}:0A\n#001{channel}:FF00\n"));
        let chart = bms::load(&path, 0.0).unwrap();
        let mines = if channel == "D1" {
            &chart.mines
        } else {
            &chart.p2_mines
        };
        assert_eq!(mines.len(), 1);
        assert_eq!(mines[0].damage, 255);
        assert_eq!(mines[0].time_ms, 4000.0);
    }
}

#[test]
fn duplicate_lnobj_objects_merge_before_promotion() {
    let path = write_text("#BPM 60\n#LNOBJ ZZ\n#00111:0101\n#00111:0200ZZ00\n");
    let chart = bms::load(&path, 0.0).unwrap();
    assert_eq!(chart.notes.len(), 1);
    assert_eq!(chart.notes[0].keysound, Some(2));
    assert_eq!(chart.notes[0].end_ms, Some(6000.0));
}

#[test]
fn duplicate_bgm_lines_keep_simultaneous_sounds() {
    let path = write_text("#BPM 60\n#00101:01\n#00101:0200\n");
    let chart = bms::load(&path, 0.0).unwrap();
    assert_eq!(chart.bgm.len(), 2);
    assert_eq!(chart.bgm[0].keysound, 1);
    assert_eq!(chart.bgm[1].keysound, 2);
    assert!(chart.bgm.iter().all(|event| event.time_ms == 4000.0));
}

#[test]
fn lntype_two_uses_pms_lanes_without_duplicate_p2_notes() {
    let path = write_text("#BPM 60\n#LNTYPE 2\n#00162:01020300\n");
    let pms_path = path.with_extension("pms");
    std::fs::rename(path, &pms_path).unwrap();
    let chart = bms::load(&pms_path, 0.0).unwrap();
    assert_eq!(chart.notes.len(), 1);
    assert_eq!(chart.notes[0].lane, 5);
    assert_eq!(chart.notes[0].time_ms, 4000.0);
    assert_eq!(chart.notes[0].end_ms, Some(7000.0));
    assert!(chart.p2_notes.is_empty());
}

#[test]
fn lntype_two_duplicate_slots_preserve_the_last_keysound() {
    let path = write_text("#BPM 60\n#LNTYPE 2\n#00151:01010000\n#00151:02000000\n");
    let chart = bms::load(&path, 0.0).unwrap();
    assert_eq!(chart.notes.len(), 1);
    assert_eq!(chart.notes[0].keysound, Some(2));
    assert_eq!(chart.notes[0].time_ms, 4000.0);
    assert_eq!(chart.notes[0].end_ms, Some(6000.0));
}

#[test]
fn lntype_two_endpoints_include_millisecond_stops() {
    let path = write_text("#BPM 60\n#LNTYPE 2\n#STP 001.500 750\n#00151:01020300\n");
    let chart = bms::load(&path, 250.0).unwrap();
    assert_eq!(chart.notes.len(), 1);
    assert_eq!(chart.notes[0].time_ms, 4250.0);
    assert_eq!(chart.notes[0].end_ms, Some(8000.0));
    assert_eq!(chart.notes[0].keysound, Some(1));
}

#[test]
fn volume_events_follow_millisecond_stops() {
    let path = write_text("#BPM 120\n#STP 001.500 750\n#00197:400080FF\n#00198:0080\n");
    let chart = bms::load(&path, 100.0).unwrap();
    let times: Vec<_> = chart
        .volume_events
        .iter()
        .map(|event| event.time_ms)
        .collect();
    assert_eq!(times, vec![2100.0, 3850.0, 3850.0, 4350.0]);
}
