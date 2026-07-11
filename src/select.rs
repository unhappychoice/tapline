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
use std::path::{Path, PathBuf};
use std::time::Duration;

const EXTS: [&str; 4] = ["bms", "bml", "bme", "pms"];

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
            if let Ok(meta) = bms::read_meta(&path) {
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

pub fn run(out: &mut Stdout, charts: &[ChartMeta]) -> Result<Option<PathBuf>> {
    if charts.is_empty() {
        return Ok(None);
    }
    let mut cursor_i: usize = 0;
    let mut scroll: usize = 0;
    loop {
        draw(out, charts, cursor_i, &mut scroll)?;
        if event::poll(Duration::from_millis(200))? {
            if let Event::Key(k) = event::read()? {
                if k.kind == KeyEventKind::Release {
                    continue;
                }
                match k.code {
                    KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') => return Ok(None),
                    KeyCode::Up | KeyCode::Char('k') => cursor_i = cursor_i.saturating_sub(1),
                    KeyCode::Down | KeyCode::Char('j') if cursor_i + 1 < charts.len() => {
                        cursor_i += 1;
                    }
                    KeyCode::Home | KeyCode::Char('g') => cursor_i = 0,
                    KeyCode::End | KeyCode::Char('G') => cursor_i = charts.len() - 1,
                    KeyCode::PageUp => cursor_i = cursor_i.saturating_sub(10),
                    KeyCode::PageDown => cursor_i = (cursor_i + 10).min(charts.len() - 1),
                    KeyCode::Enter => return Ok(Some(charts[cursor_i].path.clone())),
                    _ => {}
                }
            }
        }
    }
}

fn draw(out: &mut Stdout, charts: &[ChartMeta], cursor_i: usize, scroll: &mut usize) -> Result<()> {
    let (cols, rows) = terminal::size()?;
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

    let list_top = 4u16;
    let list_bottom = rows.saturating_sub(2);
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
        if i >= charts.len() {
            break;
        }
        let m = &charts[i];
        let selected = i == cursor_i;
        let y = list_top + row as u16;
        let arrow = if selected { "▸ " } else { "  " };
        let badge = format_badge(m);
        let title = if m.title.is_empty() {
            m.path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("(untitled)")
                .to_string()
        } else {
            m.title.clone()
        };
        let artist = if m.artist.is_empty() {
            "".to_string()
        } else {
            format!("  — {}", m.artist)
        };
        let text = format!("{}{}  {}{}", arrow, badge, title, artist);
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

    let hint = "↑↓ / j k  move    Enter  play    Esc  quit";
    queue!(
        out,
        cursor::MoveTo(
            cols.saturating_sub(hint.chars().count() as u16) / 2,
            rows - 1
        ),
        SetForegroundColor(Color::DarkGrey),
        Print(hint),
        ResetColor
    )?;

    queue!(out, terminal::EndSynchronizedUpdate)?;
    out.flush()?;
    Ok(())
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

    fn meta(title: &str) -> ChartMeta {
        ChartMeta {
            path: PathBuf::from(format!("/tmp/{}.bms", title)),
            title: title.to_string(),
            subtitle: String::new(),
            artist: String::new(),
            subartist: String::new(),
            genre: String::new(),
            bpm: 140.0,
            playlevel: Some(3),
            difficulty: Some(2),
            lane_count: 5,
        }
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
