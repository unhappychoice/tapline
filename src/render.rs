use crate::chart::difficulty_label;
use crate::game::{Game, Judgment};
use crossterm::{
    cursor, queue,
    style::{self, Color, Print, ResetColor, SetForegroundColor},
    terminal,
};
use std::io::{Stdout, Write};

pub const APPROACH_MS: f64 = 1500.0;
pub const LANE_WIDTH: u16 = 7;

const PALETTE: [Color; 7] = [
    Color::Red,
    Color::Yellow,
    Color::Green,
    Color::White,
    Color::Cyan,
    Color::Blue,
    Color::Magenta,
];

fn lane_color(lane: usize) -> Color {
    PALETTE[lane % PALETTE.len()]
}

/// Rectangles for the play field, computed from terminal size and lane count.
/// Extracted so the layout math is testable without a real terminal.
#[derive(Debug, Clone, Copy, PartialEq)]
struct PlayLayout {
    x0: u16,
    field_width: u16,
    top: u16,
    bottom: u16,
    lane_height: u16,
}

impl PlayLayout {
    fn compute(cols: u16, rows: u16, lane_count: usize) -> Option<Self> {
        let lanes = lane_count as u16;
        let field_width = LANE_WIDTH * lanes + 1;
        let x0 = cols.saturating_sub(field_width) / 2;
        let top = 4u16;
        let bottom = rows.saturating_sub(4);
        if bottom <= top + 4 {
            return None;
        }
        Some(Self {
            x0,
            field_width,
            top,
            bottom,
            lane_height: bottom - top,
        })
    }

    fn judgment_row(&self) -> u16 {
        self.bottom - 1
    }
}

/// Vertical screen position for a note whose time offset from now is
/// `remain` ms. `None` if the note is outside the approach or already
/// past the miss window.
fn note_screen_y(remain: f64, layout: &PlayLayout) -> Option<u16> {
    if !(-60.0..=APPROACH_MS).contains(&remain) {
        return None;
    }
    let frac = (1.0 - (remain / APPROACH_MS)).clamp(0.0, 1.0);
    Some(layout.top + (frac * (layout.lane_height - 1) as f64) as u16)
}

fn draw_judgment_flash(
    out: &mut Stdout,
    game: &Game,
    cols: u16,
    judgment_row: u16,
    now_ms: f64,
) -> anyhow::Result<()> {
    let elapsed = now_ms - game.flash.last_judgment_at;
    if elapsed >= 500.0 || game.flash.last_judgment.is_none() {
        return Ok(());
    }
    let (label, color) = match game.flash.last_judgment.unwrap() {
        Judgment::Perfect => ("P E R F E C T", Color::Magenta),
        Judgment::Great => ("G R E A T", Color::Green),
        Judgment::Good => ("G O O D", Color::Yellow),
        Judgment::Miss => ("M I S S", Color::Red),
    };
    let row = judgment_row.saturating_sub(2);
    let width = label.chars().count() as u16;
    let x = cols.saturating_sub(width) / 2;
    let fade = elapsed < 120.0;
    queue!(out, cursor::MoveTo(x, row), SetForegroundColor(color))?;
    if fade {
        queue!(out, style::SetAttribute(style::Attribute::Reverse))?;
    }
    queue!(
        out,
        style::SetAttribute(style::Attribute::Bold),
        Print(label),
        style::SetAttribute(style::Attribute::Reset),
        ResetColor
    )?;
    Ok(())
}

fn draw_judgment_counters(
    out: &mut Stdout,
    game: &Game,
    cols: u16,
    row: u16,
    _now_ms: f64,
) -> anyhow::Result<()> {
    let cells: [(&str, u32, Color); 4] = [
        ("PERFECT", game.perfect, Color::Magenta),
        ("GREAT", game.great, Color::Green),
        ("GOOD", game.good, Color::Yellow),
        ("MISS", game.miss, Color::Red),
    ];
    let strs: Vec<String> = cells
        .iter()
        .map(|(l, n, _)| format!("{} {}", l, n))
        .collect();
    let gap: usize = 4;
    let total_len: usize =
        strs.iter().map(|s| s.chars().count()).sum::<usize>() + gap * (cells.len() - 1);
    let mut x = cols.saturating_sub(total_len as u16) / 2;
    for (i, (_, _, color)) in cells.iter().enumerate() {
        queue!(
            out,
            cursor::MoveTo(x, row),
            SetForegroundColor(*color),
            Print(&strs[i]),
            ResetColor
        )?;
        x += strs[i].chars().count() as u16 + gap as u16;
    }
    Ok(())
}

