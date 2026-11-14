use crate::bms::{self, ChartMeta};
use crate::chart::difficulty_label;
use anyhow::Result;
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind},
    queue,
    style::{self, Color, Print, ResetColor, SetForegroundColor},
    terminal,
};
use std::io::{Stdout, Write};
use std::path::Path;
use std::time::Duration;

const EXTS: [&str; 5] = ["bms", "bml", "bme", "pms", "bmson"];

pub fn scan(root: &Path) -> Vec<ChartMeta> {
    let mut out = Vec::new();
    walk(root, 0, &mut out);
    out.sort_by(|a, b| {
        a.title
            .to_ascii_lowercase()
            .cmp(&b.title.to_ascii_lowercase())
            .then_with(|| a.path.cmp(&b.path))
    });
    out
}

fn walk(dir: &Path, depth: usize, out: &mut Vec<ChartMeta>) {
    if depth > 5 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let ty = entry.file_type().ok();
        if ty.map(|t| t.is_dir()).unwrap_or(false) {
            walk(&path, depth + 1, out);
        } else if is_bms(&path) {
            let meta = if crate::bmson::is_bmson(&path) {
                crate::bmson::read_meta(&path)
            } else {
                bms::read_meta(&path)
            };
            if let Ok(meta) = meta {
                out.push(meta);
            }
        }
    }
}

fn is_bms(p: &Path) -> bool {
    p.extension()
        .and_then(|s| s.to_str())
        .map(|e| EXTS.iter().any(|x| x.eq_ignore_ascii_case(e)))
        .unwrap_or(false)
}

#[derive(Default)]
pub struct SelectorState {
    query: String,
    lanes: Option<usize>,
    editing: bool,
    cursor: usize,
    scroll: usize,
    indices: Vec<usize>,
}

impl SelectorState {
    fn refresh(&mut self, charts: &[ChartMeta]) {
        let query = self.query.to_lowercase();
        self.indices = charts
            .iter()
            .enumerate()
            .filter(|(_, chart)| {
                self.lanes.is_none_or(|lanes| chart.lane_count == lanes)
                    && [&chart.title, &chart.subtitle, &chart.artist]
                        .iter()
                        .any(|text| text.to_lowercase().contains(&query))
            })
            .map(|(i, _)| i)
            .collect();
        self.cursor = self.cursor.min(self.indices.len().saturating_sub(1));
        self.scroll = self.scroll.min(self.cursor);
    }

    fn selected(&self) -> Option<usize> {
        self.indices.get(self.cursor).copied()
    }

    fn key(&mut self, key: KeyCode, charts: &[ChartMeta]) {
        if self.editing {
            match key {
                KeyCode::Esc | KeyCode::Enter => self.editing = false,
                KeyCode::Backspace => {
                    self.query.pop();
                }
                KeyCode::Char(c) if !c.is_control() => self.query.push(c),
                _ => return,
            }
            self.cursor = 0;
            self.scroll = 0;
            self.refresh(charts);
            return;
        }
        let last = self.indices.len().saturating_sub(1);
        match key {
            KeyCode::Char('/') => self.editing = true,
            KeyCode::Char('l') => {
                let mut lanes: Vec<_> = charts.iter().map(|chart| chart.lane_count).collect();
                lanes.sort_unstable();
                lanes.dedup();
                self.lanes = lanes
                    .into_iter()
                    .find(|&n| self.lanes.is_none_or(|old| n > old));
                self.cursor = 0;
                self.scroll = 0;
            }
            KeyCode::Char('c') => {
                self.query.clear();
                self.lanes = None;
            }
            KeyCode::Up | KeyCode::Char('k') => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => self.cursor = (self.cursor + 1).min(last),
            KeyCode::Home | KeyCode::Char('g') => self.cursor = 0,
            KeyCode::End | KeyCode::Char('G') => self.cursor = last,
            KeyCode::PageUp => self.cursor = self.cursor.saturating_sub(10),
            KeyCode::PageDown => self.cursor = (self.cursor + 10).min(last),
            _ => return,
        }
        if matches!(key, KeyCode::Char('l' | 'c')) {
            self.refresh(charts);
        }
    }
}

pub fn run(out: &mut Stdout, charts: &[ChartMeta], no_audio: bool) -> Result<Option<ChartMeta>> {
    run_with_state(out, charts, no_audio, &mut SelectorState::default())
}

