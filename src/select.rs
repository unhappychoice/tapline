mod discovery;
use crate::artwork;
use crate::bms::ChartMeta;
use crate::chart::difficulty_label;
use crate::results::Identity;
use anyhow::Result;
use caseless::default_case_fold_str;
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind},
    queue,
    style::{self, Color, Print, ResetColor, SetForegroundColor},
    terminal,
};
use image::RgbaImage;
use std::io::{Stdout, Write};
use std::path::Path;
use std::rc::Rc;
use std::time::Duration;
use unicode_width::UnicodeWidthChar;

const ROW_LEFT: u16 = 2;
const MIN_DETAIL_ROWS: u16 = 16;
const BANNER_HEIGHT: u16 = 4;

#[derive(Clone, Debug)]
pub struct ChartEntry {
    meta: ChartMeta,
    pub(crate) identity: Identity,
}

impl std::ops::Deref for ChartEntry {
    type Target = ChartMeta;

    fn deref(&self) -> &ChartMeta {
        &self.meta
    }
}

pub fn scan(root: &Path) -> Vec<ChartEntry> {
    SelectorState::default().scan(root)
}

#[cfg(test)]
pub(crate) use discovery::{count_content_read, count_metadata_parse};

#[derive(Default)]
pub struct SelectorState {
    discovery: discovery::Discovery,
    pub(crate) reopening_selector: bool,
    query: String,
    lanes: Option<usize>,
    /// `None` shows every clear status; `Some(true)`/`Some(false)` restrict
    /// to cleared / not-yet-cleared (including unplayed) charts.
    clear_filter: Option<bool>,
    /// Per-chart clear status aligned by index with the `charts` slice, as
    /// loaded from the records store. Empty when records are unavailable,
    /// in which case the clear filter has no effect.
    clears: Vec<Option<bool>>,
    editing: bool,
    cursor: usize,
    scroll: usize,
    indices: Vec<usize>,
    /// Case-folded title/subtitle/artist for each chart, keyed by index into
    /// the `charts` slice passed to `refresh`. Invalidated on each scan and
    /// reused across search keystrokes within the same collection.
    case_folded_cache: Vec<(String, String, String)>,
}

#[derive(Debug, PartialEq, Eq)]
enum SelectorAction {
    None,
    Redraw,
    Play(usize),
    Quit,
}

impl SelectorState {
    pub fn scan(&mut self, root: &Path) -> Vec<ChartEntry> {
        self.case_folded_cache.clear();
        self.discovery.scan(root, &crate::bms::random_seed)
    }

    pub fn refresh_changed_chart(&mut self, path: &Path) {
        self.discovery.invalidate(path);
        self.reopen();
    }

    pub fn reopen(&mut self) {
        self.reopening_selector = true;
    }

    fn set_clears(&mut self, clears: Vec<Option<bool>>) {
        self.clears = clears;
    }

    fn refresh(&mut self, charts: &[ChartEntry]) {
        if self.case_folded_cache.len() != charts.len() {
            self.case_folded_cache = charts
                .iter()
                .map(|chart| {
                    (
                        default_case_fold_str(&chart.title),
                        default_case_fold_str(&chart.subtitle),
                        default_case_fold_str(&chart.artist),
                    )
                })
                .collect();
        }
        let query = default_case_fold_str(&self.query);
        self.indices = charts
            .iter()
            .enumerate()
            .filter(|(i, chart)| {
                self.lanes.is_none_or(|lanes| chart.lane_count == lanes)
                    && self.clear_filter.is_none_or(|wanted| {
                        self.clears.len() != charts.len()
                            || wanted == (self.clears[*i] == Some(true))
                    })
                    && {
                        let (title, subtitle, artist) = &self.case_folded_cache[*i];
                        [title, subtitle, artist]
                            .iter()
                            .any(|text| text.contains(&query))
                    }
            })
            .map(|(i, _)| i)
            .collect();
        self.cursor = self.cursor.min(self.indices.len().saturating_sub(1));
        self.scroll = self.scroll.min(self.cursor);
    }

    fn selected(&self) -> Option<usize> {
        self.indices.get(self.cursor).copied()
    }

    fn handle_key(&mut self, key: KeyEvent, charts: &[ChartEntry]) -> SelectorAction {
        match (key.kind, key.code, self.editing) {
            (KeyEventKind::Release, _, _) => return SelectorAction::None,
            (KeyEventKind::Repeat, KeyCode::Enter | KeyCode::Esc, true)
            | (KeyEventKind::Repeat, KeyCode::Char('/' | 'l' | 'c' | 'v'), false) => {
                return SelectorAction::None;
            }
            (KeyEventKind::Press, KeyCode::Esc | KeyCode::Char('q' | 'Q'), false) => {
                return SelectorAction::Quit;
            }
            (KeyEventKind::Press, KeyCode::Enter, false) => {
                return self
                    .selected()
                    .map_or(SelectorAction::None, SelectorAction::Play);
            }
            _ => {}
        }
        if self.key(key.code, charts) {
            SelectorAction::Redraw
        } else {
            SelectorAction::None
        }
    }