fn format_lane_key(keys: &[char]) -> String {
    let labels: Vec<String> = keys.iter().map(display_key).collect();
    let joined = labels.join("/");
    let visible_width = joined.chars().count();
    let target = 3;
    if visible_width >= target {
        joined
    } else {
        format!("{:^3}", joined)
    }
}

fn format_key_hint(keys: &[Vec<char>]) -> String {
    keys.iter()
        .map(|ks| ks.iter().map(display_key).collect::<Vec<_>>().join("/"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn display_key(c: &char) -> String {
    if *c == ' ' {
        "SPACE".to_string()
    } else {
        c.to_string()
    }
}

fn format_difficulty_badge(game: &Game) -> String {
    let lv = game.chart.playlevel.map(|v| format!("Lv {}", v));
    let dif = match difficulty_label(game.chart.difficulty) {
        "" => None,
        s => Some(s.to_string()),
    };
    let bpm = format!("BPM {:.0}", game.chart.bpm);
    let mut parts: Vec<String> = Vec::new();
    if let Some(d) = dif {
        parts.push(d);
    }
    if let Some(l) = lv {
        parts.push(l);
    }
    parts.push(bpm);
    parts.push(format!("{}K", game.chart.lane_count));
    parts.join("  ·  ")
}

pub fn draw(out: &mut Stdout, game: &Game, now_ms: f64) -> anyhow::Result<()> {
    let (cols, rows) = terminal::size()?;
    let lanes = game.chart.lane_count as u16;
    let Some(layout) = PlayLayout::compute(cols, rows, game.chart.lane_count) else {
        return Ok(());
    };
    let PlayLayout {
        x0,
        field_width,
        top,
        bottom,
        lane_height: _,
    } = layout;

    queue!(out, terminal::BeginSynchronizedUpdate)?;

    let title = if game.chart.title.is_empty() {
        "TAPLINE".to_string()
    } else {
        game.chart.title.clone()
    };
    queue!(
        out,
        cursor::MoveTo(cols.saturating_sub(title.chars().count() as u16) / 2, 1),
        SetForegroundColor(Color::Magenta),
        Print(&title),
        ResetColor
    )?;

    let hud = format!(
        "score {:>6}    combo {:>3}    acc {:>5.1}%",
        game.score,
        game.combo,
        game.accuracy()
    );
    queue!(
        out,
        cursor::MoveTo(cols.saturating_sub(hud.len() as u16) / 2, 2),
        Print(hud)
    )?;

    draw_judgment_counters(out, game, cols, 3, now_ms)?;

    for lane in 0..lanes {
        let lx = x0 + 1 + lane * LANE_WIDTH;
        for r in top..bottom {
            queue!(
                out,
                cursor::MoveTo(lx, r),
                SetForegroundColor(Color::DarkGrey),
                Print("│      ")
            )?;
        }
        queue!(out, cursor::MoveTo(lx + LANE_WIDTH, top), Print("│"))?;
    }
    queue!(out, ResetColor)?;

    let judgment_row = layout.judgment_row();
    for x in x0..(x0 + field_width) {
        queue!(
            out,
            cursor::MoveTo(x, judgment_row),
            SetForegroundColor(Color::White),
            Print("═")
        )?;
    }
    queue!(out, ResetColor)?;

    for lane in 0..(lanes as usize) {
        let lx = x0 + 1 + lane as u16 * LANE_WIDTH + 3;
        let flash = now_ms
            - game
                .flash
                .last_lane_hit
                .get(lane)
                .copied()
                .unwrap_or(-9999.0)
            < 120.0;
        let color = if flash {
            Color::White
        } else {
            lane_color(lane)
        };
        let display = format_lane_key(
            game.chart
                .keys
                .get(lane)
                .map(|v| v.as_slice())
                .unwrap_or(&[]),
        );
        queue!(
            out,
            cursor::MoveTo(lx.saturating_sub(1), bottom + 1),
            SetForegroundColor(color),
            style::SetAttribute(style::Attribute::Bold),
            Print(display),
            style::SetAttribute(style::Attribute::Reset),
            ResetColor
        )?;
    }

    for note in &game.chart.notes {
        if note.hit {
            continue;
        }
        let Some(y) = note_screen_y(note.time_ms - now_ms, &layout) else {
            continue;
        };
        let lx = x0 + 1 + note.lane as u16 * LANE_WIDTH + 2;
        queue!(
            out,
            cursor::MoveTo(lx, y),
            SetForegroundColor(lane_color(note.lane)),
            Print("[==]"),
            ResetColor
        )?;
    }

    draw_judgment_flash(out, game, cols, layout.judgment_row(), now_ms)?;

    let key_hint = format_key_hint(&game.chart.keys);
    let hint = format!("keys {}  ·  quit Esc / Q", key_hint);
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

pub fn draw_intro(
    out: &mut Stdout,
    game: &Game,
    countdown_ms: f64,
    audio_on: bool,
) -> anyhow::Result<()> {
    let (cols, rows) = terminal::size()?;
    queue!(out, terminal::BeginSynchronizedUpdate)?;
    let title = if game.chart.title.is_empty() {
        "T A P L I N E".to_string()
    } else {
        game.chart.title.clone()
    };
    queue!(
        out,
        cursor::MoveTo(
            cols.saturating_sub(title.chars().count() as u16) / 2,
            rows / 2 - 3
        ),
        SetForegroundColor(Color::Magenta),
        style::SetAttribute(style::Attribute::Bold),
        Print(&title),
        style::SetAttribute(style::Attribute::Reset),
        ResetColor
    )?;
    if !game.chart.artist.is_empty() {
        let art = format!("— {}", game.chart.artist);
        queue!(
            out,
            cursor::MoveTo(
                cols.saturating_sub(art.chars().count() as u16) / 2,
                rows / 2 - 2
            ),
            SetForegroundColor(Color::DarkGrey),
            Print(&art),
            ResetColor
        )?;
    }
    let badge = format_difficulty_badge(game);
    if !badge.is_empty() {
        queue!(
            out,
            cursor::MoveTo(
                cols.saturating_sub(badge.chars().count() as u16) / 2,
                rows / 2 - 1
            ),
            SetForegroundColor(Color::Cyan),
            Print(&badge),
            ResetColor
        )?;
    }
    let key_hint = format_key_hint(&game.chart.keys);
    let msg = format!("hit  {}  on the line", key_hint);
    queue!(
        out,
        cursor::MoveTo(
            cols.saturating_sub(msg.chars().count() as u16) / 2,
            rows / 2
        ),
        Print(msg)
    )?;
    let n = (countdown_ms / 1000.0).ceil() as u32;
    let count = format!("{}", n.max(1));
    queue!(
        out,
        cursor::MoveTo(cols.saturating_sub(count.len() as u16) / 2, rows / 2 + 2),
        SetForegroundColor(Color::Yellow),
        style::SetAttribute(style::Attribute::Bold),
        Print(count),
        style::SetAttribute(style::Attribute::Reset),
        ResetColor
    )?;
    let badge = if audio_on { "audio on" } else { "silent" };
    queue!(
        out,
        cursor::MoveTo(cols.saturating_sub(badge.len() as u16) / 2, rows - 2),
        SetForegroundColor(Color::DarkGrey),
        Print(badge),
        ResetColor
    )?;
    queue!(out, terminal::EndSynchronizedUpdate)?;
    out.flush()?;
    Ok(())
}

pub fn draw_result(out: &mut Stdout, game: &Game) -> anyhow::Result<()> {
    let (cols, rows) = terminal::size()?;
    queue!(out, terminal::BeginSynchronizedUpdate)?;
    let mut y = rows / 2 - 4;
    let line = |o: &mut Stdout, text: String, color: Color, yy: u16| -> anyhow::Result<()> {
        queue!(
            o,
            cursor::MoveTo(cols.saturating_sub(text.chars().count() as u16) / 2, yy),
            SetForegroundColor(color),
            Print(text),
            ResetColor
        )?;
        Ok(())
    };
    line(out, "R E S U L T".to_string(), Color::Magenta, y)?;
    y += 2;
    line(
        out,
        format!("score       {:>6}", game.score),
        Color::White,
        y,
    )?;
    y += 1;
    line(
        out,
        format!("max combo   {:>6}", game.max_combo),
        Color::Cyan,
        y,
    )?;
    y += 1;
    line(
        out,
        format!("accuracy    {:>5.1}%", game.accuracy()),
        Color::Green,
        y,
    )?;
    y += 2;
    line(
        out,
        format!(
            "perfect {}   great {}   good {}   miss {}",
            game.perfect, game.great, game.good, game.miss
        ),
        Color::DarkGrey,
        y,
    )?;
    y += 2;
    line(out, "press any key to exit".to_string(), Color::DarkGrey, y)?;
    queue!(out, terminal::EndSynchronizedUpdate)?;
    out.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chart::{keys_for, Chart, Note};
    use crate::game::Game;

    fn base_chart(lane_count: usize) -> Chart {
        Chart {
            title: "Song".into(),
            artist: "Artist".into(),
            bpm: 140.0,
            playlevel: Some(4),
            difficulty: Some(2),
            notes: vec![Note {
                time_ms: 1000.0,
                lane: 0,
                hit: false,
                keysound: None,
            }],
            duration_ms: 30_000.0,
            lane_count,
            keys: keys_for(lane_count),
            ..Chart::default()
        }
    }

    #[test]
    fn display_key_shows_letters_verbatim_and_expands_space() {
        assert_eq!(display_key(&'A'), "A");
        assert_eq!(display_key(&'S'), "S");
        assert_eq!(display_key(&' '), "SPACE");
    }

    #[test]
    fn format_lane_key_centers_a_single_letter_in_a_three_wide_cell() {
        assert_eq!(format_lane_key(&['S']), " S ");
    }

    #[test]
    fn format_lane_key_joins_multiple_bindings_with_slash() {
        assert_eq!(format_lane_key(&['F', 'J']), "F/J");
    }

    #[test]
    fn format_lane_key_renders_space_as_word() {
        assert_eq!(format_lane_key(&[' ']), "SPACE");
    }

    #[test]
    fn format_lane_key_handles_empty_bindings() {
        assert_eq!(format_lane_key(&[]), "   ");
    }

    #[test]
    fn format_key_hint_lists_each_lane_space_separated() {
        let keys = keys_for(5);
        // 5K: S D F/J K L
        assert_eq!(format_key_hint(&keys), "S D F/J K L");
    }

    #[test]
    fn format_key_hint_swaps_space_char_for_the_word_space() {
        let keys = keys_for(7);
        assert_eq!(format_key_hint(&keys), "S D F SPACE J K L");
    }

    #[test]
    fn format_difficulty_badge_full_metadata() {
        let g = Game::new(base_chart(5));
        assert_eq!(
            format_difficulty_badge(&g),
            "NORMAL  ·  Lv 4  ·  BPM 140  ·  5K"
        );
    }

    #[test]
    fn format_difficulty_badge_drops_missing_pieces() {
        let mut chart = base_chart(4);
        chart.difficulty = None;
        chart.playlevel = None;
        let g = Game::new(chart);
        assert_eq!(format_difficulty_badge(&g), "BPM 140  ·  4K");
    }

    #[test]
    fn format_difficulty_badge_shows_only_bpm_and_lanes_for_bare_chart() {
        let mut chart = base_chart(7);
        chart.difficulty = None;
        chart.playlevel = None;
        chart.bpm = 200.0;
        let g = Game::new(chart);
        assert_eq!(format_difficulty_badge(&g), "BPM 200  ·  7K");
    }

    // ---------- PlayLayout / note_screen_y ----------

    #[test]
    fn play_layout_computes_the_centered_playfield_for_4k() {
        let l = PlayLayout::compute(80, 24, 4).unwrap();
        assert_eq!(l.top, 4);
        assert_eq!(l.bottom, 20);
        assert_eq!(l.lane_height, 16);
        assert_eq!(l.field_width, LANE_WIDTH * 4 + 1);
        // Field is centered on 80 cols.
        assert_eq!(l.x0, (80 - l.field_width) / 2);
        assert_eq!(l.judgment_row(), 19);
    }

    #[test]
    fn play_layout_returns_none_for_tiny_windows() {
        // A window that can't fit `top + 4` rows below the header returns
        // None, which draw() uses to bail out early instead of panicking.
        assert!(PlayLayout::compute(80, 8, 4).is_none());
        assert!(PlayLayout::compute(80, 6, 4).is_none());
        assert!(PlayLayout::compute(80, 4, 4).is_none());
    }

    #[test]
    fn play_layout_field_width_scales_with_lane_count() {
        let four = PlayLayout::compute(80, 24, 4).unwrap();
        let seven = PlayLayout::compute(80, 24, 7).unwrap();
        assert!(
            seven.field_width > four.field_width,
            "7K should be wider than 4K"
        );
    }

    #[test]
    fn note_screen_y_returns_none_when_note_is_far_in_the_future() {
        let l = PlayLayout::compute(80, 24, 4).unwrap();
        assert!(note_screen_y(APPROACH_MS + 1.0, &l).is_none());
    }

    #[test]
    fn note_screen_y_returns_none_when_note_is_more_than_60ms_stale() {
        let l = PlayLayout::compute(80, 24, 4).unwrap();
        assert!(note_screen_y(-100.0, &l).is_none());
    }

    #[test]
    fn note_screen_y_at_approach_horizon_sits_near_the_top() {
        let l = PlayLayout::compute(80, 24, 4).unwrap();
        let y = note_screen_y(APPROACH_MS, &l).unwrap();
        assert_eq!(y, l.top);
    }

    #[test]
    fn note_screen_y_on_the_beat_sits_near_the_bottom_of_the_lane() {
        let l = PlayLayout::compute(80, 24, 4).unwrap();
        let y = note_screen_y(0.0, &l).unwrap();
        assert_eq!(y, l.top + (l.lane_height - 1));
    }

    #[test]
    fn note_screen_y_grows_monotonically_as_remain_shrinks() {
        let l = PlayLayout::compute(80, 24, 4).unwrap();
        let mut prev = None::<u16>;
        for step in (0..=APPROACH_MS as u32).step_by(100) {
            let remain = APPROACH_MS - step as f64;
            let y = note_screen_y(remain, &l).unwrap();
            if let Some(p) = prev {
                assert!(
                    y >= p,
                    "y went backwards at remain={}: {} < {}",
                    remain,
                    y,
                    p
                );
            }
            prev = Some(y);
        }
    }

    // ------------- property tests -------------

    use proptest::prelude::*;

    proptest! {
        #[test]
        fn display_key_is_never_empty_for_any_char(c in any::<char>()) {
            let out = display_key(&c);
            prop_assert!(!out.is_empty());
        }

        #[test]
        fn format_lane_key_never_panics_on_arbitrary_keys(
            keys in prop::collection::vec(any::<char>(), 0..8)
        ) {
            let _ = format_lane_key(&keys);
        }

        #[test]
        fn format_lane_key_is_at_least_three_wide_for_letter_input(
            keys in prop::collection::vec("[A-Z]", 1..4)
        ) {
            let chars: Vec<char> = keys.iter().map(|s| s.chars().next().unwrap()).collect();
            let out = format_lane_key(&chars);
            prop_assert!(out.chars().count() >= 3, "got {:?}", out);
        }

        #[test]
        fn format_key_hint_yields_one_group_per_lane(
            n in 1usize..8
        ) {
            let lanes: Vec<Vec<char>> = (0..n).map(|_| vec!['A']).collect();
            let out = format_key_hint(&lanes);
            let parts: Vec<_> = out.split_whitespace().collect();
            prop_assert_eq!(parts.len(), n);
        }

        #[test]
        fn format_difficulty_badge_always_contains_bpm_and_lane_marker(
            bpm in 30.0f64..300.0,
            lane_count in prop_oneof![Just(4usize), Just(5), Just(7)],
            playlevel in prop::option::of(1u8..=12),
            difficulty in prop::option::of(1u8..=5),
        ) {
            let mut chart = base_chart(lane_count);
            chart.bpm = bpm;
            chart.playlevel = playlevel;
            chart.difficulty = difficulty;
            let g = Game::new(chart);
            let badge = format_difficulty_badge(&g);
            prop_assert!(badge.contains("BPM"), "expected BPM in {}", badge);
            prop_assert!(
                badge.contains(&format!("{}K", lane_count)),
                "expected {}K in {}",
                lane_count,
                badge
            );
        }
    }
}