pub fn run_with_state(
    out: &mut Stdout,
    charts: &[ChartMeta],
    no_audio: bool,
    state: &mut SelectorState,
) -> Result<Option<ChartMeta>> {
    if charts.is_empty() {
        return Ok(None);
    }
    state.editing = false;
    state.refresh(charts);
    let audio = crate::audio::preview_output(!no_audio);
    let mut preview = crate::preview::SelectionPreview::default();
    loop {
        let selected = state.selected();
        preview.update(
            selected,
            selected.and_then(|i| charts[i].preview.as_deref()),
            |path| crate::preview::play(audio.as_ref().map(|(_, handle)| handle), path),
        );
        draw(out, charts, state)?;
        if event::poll(Duration::from_millis(200))? {
            if let Event::Key(k) = event::read()? {
                if k.kind == KeyEventKind::Release {
                    continue;
                }
                if !state.editing {
                    match k.code {
                        KeyCode::Esc | KeyCode::Char('q' | 'Q') => return Ok(None),
                        KeyCode::Enter if selected.is_some() => {
                            return Ok(selected.map(|i| charts[i].clone()))
                        }
                        _ => {}
                    }
                }
                state.key(k.code, charts);
            }
        }
    }
}

fn draw(out: &mut Stdout, charts: &[ChartMeta], state: &mut SelectorState) -> Result<()> {
    draw_frame(out, charts, state, terminal::size()?)
}

fn draw_frame(
    out: &mut impl Write,
    charts: &[ChartMeta],
    state: &mut SelectorState,
    (cols, rows): (u16, u16),
) -> Result<()> {
    let cursor_i = state.cursor;
    let scroll = &mut state.scroll;
    queue!(
        out,
        terminal::BeginSynchronizedUpdate,
        terminal::Clear(terminal::ClearType::All)
    )?;

    let title = "T A P L I N E";
    queue!(
        out,
        cursor::MoveTo(cols.saturating_sub(title.chars().count() as u16) / 2, 1),
        SetForegroundColor(Color::Magenta),
        style::SetAttribute(style::Attribute::Bold),
        Print(title),
        style::SetAttribute(style::Attribute::Reset),
        ResetColor
    )?;

    let subtitle = format!("select a chart  ·  {} loaded", charts.len());
    queue!(
        out,
        cursor::MoveTo(cols.saturating_sub(subtitle.chars().count() as u16) / 2, 2),
        SetForegroundColor(Color::DarkGrey),
        Print(&subtitle),
        ResetColor
    )?;

    let status = format!(
        "{} / {} matches | Lanes: {} | {}Query: {}",
        state.indices.len(),
        charts.len(),
        state
            .lanes
            .map(|n| format!("{n}K"))
            .unwrap_or_else(|| "all".into()),
        if state.editing { "/ editing | " } else { "" },
        crate::terminal_text::sanitize(&state.query),
    );
    let status: String = status
        .chars()
        .take(cols.saturating_sub(4) as usize)
        .collect();
    queue!(out, cursor::MoveTo(2, 3), Print(status))?;
    let list_top = 5u16;
    let detail_rows = if rows >= 16 { 6 } else { 0 };
    let list_bottom = rows.saturating_sub(2 + detail_rows);
    let view_rows = list_bottom.saturating_sub(list_top) as usize;
    if view_rows == 0 {
        queue!(out, terminal::EndSynchronizedUpdate)?;
        out.flush()?;
        return Ok(());
    }

    if cursor_i < *scroll {
        *scroll = cursor_i;
    }
    if cursor_i >= *scroll + view_rows {
        *scroll = cursor_i + 1 - view_rows;
    }

    for row in 0..view_rows {
        let i = *scroll + row;
        if i >= state.indices.len() {
            break;
        }
        let m = &charts[state.indices[i]];
        let selected = i == cursor_i;
        let y = list_top + row as u16;
        let arrow = if selected { "▸ " } else { "  " };
        let text = format_chart_row(m, arrow);
        let clipped: String = text.chars().take(cols as usize).collect();
        let color = if selected { Color::White } else { Color::Grey };
        queue!(out, cursor::MoveTo(2, y), SetForegroundColor(color))?;
        if selected {
            queue!(out, style::SetAttribute(style::Attribute::Bold))?;
        }
        queue!(
            out,
            Print(&clipped),
            style::SetAttribute(style::Attribute::Reset),
            ResetColor
        )?;
    }

    if state.indices.is_empty() {
        queue!(
            out,
            cursor::MoveTo(2, list_top),
            Print("No matching charts. Edit / search or press c to clear.")
        )?;
    } else if detail_rows > 0 {
        draw_details(out, &charts[state.indices[cursor_i]], cols, list_bottom)?;
    }

    let hint = if state.editing {
        "Type to search  Backspace delete  Enter / Esc finish editing"
    } else {
        "↑↓ move  / search  l lanes  c clear  Enter play  Esc quit"
    };
    queue!(
        out,
        cursor::MoveTo(
            cols.saturating_sub(hint.chars().count() as u16) / 2,
            rows.saturating_sub(1)
        ),
        SetForegroundColor(Color::DarkGrey),
        Print(hint),
        ResetColor
    )?;

    queue!(out, terminal::EndSynchronizedUpdate)?;
    out.flush()?;
    Ok(())
}