    fn key(&mut self, key: KeyCode, charts: &[ChartEntry]) -> bool {
        let before = self.visible_state();
        self.apply_key(key, charts);
        before != self.visible_state()
    }

    fn visible_state(&self) -> (usize, usize, Option<usize>, Option<bool>, bool, usize) {
        // Key edits only append, remove, or clear query characters.
        (
            self.cursor,
            self.scroll,
            self.lanes,
            self.clear_filter,
            self.editing,
            self.query.len(),
        )
    }

    fn apply_key(&mut self, key: KeyCode, charts: &[ChartEntry]) {
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
            KeyCode::Char('v') => {
                self.clear_filter = match self.clear_filter {
                    None => Some(true),
                    Some(true) => Some(false),
                    Some(false) => None,
                };
                self.cursor = 0;
                self.scroll = 0;
            }
            KeyCode::Char('c') => {
                self.query.clear();
                self.lanes = None;
                self.clear_filter = None;
            }
            KeyCode::Up | KeyCode::Char('k') => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => self.cursor = (self.cursor + 1).min(last),
            KeyCode::Home | KeyCode::Char('g') => self.cursor = 0,
            KeyCode::End | KeyCode::Char('G') => self.cursor = last,
            KeyCode::PageUp => self.cursor = self.cursor.saturating_sub(10),
            KeyCode::PageDown => self.cursor = (self.cursor + 10).min(last),
            _ => return,
        }
        if matches!(key, KeyCode::Char('l' | 'c' | 'v')) {
            self.refresh(charts);
        }
    }
}

pub fn run(out: &mut Stdout, charts: &[ChartEntry], no_audio: bool) -> Result<Option<ChartEntry>> {
    run_with_state(out, charts, no_audio, &mut SelectorState::default())
}

pub fn run_with_state(
    out: &mut Stdout,
    charts: &[ChartEntry],
    no_audio: bool,
    state: &mut SelectorState,
) -> Result<Option<ChartEntry>> {
    let reopening_selector = std::mem::take(&mut state.reopening_selector);
    if charts.is_empty() && !reopening_selector {
        return Ok(None);
    }
    let records_path = crate::results::data_path();
    let store = records_path.and_then(|path| crate::results::Store::load(&path).ok());
    let progress = load_progress(charts, store.as_ref());
    state.set_clears(load_clears(charts, store.as_ref()));
    state.editing = false;
    state.refresh(charts);
    let audio = crate::audio::preview_output(!no_audio);
    let mut preview = audio
        .as_ref()
        .and_then(|_| crate::preview::SelectionPreview::audio());
    run_loop(
        out,
        charts,
        state,
        &progress,
        terminal::size,
        || {
            if event::poll(Duration::from_millis(200))? {
                return event::read().map(Some);
            }
            Ok(None)
        },
        |selected, path| {
            if let Some((preview, audio)) = preview.as_mut().zip(audio.as_ref()) {
                preview.update(selected, path, |source| crate::preview::play(audio, source));
            }
        },
    )
}

fn run_loop(
    out: &mut impl Write,
    charts: &[ChartEntry],
    state: &mut SelectorState,
    progress: &[String],
    mut size: impl FnMut() -> std::io::Result<(u16, u16)>,
    mut input: impl FnMut() -> std::io::Result<Option<Event>>,
    mut preview: impl FnMut(Option<usize>, Option<&Path>),
) -> Result<Option<ChartEntry>> {
    let mut frame = SelectorFrame::default();
    loop {
        let selected = state.selected();
        preview(
            selected,
            selected.and_then(|i| charts[i].preview.as_deref()),
        );
        frame.draw_if_changed(out, charts, state, progress, size()?)?;
        let Some(input) = input()? else {
            continue;
        };
        match input {
            Event::Resize(..) => frame.invalidate(),
            Event::Key(key) => match state.handle_key(key, charts) {
                SelectorAction::None => {}
                SelectorAction::Redraw => frame.invalidate(),
                SelectorAction::Play(i) => return Ok(Some(charts[i].clone())),
                SelectorAction::Quit => return Ok(None),
            },
            _ => {}
        }
    }
}

fn load_progress(charts: &[ChartEntry], store: Option<&crate::results::Store>) -> Vec<String> {
    let Some(store) = store else {
        return vec!["[RECORDS UNAVAILABLE]".into(); charts.len()];
    };
    charts
        .iter()
        .map(|chart| format_progress(store.get(&chart.identity)))
        .collect()
}

fn load_clears(charts: &[ChartEntry], store: Option<&crate::results::Store>) -> Vec<Option<bool>> {
    let Some(store) = store else {
        return Vec::new();
    };
    charts
        .iter()
        .map(|chart| store.get(&chart.identity).map(|record| record.best.clear))
        .collect()
}

#[derive(Default)]
struct SelectorFrame {
    size: Option<(u16, u16)>,
    banner: Option<Rc<RgbaImage>>,
}

impl SelectorFrame {
    fn invalidate(&mut self) {
        self.size = None;
    }

