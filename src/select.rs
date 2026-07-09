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