fn draw_details(out: &mut impl Write, chart: &ChartMeta, cols: u16, top: u16) -> Result<()> {
    let comment: String = crate::terminal_text::sanitize(&chart.comment)
        .chars()
        .take(cols.saturating_sub(4) as usize)
        .collect();
    queue!(out, cursor::MoveTo(2, top), Print(comment))?;
    let dir = chart.path.parent().unwrap_or(Path::new("."));
    crate::artwork::draw(out, dir, &chart.banner, (0, top + 1, cols, 4))?;
    Ok(())
}

fn format_chart_row(m: &ChartMeta, arrow: &str) -> String {
    let badge = format_badge(m);
    let title = if m.title.is_empty() {
        m.path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("(untitled)")
            .to_string()
    } else if m.subtitle.is_empty() {
        m.title.clone()
    } else {
        format!("{}  ~{}~", m.title, m.subtitle)
    };
    let artist = if m.artist.is_empty() {
        "".to_string()
    } else {
        format!("  — {}", m.artist)
    };
    let text = format!("{}{}  {}{}", arrow, badge, title, artist);
    crate::terminal_text::sanitize(&text)
}

fn format_badge(m: &ChartMeta) -> String {
    let lanes = format!("{}K", m.lane_count);
    let dif = match difficulty_label(m.difficulty) {
        "" => "".to_string(),
        s => format!(" {}", s),
    };
    let lv = m.playlevel.map(|v| format!(" Lv{}", v)).unwrap_or_default();
    let bpm = format!(" BPM{:.0}", m.bpm);
    format!("[{}{}{}{}]", lanes, dif, lv, bpm)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn meta(title: &str) -> ChartMeta {
        ChartMeta {
            path: PathBuf::from(format!("/tmp/{}.bms", title)),
            random_seed: 0,
            title: title.to_string(),
            subtitle: String::new(),
            artist: String::new(),
            subartist: String::new(),
            genre: String::new(),
            preview: None,
            banner: String::new(),
            comment: String::new(),
            bpm: 140.0,
            playlevel: Some(3),
            difficulty: Some(2),
            lane_count: 5,
        }
    }

    #[test]
    fn empty_results_render_conditions_and_no_stale_chart_details() {
        let mut chart = meta("Alpha");
        chart.comment = "Selected chart comment".into();
        let charts = vec![chart];
        let mut state = SelectorState {
            query: "missing".into(),
            lanes: Some(7),
            ..Default::default()
        };
        state.refresh(&charts);
        let mut out = Vec::new();
        draw_frame(&mut out, &charts, &mut state, (100, 24)).unwrap();
        let output = String::from_utf8(out).unwrap();
        assert!(output.contains("0 / 1 matches | Lanes: 7K | Query: missing"));
        assert!(output.contains("No matching charts"));
        assert!(!output.contains("Selected chart comment"));
        assert!(!output.contains("Alpha"));
    }

    #[test]
    fn search_matches_title_subtitle_and_artist_case_insensitively() {
        let mut charts = vec![meta("ÉTOILE"), meta("Other"), meta("Third")];
        charts[1].subtitle = "Étoile remix".into();
        charts[2].artist = "étoile".into();
        let mut state = SelectorState {
            query: "éTOILE".into(),
            ..Default::default()
        };
        state.refresh(&charts);
        assert_eq!(state.indices, vec![0, 1, 2]);
        charts[1].lane_count = 7;
        state.lanes = Some(7);
        state.refresh(&charts);
        assert_eq!(state.indices, vec![1]);
        assert_eq!(state.selected(), Some(1));
        assert_eq!(charts[0].title, "ÉTOILE");
    }

    #[test]
    fn navigation_stays_in_bounds_and_empty_results_can_be_cleared() {
        let charts = vec![meta("Alpha"), meta("Beta")];
        let mut state = SelectorState::default();
        state.refresh(&charts);
        state.key(KeyCode::PageDown, &charts);
        assert_eq!(state.selected(), Some(1));
        state.scroll = 1;
        state.key(KeyCode::Char('/'), &charts);
        state.key(KeyCode::Char('q'), &charts);
        assert_eq!(state.query, "q");
        assert_eq!(state.selected(), None);
        assert_eq!(state.scroll, 0);
        state.key(KeyCode::Enter, &charts);
        for key in [
            KeyCode::End,
            KeyCode::Down,
            KeyCode::PageDown,
            KeyCode::Up,
            KeyCode::PageUp,
            KeyCode::Home,
        ] {
            state.key(key, &charts);
            assert_eq!(state.cursor, 0);
            assert_eq!(state.selected(), None);
        }
        state.key(KeyCode::Char('c'), &charts);
        assert_eq!(state.selected(), Some(0));
    }

    #[test]
    fn editing_and_lane_cycle_combine_and_retain_conditions_on_refresh() {
        let mut charts = vec![meta("Alpha"), meta("Beta")];
        charts[1].lane_count = 7;
        let mut state = SelectorState::default();
        state.refresh(&charts);
        state.key(KeyCode::Char('l'), &charts);
        assert_eq!(state.lanes, Some(5));
        state.key(KeyCode::Char('/'), &charts);
        state.key(KeyCode::Char('B'), &charts);
        state.key(KeyCode::Esc, &charts);
        assert!(state.indices.is_empty());
        state.key(KeyCode::Char('l'), &charts);
        assert_eq!(state.selected(), Some(1));
        state.refresh(&charts);
        assert_eq!(state.query, "B");
        assert_eq!(state.lanes, Some(7));
        state.key(KeyCode::Char('l'), &charts);
        assert_eq!(state.lanes, None);
        state.key(KeyCode::Char('/'), &charts);
        state.key(KeyCode::Backspace, &charts);
        assert_eq!(state.indices, vec![0, 1]);
    }

    #[test]
    fn selected_details_render_banner_and_sanitized_comment() {
        crossterm::style::force_color_output(true);
        let dir = tempdir();
        image::RgbImage::from_pixel(4, 2, image::Rgb([255, 0, 0]))
            .save(dir.join("banner.png"))
            .unwrap();
        let mut chart = meta("Song");
        chart.path = dir.join("song.bms");
        chart.banner = "banner.png".into();
        chart.comment = "Great song!\x1b\x07".into();
        let mut out = Vec::new();
        draw_details(&mut out, &chart, 40, 10).unwrap();
        let output = String::from_utf8(out).unwrap();
        assert!(output.contains("Great song!"));
        assert!(!output.contains('\x07'));
        assert!(output.contains('▀'));
        assert!(output.contains("38;2;255;0;0"));
    }

    #[test]
    fn chart_row_sanitizes_metadata_before_clipping() {
        let mut chart = meta("曲名\x1b]52;c;VEVTVA==\x07");
        chart.subtitle = "副題\u{9b}2J".into();
        chart.artist = "作者\r\n\t\x7f".into();
        let row = format_chart_row(&chart, "▸ ");
        assert!(row.ends_with("曲名]52;c;VEVTVA==  ~副題2J~  — 作者"));
        assert!(!row.chars().any(char::is_control));
    }

    #[test]
    fn chart_row_sanitizes_filename_fallback() {
        let mut chart = meta("");
        chart.path = PathBuf::from("曲名\x1b]52;c;VEVTVA==\x07.bms");
        let row = format_chart_row(&chart, "  ");
        assert!(row.ends_with("曲名]52;c;VEVTVA==.bms"));
        assert!(!row.chars().any(char::is_control));
    }

    #[test]
    fn scan_returns_subtitle_alongside_title_when_present() {
        let dir = tempdir();
        std::fs::write(
            dir.join("s.bms"),
            "#TITLE Song\n#SUBTITLE ~ext~\n#BPM 130\n#00111:0100\n",
        )
        .unwrap();
        let charts = scan(&dir);
        assert_eq!(charts.len(), 1);
        assert_eq!(charts[0].subtitle, "~ext~");
    }

    #[test]
    fn is_bms_recognises_the_four_supported_extensions() {
        assert!(is_bms(Path::new("song.bms")));
        assert!(is_bms(Path::new("song.bml")));
        assert!(is_bms(Path::new("song.bme")));
        assert!(is_bms(Path::new("song.pms")));
    }

    #[test]
    fn is_bms_is_case_insensitive() {
        assert!(is_bms(Path::new("song.BMS")));
        assert!(is_bms(Path::new("song.Pms")));
    }

    #[test]
    fn is_bms_rejects_unrelated_files() {
        assert!(!is_bms(Path::new("song.wav")));
        assert!(!is_bms(Path::new("song.mp3")));
        assert!(!is_bms(Path::new("song.txt")));
        assert!(!is_bms(Path::new("README")));
        assert!(!is_bms(Path::new(".hidden")));
    }

    #[test]
    fn format_badge_includes_lanes_difficulty_level_and_bpm() {
        let m = meta("A");
        assert_eq!(format_badge(&m), "[5K NORMAL Lv3 BPM140]");
    }

    #[test]
    fn format_badge_omits_difficulty_label_when_missing() {
        let mut m = meta("A");
        m.difficulty = None;
        assert_eq!(format_badge(&m), "[5K Lv3 BPM140]");
    }

    #[test]
    fn format_badge_omits_level_when_missing() {
        let mut m = meta("A");
        m.playlevel = None;
        assert_eq!(format_badge(&m), "[5K NORMAL BPM140]");
    }

    #[test]
    fn scan_finds_charts_recursively_and_sorts_by_title() {
        let dir = tempdir();
        std::fs::write(
            dir.join("z.bms"),
            "#TITLE Zebra\n#ARTIST who\n#BPM 130\n#00111:0100\n",
        )
        .unwrap();
        let sub = dir.join("nested");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(
            sub.join("a.bms"),
            "#TITLE Alpha\n#ARTIST who\n#BPM 130\n#00111:0100\n",
        )
        .unwrap();
        std::fs::write(dir.join("notes.txt"), "ignore me").unwrap();

        let charts = scan(&dir);
        let titles: Vec<_> = charts.iter().map(|c| c.title.as_str()).collect();
        assert_eq!(titles, vec!["Alpha", "Zebra"]);
    }

    #[test]
    fn scan_returns_empty_for_missing_directory() {
        let charts = scan(Path::new("/nonexistent-tapline-test-dir-xyz"));
        assert!(charts.is_empty());
    }

    #[test]
    fn scan_stops_recursing_past_five_levels() {
        let dir = tempdir();
        // Build a chain deeper than the walker's cap.
        let mut cursor = dir.clone();
        for i in 0..8 {
            cursor = cursor.join(format!("d{}", i));
            std::fs::create_dir(&cursor).unwrap();
        }
        // Charts at various depths.
        std::fs::write(dir.join("root.bms"), "#TITLE Root\n#BPM 130\n#00111:0100\n").unwrap();
        std::fs::write(
            dir.join("d0/d1/d2/d3/d4/d5/deep.bms"),
            "#TITLE Deep\n#BPM 130\n#00111:0100\n",
        )
        .unwrap();
        let charts = scan(&dir);
        let titles: Vec<_> = charts.iter().map(|c| c.title.as_str()).collect();
        assert!(titles.contains(&"Root"));
        assert!(
            !titles.contains(&"Deep"),
            "walker should not have descended six levels: {:?}",
            titles
        );
    }

    #[test]
    fn scan_skips_files_that_look_bms_but_are_unreadable_content() {
        let dir = tempdir();
        // A well-formed bms.
        std::fs::write(dir.join("ok.bms"), "#TITLE Ok\n#BPM 130\n#00111:0100\n").unwrap();
        // A binary blob renamed to .bms.
        std::fs::write(dir.join("bad.bms"), b"\x00\x01\x02\x03\x04\x05\xff\xfe").unwrap();
        let charts = scan(&dir);
        // Both should show up; the binary one will just have a blank title.
        assert_eq!(charts.len(), 2);
    }

    #[test]
    fn format_badge_omits_both_when_playlevel_and_difficulty_missing() {
        let mut m = meta("A");
        m.difficulty = None;
        m.playlevel = None;
        assert_eq!(format_badge(&m), "[5K BPM140]");
    }

    #[test]
    fn format_badge_rounds_bpm_to_the_nearest_integer() {
        let mut m = meta("A");
        m.bpm = 173.6;
        assert_eq!(format_badge(&m), "[5K NORMAL Lv3 BPM174]");
    }

    fn tempdir() -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let dir = std::env::temp_dir().join(format!("tapline-select-test-{}-{}", pid, n));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}