    fn draw_if_changed(
        &mut self,
        out: &mut impl Write,
        charts: &[ChartEntry],
        state: &mut SelectorState,
        progress: &[String],
        size: (u16, u16),
    ) -> Result<()> {
        let banner = state
            .selected()
            .filter(|_| size.1 >= MIN_DETAIL_ROWS)
            .and_then(|i| chart_banner(&charts[i], size.0));
        let same_banner = match (&self.banner, &banner) {
            (Some(previous), Some(current)) => Rc::ptr_eq(previous, current),
            (None, None) => true,
            _ => false,
        };
        if self.size != Some(size) || !same_banner {
            draw_frame(out, charts, state, progress, size, banner.as_deref())?;
            self.size = Some(size);
            self.banner = banner;
        }
        Ok(())
    }
}

fn chart_banner(chart: &ChartMeta, width: u16) -> Option<Rc<RgbaImage>> {
    let dir = chart.path.parent().unwrap_or(Path::new("."));
    artwork::thumbnail(dir, &chart.banner, (width, BANNER_HEIGHT))
}

fn draw_frame(
    out: &mut impl Write,
    charts: &[ChartEntry],
    state: &mut SelectorState,
    progress: &[String],
    (cols, rows): (u16, u16),
    banner: Option<&RgbaImage>,
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
        "{} / {} matches | Lanes: {} | Clear: {} | {}Query: {}",
        state.indices.len(),
        charts.len(),
        state
            .lanes
            .map(|n| format!("{n}K"))
            .unwrap_or_else(|| "all".into()),
        match state.clear_filter {
            None => "all",
            Some(true) => "cleared",
            Some(false) => "uncleared",
        },
        if state.editing { "/ editing | " } else { "" },
        crate::terminal_text::sanitize(&state.query),
    );
    let row_width = cols.saturating_sub(ROW_LEFT) as usize;
    queue!(
        out,
        cursor::MoveTo(ROW_LEFT, 3),
        Print(clip_to_width(&status, row_width))
    )?;
    let list_top = 5u16;
    let detail_rows = if rows >= MIN_DETAIL_ROWS {
        BANNER_HEIGHT + 2
    } else {
        0
    };
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
        let text = format!(
            "{} {}",
            progress[state.indices[i]],
            format_chart_row(m, arrow)
        );
        let clipped = clip_to_width(&text, row_width);
        let color = if selected { Color::White } else { Color::Grey };
        queue!(out, cursor::MoveTo(ROW_LEFT, y), SetForegroundColor(color))?;
        if selected {
            queue!(out, style::SetAttribute(style::Attribute::Bold))?;
        }
        queue!(
            out,
            Print(clipped),
            style::SetAttribute(style::Attribute::Reset),
            ResetColor
        )?;
    }

    if state.indices.is_empty() {
        queue!(
            out,
            cursor::MoveTo(2, list_top),
            Print("No matching charts. Edit / search or press c to clear filters.")
        )?;
    } else if detail_rows > 0 {
        draw_details(
            out,
            &charts[state.indices[cursor_i]],
            cols,
            list_bottom,
            banner,
        )?;
    }

    let hint = if state.editing {
        "Type to search  Backspace delete  Enter / Esc finish editing"
    } else {
        "↑↓ move  / search  l lanes  v clear status  c clear  Enter play  Esc quit"
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

fn clip_to_width(text: &str, mut remaining: usize) -> &str {
    for (index, ch) in text.char_indices() {
        let width = ch.width().unwrap_or(0);
        if width > remaining {
            return &text[..index];
        }
        remaining -= width;
    }
    text
}

fn draw_details(
    out: &mut impl Write,
    chart: &ChartMeta,
    cols: u16,
    top: u16,
    banner: Option<&RgbaImage>,
) -> Result<()> {
    let comment = crate::terminal_text::sanitize(&chart.comment);
    let clipped = clip_to_width(&comment, cols.saturating_sub(ROW_LEFT) as usize);
    queue!(out, cursor::MoveTo(ROW_LEFT, top), Print(clipped))?;
    if let Some(banner) = banner {
        artwork::draw_pixels(out, banner, (0, top + 1, cols, BANNER_HEIGHT))?;
    }
    Ok(())
}

fn format_progress(record: Option<&crate::results::Record>) -> String {
    match record {
        Some(record) => format!(
            "[BEST {:.1}% {}]",
            record.best.accuracy,
            if record.best.clear { "CLEAR" } else { "FAILED" }
        ),
        None => "[UNPLAYED]".into(),
    }
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
    use super::discovery::is_bms;
    use super::*;
    use std::path::PathBuf;

    fn meta(title: &str) -> ChartEntry {
        let meta = ChartMeta {
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
        };
        let identity = Identity::from_bytes(&meta.path, title.as_bytes(), Some(meta.random_seed));
        ChartEntry { meta, identity }
    }

    fn key_event(code: KeyCode, kind: KeyEventKind) -> KeyEvent {
        KeyEvent::new_with_kind(code, event::KeyModifiers::NONE, kind)
    }

    #[test]
    fn blocked_preview_allows_redraw_navigation_empty_results_and_quit() {
        use std::sync::mpsc;
        const TIMEOUT: Duration = Duration::from_secs(5);

        struct Loaded(mpsc::Sender<()>);
        impl Drop for Loaded {
            fn drop(&mut self) {
                let _ = self.0.send(());
            }
        }

        let (started, loads) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        let (discard, discarded) = mpsc::channel();
        let mut preview = crate::preview::SelectionPreview::new(move |path| {
            started.send(path.to_path_buf()).unwrap();
            wait.recv_timeout(TIMEOUT).ok()?;
            Some(Loaded(discard.clone()))
        })
        .unwrap();
        let mut charts = [meta("Alpha"), meta("Beta")];
        charts[0].meta.preview = Some("alpha.wav".into());
        charts[1].meta.preview = Some("beta.wav".into());
        let mut state = SelectorState::default();
        state.refresh(&charts);
        let mut out = Vec::new();
        let mut keys = [
            KeyCode::Down,
            KeyCode::Char('/'),
            KeyCode::Char('z'),
            KeyCode::Esc,
            KeyCode::Char('c'),
            KeyCode::Down,
            KeyCode::Char('q'),
        ]
        .into_iter();
        let result = run_loop(
            &mut out,
            &charts,
            &mut state,
            &["[UNPLAYED]".into(), "[UNPLAYED]".into()],
            || Ok((100, 12)),
            || {
                let code = keys.next().unwrap();
                match code {
                    KeyCode::Down => {
                        assert_eq!(loads.recv_timeout(TIMEOUT).unwrap(), Path::new("alpha.wav"));
                    }
                    KeyCode::Esc => {
                        release.send(()).unwrap();
                        discarded.recv_timeout(TIMEOUT).unwrap();
                    }
                    _ => {}
                }
                Ok(Some(Event::Key(key_event(code, KeyEventKind::Press))))
            },
            move |selected, path| {
                preview.update(selected, path, |_| panic!("stale preview played"));
            },
        )
        .unwrap();
        assert!(result.is_none());
        let output = String::from_utf8(out).unwrap();
        assert!(output.contains("▸ [5K NORMAL Lv3 BPM140]  Alpha"));
        assert!(output.contains("▸ [5K NORMAL Lv3 BPM140]  Beta"));
        assert!(output.contains("No matching charts"));
        release.send(()).unwrap();
        discarded.recv_timeout(TIMEOUT).unwrap();
        assert_eq!(
            loads.recv_timeout(TIMEOUT),
            Err(mpsc::RecvTimeoutError::Disconnected)
        );
    }

    #[test]
    fn finishing_search_requires_a_fresh_enter_to_play() {
        let charts = [meta("Alpha")];
        let mut state = SelectorState {
            query: "Al".into(),
            editing: true,
            ..Default::default()
        };
        state.refresh(&charts);
        let press = key_event(KeyCode::Enter, KeyEventKind::Press);
        let repeat = key_event(KeyCode::Enter, KeyEventKind::Repeat);

        assert_eq!(state.handle_key(repeat, &charts), SelectorAction::None);
        assert!(state.editing);
        assert_eq!(state.handle_key(press, &charts), SelectorAction::Redraw);
        assert!(!state.editing);
        assert_eq!(state.handle_key(repeat, &charts), SelectorAction::None);
        assert_eq!(state.handle_key(repeat, &charts), SelectorAction::None);
        assert_eq!(state.handle_key(press, &charts), SelectorAction::Play(0));
    }

    #[test]
    fn selector_ignores_enter_repeat_on_return_from_results_and_accepts_navigation_repeats() {
        let charts = [meta("Alpha"), meta("Beta"), meta("Gamma")];
        let mut state = SelectorState::default();
        state.refresh(&charts);
        let mut handle = |code, kind| state.handle_key(key_event(code, kind), &charts);
        use KeyEventKind::{Press, Repeat};

        assert_eq!(handle(KeyCode::Enter, Repeat), SelectorAction::None);
        assert_eq!(handle(KeyCode::Down, Press), SelectorAction::Redraw);
        assert_eq!(handle(KeyCode::Down, Repeat), SelectorAction::Redraw);
        assert_eq!(handle(KeyCode::Enter, Press), SelectorAction::Play(2));
    }

    #[test]
    fn selector_repeats_edit_text_without_triggering_transitions_or_filters() {
        let charts = [meta("Alpha")];
        let mut state = SelectorState::default();
        state.refresh(&charts);
        let mut handle = |code, kind| state.handle_key(key_event(code, kind), &charts);
        use KeyEventKind::{Press, Release, Repeat};

        for code in [
            KeyCode::Char('/'),
            KeyCode::Char('l'),
            KeyCode::Esc,
            KeyCode::Char('q'),
            KeyCode::Char('Q'),
        ] {
            assert_eq!(handle(code, Repeat), SelectorAction::None);
        }
        assert_eq!(handle(KeyCode::Char('/'), Press), SelectorAction::Redraw);
        assert_eq!(handle(KeyCode::Char('/'), Repeat), SelectorAction::Redraw);
        assert_eq!(handle(KeyCode::Char('c'), Repeat), SelectorAction::Redraw);
        assert_eq!(handle(KeyCode::Backspace, Repeat), SelectorAction::Redraw);
        assert_eq!(handle(KeyCode::Backspace, Release), SelectorAction::None);
        assert_eq!(handle(KeyCode::Esc, Repeat), SelectorAction::None);
        assert_eq!(handle(KeyCode::Esc, Press), SelectorAction::Redraw);
        assert_eq!(handle(KeyCode::Char('c'), Repeat), SelectorAction::None);
        assert_eq!(handle(KeyCode::Esc, Press), SelectorAction::Quit);
        assert_eq!(state.query, "/");
    }

    #[test]
    fn status_and_comment_clip_wide_text_at_the_terminal_edge() {
        let status_prefix = "1 / 1 matches | Lanes: all | Clear: all | Query: ";
        let query_cells = 78 - status_prefix.len();
        let fitting_query = format!(
            "{}{}",
            "a".repeat(query_cells % 2),
            "界".repeat(query_cells / 2)
        );
        let fitting_comment = "語".repeat(39);
        for extra_chars in [0, 21] {
            let query = format!("{fitting_query}{}", "界".repeat(extra_chars));
            let mut chart = meta(&query);
            chart.meta.comment = format!("{fitting_comment}{}", "語".repeat(extra_chars));
            let charts = [chart];
            let mut state = SelectorState {
                query,
                ..SelectorState::default()
            };
            state.refresh(&charts);
            let mut out = Vec::new();
            draw_frame(
                &mut out,
                &charts,
                &mut state,
                &["[UNPLAYED]".into()],
                (80, 16),
                None,
            )
            .unwrap();

            let frame = String::from_utf8(out).unwrap();
            let status = format!("{status_prefix}{fitting_query}");
            assert!(frame.contains(&status));
            assert!(!frame.contains(&format!("{status}界")));
            assert!(frame.contains(&fitting_comment));
            assert!(!frame.contains(&format!("{fitting_comment}語")));
        }
    }

    #[test]
    fn idle_frames_emit_no_output_and_input_or_resize_redraws() {
        let mut charts = [meta("Alpha"), meta("Beta")];
        charts[1].meta.lane_count = 7;
        let progress = ["[UNPLAYED]".into(), "[UNPLAYED]".into()];
        let mut state = SelectorState::default();
        state.refresh(&charts);
        let mut frame = SelectorFrame::default();
        let mut render = |state: &mut SelectorState, key: Option<KeyCode>, size| {
            if key.is_some_and(|key| state.key(key, &charts)) {
                frame.invalidate();
            }
            let mut out = Vec::new();
            frame
                .draw_if_changed(&mut out, &charts, state, &progress, size)
                .unwrap();
            String::from_utf8(out).unwrap()
        };

        let size = (100, 24);
        let initial = render(&mut state, None, size);
        #[cfg(not(windows))]
        assert!(initial.contains("\x1b[2J"));
        assert!(initial.contains("Alpha"));
        for _ in 0..3 {
            assert!(render(&mut state, None, size).is_empty());
        }
        let moved = render(&mut state, Some(KeyCode::Down), size);
        assert!(moved.contains("▸ [7K NORMAL Lv3 BPM140]  Beta"));
        assert!(render(&mut state, Some(KeyCode::Down), size).is_empty());
        assert!(render(&mut state, Some(KeyCode::Char('/')), size).contains("/ editing"));
        let filtered = render(&mut state, Some(KeyCode::Char('A')), size);
        assert!(filtered.contains("Query: A"));
        assert!(render(&mut state, Some(KeyCode::Char('l')), size).contains("1 / 2 matches"));
        assert!(!render(&mut state, Some(KeyCode::Enter), size).contains("/ editing"));
        assert!(render(&mut state, Some(KeyCode::Char('l')), size).contains("Lanes: 5K"));
        assert!(render(&mut state, None, (80, 12)).contains("Alpha"));
        assert!(render(&mut state, None, (80, 12)).is_empty());
    }

    #[test]
    fn banner_changes_redraw_without_input() {
        let dir = tempdir();
        let banner = dir.join("banner.png");
        let mut chart = meta("Song");
        chart.meta.path = dir.join("song.bms");
        chart.meta.banner = "banner.png".into();
        let charts = [chart];
        let mut state = SelectorState::default();
        state.refresh(&charts);
        let mut frame = SelectorFrame::default();
        let mut render = || {
            let mut out = Vec::new();
            frame
                .draw_if_changed(
                    &mut out,
                    &charts,
                    &mut state,
                    &["[UNPLAYED]".into()],
                    (80, 24),
                )
                .unwrap();
            String::from_utf8(out).unwrap()
        };

        let save_banner = |width, color| {
            image::RgbImage::from_pixel(width, 2, image::Rgb(color))
                .save(&banner)
                .unwrap();
        };
        save_banner(4, [255, 0, 0]);
        assert!(render().contains('▀'));
        assert!(render().is_empty());

        // Change the encoded length so detection does not depend on timestamp resolution.
        let original_len = std::fs::metadata(&banner).unwrap().len();
        save_banner(32, [0, 255, 0]);
        assert_ne!(std::fs::metadata(&banner).unwrap().len(), original_len);
        assert!(render().contains('▀'));
        assert!(render().is_empty());

        std::fs::remove_file(&banner).unwrap();
        let removed = render();
        #[cfg(not(windows))]
        assert!(removed.contains("\x1b[2J"));
        assert!(removed.contains("Song"));
        assert!(!removed.contains('▀'));
        assert!(render().is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn empty_results_render_conditions_and_no_stale_chart_details() {
        let mut chart = meta("Alpha");
        chart.meta.comment = "Selected chart comment".into();
        let charts = vec![chart];
        let mut state = SelectorState {
            query: "missing".into(),
            lanes: Some(7),
            ..Default::default()
        };
        state.refresh(&charts);
        let mut out = Vec::new();
        SelectorFrame::default()
            .draw_if_changed(
                &mut out,
                &charts,
                &mut state,
                &["[UNPLAYED]".into()],
                (100, 24),
            )
            .unwrap();
        let output = String::from_utf8(out).unwrap();
        assert!(output.contains("0 / 1 matches | Lanes: 7K | Clear: all | Query: missing"));
        assert!(output.contains("No matching charts"));
        assert!(!output.contains("Selected chart comment"));
        assert!(!output.contains("Alpha"));
    }

    #[test]
    fn filtered_and_scrolled_rows_keep_their_chart_progress() {
        let charts: Vec<_> = (0..8).map(|i| meta(&format!("Song {i}"))).collect();
        let progress: Vec<_> = (0..8).map(|i| format!("[BEST {i}.0% CLEAR]")).collect();
        let mut state = SelectorState {
            query: "Song 7".into(),
            ..Default::default()
        };
        state.refresh(&charts);
        let mut out = Vec::new();
        SelectorFrame::default()
            .draw_if_changed(&mut out, &charts, &mut state, &progress, (100, 12))
            .unwrap();
        let output = String::from_utf8(out).unwrap();
        assert!(output.contains("[BEST 7.0% CLEAR] ▸ [5K NORMAL Lv3 BPM140]  Song 7"));
        assert!(!output.contains("[BEST 0.0% CLEAR]"));

        state.query.clear();
        state.refresh(&charts);
        state.key(KeyCode::End, &charts);
        let mut out = Vec::new();
        SelectorFrame::default()
            .draw_if_changed(&mut out, &charts, &mut state, &progress, (100, 12))
            .unwrap();
        let output = String::from_utf8(out).unwrap();
        assert!(state.scroll > 0);
        assert!(output.contains("[BEST 7.0% CLEAR] ▸ [5K NORMAL Lv3 BPM140]  Song 7"));
    }

    #[test]
    fn chart_rows_use_only_the_display_columns_after_the_margin() {
        let prefix = "[UNPLAYED] ▸ [5K NORMAL Lv3 BPM140]  ";
        let ascii = "a".repeat(5);
        for (title, visible) in [
            ("a".repeat(100), ascii.as_str()),
            ("日本語".repeat(30), "日本"),
            ("日本a".into(), "日本a"),
        ] {
            let charts = [meta(&title)];
            let mut state = SelectorState::default();
            state.refresh(&charts);
            let mut out = Vec::new();
            let cols = 2 + prefix.chars().count() as u16 + 5;
            SelectorFrame::default()
                .draw_if_changed(
                    &mut out,
                    &charts,
                    &mut state,
                    &["[UNPLAYED]".into()],
                    (cols, 12),
                )
                .unwrap();
            let output = String::from_utf8(out).unwrap();
            assert!(output.contains(&format!("{prefix}{visible}\x1b[")));
        }
    }

    #[test]
    fn search_matches_title_subtitle_and_artist_case_insensitively() {
        let mut charts = vec![meta("ÉTOILE"), meta("Other"), meta("Third")];
        charts[1].meta.subtitle = "Étoile remix".into();
        charts[2].meta.artist = "étoile".into();
        let mut state = SelectorState {
            query: "éTOILE".into(),
            ..Default::default()
        };
        state.refresh(&charts);
        assert_eq!(state.indices, vec![0, 1, 2]);
        charts[1].meta.lane_count = 7;
        state.lanes = Some(7);
        state.refresh(&charts);
        assert_eq!(state.indices, vec![1]);
        assert_eq!(state.selected(), Some(1));
        assert_eq!(charts[0].title, "ÉTOILE");
    }

    #[test]
    fn search_matches_unicode_case_folds_in_each_metadata_field() {
        let mut charts = [meta("ΟΣ"), meta("Other"), meta("Third"), meta("Unrelated")];
        charts[1].meta.subtitle = "Remix ΟΣ".into();
        charts[2].meta.artist = "DJ ΟΣ".into();
        let mut state = SelectorState::default();

        ["ΟΣ", "ος", "οσ"].into_iter().for_each(|query| {
            state.query = query.into();
            state.refresh(&charts);
            assert_eq!(state.indices, [0, 1, 2], "query: {query}");
        });
    }

    #[test]
    fn search_matches_full_case_fold_expansions_and_ascii_case() {
        let charts = [meta("Die Straße"), meta("Unrelated")];
        let mut state = SelectorState::default();

        ["strasse", "STRASSE", "sTrAsSe", "dIE"]
            .into_iter()
            .for_each(|query| {
                state.query = query.into();
                state.refresh(&charts);
                assert_eq!(state.indices, [0], "query: {query}");
            });
    }

    #[test]
    fn rescan_refreshes_metadata_and_reordered_search_entries_at_the_same_count() {
        let dir = tempdir();
        let path = dir.join("song.bms");
        std::fs::write(
            &path,
            "#TITLE Alpha\n#SUBTITLE Old remix\n#ARTIST Original\n",
        )
        .unwrap();
        std::fs::write(dir.join("other.bms"), "#TITLE Middle\n").unwrap();
        let mut state = SelectorState::default();
        let charts = state.scan(&dir);
        state.refresh(&charts);
        assert_eq!(charts[0].path, path);

        std::fs::write(
            &path,
            "#TITLE Omega\n#SUBTITLE New remix\n#ARTIST Updated\n",
        )
        .unwrap();
        let rescanned = state.scan(&dir);
        assert_eq!(rescanned.len(), charts.len());
        assert_eq!(rescanned[1].path, path);
        [
            "Omega",
            "New remix",
            "Updated",
            "Middle",
            "Alpha",
            "Old remix",
            "Original",
        ]
        .into_iter()
        .zip([vec![1], vec![1], vec![1], vec![0], vec![], vec![], vec![]])
        .for_each(|(query, expected)| {
            state.query = query.into();
            state.refresh(&rescanned);
            assert_eq!(state.indices, expected, "query: {query}");
        });
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn search_keystrokes_reuse_cached_case_folded_string_allocations() {
        let charts = vec![meta("Alpha"), meta("Beta")];
        let mut state = SelectorState::default();
        state.refresh(&charts);
        let allocations = |state: &SelectorState| {
            state
                .case_folded_cache
                .iter()
                .map(|(title, subtitle, artist)| {
                    (
                        title.as_ptr() as usize,
                        subtitle.as_ptr() as usize,
                        artist.as_ptr() as usize,
                    )
                })
                .collect::<Vec<_>>()
        };
        let before = allocations(&state);
        state.key(KeyCode::Char('/'), &charts);
        state.key(KeyCode::Char('B'), &charts);
        assert_eq!(state.indices, vec![1]);
        assert_eq!(allocations(&state), before);
        state.key(KeyCode::Char('e'), &charts);
        assert_eq!(state.indices, vec![1]);
        assert_eq!(allocations(&state), before);
        state.key(KeyCode::Backspace, &charts);
        assert_eq!(state.indices, vec![1]);
        assert_eq!(allocations(&state), before);
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
        charts[1].meta.lane_count = 7;
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
    fn clear_filter_cycles_through_all_cleared_and_uncleared() {
        let charts = vec![meta("Alpha"), meta("Beta"), meta("Gamma")];
        let mut state = SelectorState::default();
        state.set_clears(vec![Some(true), Some(false), None]);
        state.refresh(&charts);
        assert_eq!(state.indices, vec![0, 1, 2]);

        state.key(KeyCode::Char('v'), &charts);
        assert_eq!(state.clear_filter, Some(true));
        assert_eq!(state.indices, vec![0]);

        state.key(KeyCode::Char('v'), &charts);
        assert_eq!(state.clear_filter, Some(false));
        assert_eq!(state.indices, vec![1, 2]);

        state.key(KeyCode::Char('v'), &charts);
        assert_eq!(state.clear_filter, None);
        assert_eq!(state.indices, vec![0, 1, 2]);
    }

    #[test]
    fn clear_filter_shows_everything_when_records_are_unavailable() {
        let charts = vec![meta("Alpha"), meta("Beta")];
        let mut state = SelectorState::default();
        state.refresh(&charts);
        state.key(KeyCode::Char('v'), &charts);
        assert_eq!(state.clear_filter, Some(true));
        assert_eq!(state.indices, vec![0, 1]);
    }

    #[test]
    fn c_resets_clear_filter_along_with_lanes_and_query() {
        let mut charts = vec![meta("Alpha"), meta("Beta")];
        charts[1].meta.lane_count = 7;
        let mut state = SelectorState::default();
        state.set_clears(vec![Some(true), None]);
        state.refresh(&charts);
        state.key(KeyCode::Char('v'), &charts);
        state.key(KeyCode::Char('l'), &charts);
        state.key(KeyCode::Char('/'), &charts);
        state.key(KeyCode::Char('Z'), &charts);
        state.key(KeyCode::Esc, &charts);
        assert_eq!(state.clear_filter, Some(true));
        assert_eq!(state.lanes, Some(5));
        assert_eq!(state.query, "Z");
        assert!(state.indices.is_empty());

        state.key(KeyCode::Char('c'), &charts);
        assert_eq!(state.clear_filter, None);
        assert_eq!(state.lanes, None);
        assert_eq!(state.query, "");
        assert_eq!(state.indices, vec![0, 1]);
    }

    #[test]
    fn status_line_reports_the_active_clear_filter() {
        let charts = vec![meta("Alpha")];
        let mut state = SelectorState::default();
        state.set_clears(vec![Some(true)]);
        state.refresh(&charts);
        state.key(KeyCode::Char('v'), &charts);
        let mut out = Vec::new();
        SelectorFrame::default()
            .draw_if_changed(
                &mut out,
                &charts,
                &mut state,
                &["[BEST 100.0% CLEAR]".into()],
                (100, 24),
            )
            .unwrap();
        let output = String::from_utf8(out).unwrap();
        assert!(output.contains("Clear: cleared"));
    }

    #[test]
    fn selected_details_render_banner_and_sanitized_comment() {
        crossterm::style::force_color_output(true);
        let dir = tempdir();
        image::RgbImage::from_pixel(4, 2, image::Rgb([255, 0, 0]))
            .save(dir.join("banner.png"))
            .unwrap();
        let mut chart = meta("Song");
        chart.meta.path = dir.join("song.bms");
        chart.meta.banner = "banner.png".into();
        chart.meta.comment = "Great song!\x1b\x07".into();
        let mut out = Vec::new();
        draw_details(
            &mut out,
            &chart,
            40,
            10,
            chart_banner(&chart, 40).as_deref(),
        )
        .unwrap();
        let output = String::from_utf8(out).unwrap();
        assert!(output.contains("Great song!"));
        assert!(!output.contains('\x07'));
        assert!(output.contains('▀'));
        assert!(output.contains("38;2;255;0;0"));
    }

    #[test]
    fn chart_row_sanitizes_metadata_before_clipping() {
        let mut chart = meta("曲名\x1b]52;c;VEVTVA==\x07");
        chart.meta.subtitle = "副題\u{9b}2J".into();
        chart.meta.artist = "作者\r\n\t\x7f".into();
        let row = format_chart_row(&chart, "▸ ");
        assert!(row.ends_with("曲名]52;c;VEVTVA==  ~副題2J~  — 作者"));
        assert!(!row.chars().any(char::is_control));
    }

    #[test]
    fn chart_row_sanitizes_filename_fallback() {
        let mut chart = meta("");
        chart.meta.path = PathBuf::from("曲名\x1b]52;c;VEVTVA==\x07.bms");
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
        m.meta.difficulty = None;
        assert_eq!(format_badge(&m), "[5K Lv3 BPM140]");
    }

    #[test]
    fn format_badge_omits_level_when_missing() {
        let mut m = meta("A");
        m.meta.playlevel = None;
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
        m.meta.difficulty = None;
        m.meta.playlevel = None;
        assert_eq!(format_badge(&m), "[5K BPM140]");
    }

    #[test]
    fn bmson_levels_survive_discovery_and_loading_and_display_in_badges() {
        let dir = tempdir();
        let path = dir.join("chart.bmson");
        let fixture = include_str!("../tests/fixtures/minimal.bmson");
        for (level, badge) in [
            (Some(256), "[4K Lv256 BPM120]"),
            (Some(u32::MAX), "[4K Lv4294967295 BPM120]"),
            (None, "[4K BPM120]"),
        ] {
            let mut data: serde_json::Value = serde_json::from_str(fixture).unwrap();
            let info = data["info"].as_object_mut().unwrap();
            match level {
                Some(level) => info.insert("level".into(), level.into()),
                None => info.remove("level"),
            };
            std::fs::write(&path, data.to_string()).unwrap();
            let charts = scan(&dir);
            assert_eq!(charts.len(), 1);
            assert_eq!(charts[0].playlevel, level);
            assert_eq!(format_badge(&charts[0]), badge);
            assert_eq!(crate::bmson::load(&path, 0.0).unwrap().playlevel, level);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn format_badge_rounds_bpm_to_the_nearest_integer() {
        let mut m = meta("A");
        m.meta.bpm = 173.6;
        assert_eq!(format_badge(&m), "[5K NORMAL Lv3 BPM174]");
    }

    #[test]
    fn load_clears_reports_best_clear_status_per_chart_or_none_when_unavailable() {
        let charts = [meta("Alpha"), meta("Beta")];
        assert_eq!(load_clears(&charts, None), Vec::<Option<bool>>::new());

        let mut store = crate::results::Store::default();
        let mut game = crate::game::Game::new(crate::chart::built_in(120.0, 0.0));
        game.completed = true;
        game.perfect = 3;
        game.miss = 1;
        store.record(&charts[0].identity, &game, false);
        assert_eq!(load_clears(&charts, Some(&store)), vec![Some(false), None]);
        (0..1000).for_each(|_| game.gauge.apply(crate::game::Judgment::Perfect));
        store.record(&charts[0].identity, &game, false);
        assert_eq!(load_clears(&charts, Some(&store)), vec![Some(true), None]);
    }

    pub(super) fn tempdir() -> PathBuf {
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

#[cfg(test)]
mod progress_tests {
    use super::*;

    #[test]
    fn progress_shows_unplayed_best_accuracy_and_clear_achievement() {
        assert_eq!(format_progress(None), "[UNPLAYED]");
        let mut store = crate::results::Store::default();
        let id = crate::results::Identity::built_in(120.0);
        let mut game = crate::game::Game::new(crate::chart::built_in(120.0, 0.0));
        game.completed = true;
        game.perfect = 3;
        game.miss = 1;
        store.record(&id, &game, false);
        assert_eq!(format_progress(store.get(&id)), "[BEST 75.0% FAILED]");
        (0..1000).for_each(|_| game.gauge.apply(crate::game::Judgment::Perfect));
        store.record(&id, &game, false);
        assert_eq!(format_progress(store.get(&id)), "[BEST 75.0% CLEAR]");
    }
}
