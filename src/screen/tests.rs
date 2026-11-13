use super::*;
use crossterm::{
    cursor, queue,
    style::{Attribute, Color, Print, SetAttribute, SetForegroundColor},
    terminal,
};

fn screen(cols: u16, rows: u16) -> Screen {
    let mut screen = Screen::new();
    screen.resize(cols, rows);
    screen
}

fn present(screen: &mut Screen) -> String {
    let mut out = Vec::new();
    screen.present(&mut out).unwrap();
    String::from_utf8(out).unwrap()
}

fn row_text(screen: &Screen, y: u16) -> String {
    (0..screen.cols)
        .map(|x| screen.cell(x, y).ch)
        .filter(|&ch| ch != CONTINUATION)
        .collect()
}

#[test]
fn first_present_sends_every_cell() {
    let mut s = screen(4, 2);
    queue!(s, cursor::MoveTo(1, 1), Print("ab")).unwrap();
    let out = present(&mut s);
    assert_eq!(out.matches(' ').count(), 6);
    assert!(out.contains("ab"));
}

#[test]
fn unchanged_frames_send_no_cells() {
    let mut s = screen(10, 3);
    queue!(s, cursor::MoveTo(2, 1), Print("hello")).unwrap();
    present(&mut s);
    queue!(s, cursor::MoveTo(2, 1), Print("hello")).unwrap();
    assert_eq!(present(&mut s), "\x1b[?2026h\x1b[0m\x1b[?2026l");
}

#[test]
fn only_changed_cells_are_sent_with_their_position() {
    let mut s = screen(10, 3);
    queue!(s, cursor::MoveTo(0, 0), Print("abcdef")).unwrap();
    present(&mut s);
    queue!(s, cursor::MoveTo(0, 0), Print("abXdef")).unwrap();
    let out = present(&mut s);
    assert!(out.contains("\x1b[1;3H"));
    assert!(out.contains('X'));
    assert!(!out.contains('a') && !out.contains('f'));
}

#[test]
fn style_changes_are_tracked_per_cell() {
    let mut s = screen(6, 1);
    queue!(s, SetForegroundColor(Color::Blue), Print("ab")).unwrap();
    present(&mut s);
    queue!(
        s,
        cursor::MoveTo(0, 0),
        SetForegroundColor(Color::Red),
        Print("a")
    )
    .unwrap();
    let out = present(&mut s);
    assert!(out.contains("\x1b[0;38;5;9ma"), "{out:?}");
    assert!(!out.contains('b'));
}

#[test]
fn sgr_parsing_handles_rgb_reverse_and_resets() {
    let mut style = Style::DEFAULT;
    style.apply_sgr("1;38;2;10;20;30;7");
    assert_eq!(style.to_sgr(), "\x1b[0;1;7;38;2;10;20;30m");
    style.apply_sgr("27;39");
    assert_eq!(style.to_sgr(), "\x1b[0;1m");
    style.apply_sgr("0");
    assert_eq!(style, Style::DEFAULT);
}

#[test]
fn reverse_attribute_round_trips_through_crossterm_output() {
    let mut s = screen(2, 1);
    queue!(s, SetAttribute(Attribute::Reverse), Print("x")).unwrap();
    assert!(present(&mut s).contains("\x1b[0;7mx"));
}

#[test]
fn clear_commands_blank_the_model() {
    let mut s = screen(5, 2);
    queue!(s, Print("abcde"), cursor::MoveTo(0, 1), Print("fghij")).unwrap();
    queue!(
        s,
        cursor::MoveTo(2, 0),
        terminal::Clear(terminal::ClearType::UntilNewLine)
    )
    .unwrap();
    assert_eq!(row_text(&s, 0), "ab   ");
    queue!(s, terminal::Clear(terminal::ClearType::CurrentLine)).unwrap();
    assert_eq!(row_text(&s, 0), "     ");
    queue!(s, terminal::Clear(terminal::ClearType::All)).unwrap();
    assert_eq!(row_text(&s, 1), "     ");
}

#[test]
fn wide_characters_occupy_two_cells_and_are_replaced_cleanly() {
    let mut s = screen(6, 1);
    queue!(s, Print("日本")).unwrap();
    assert_eq!(row_text(&s, 0), "日本  ");
    queue!(s, cursor::MoveTo(1, 0), Print("x")).unwrap();
    assert_eq!(row_text(&s, 0), " x本  ");
}

#[test]
fn split_writes_of_multibyte_text_and_escapes_are_buffered() {
    let mut s = screen(6, 1);
    let bytes = "\x1b[1;3H█".as_bytes();
    bytes.iter().for_each(|b| {
        s.write_all(std::slice::from_ref(b)).unwrap();
    });
    assert_eq!(row_text(&s, 0), "  █   ");
}

#[test]
fn resize_forces_a_full_redraw_and_clips_out_of_bounds_writes() {
    let mut s = screen(3, 1);
    queue!(s, Print("abc")).unwrap();
    present(&mut s);
    s.resize(4, 1);
    queue!(
        s,
        cursor::MoveTo(0, 0),
        Print("abcdef"),
        cursor::MoveTo(9, 9),
        Print("z")
    )
    .unwrap();
    let out = present(&mut s);
    assert!(out.contains("abcd"));
    assert!(!out.contains('z'));
}

#[test]
fn presented_diffs_reproduce_the_model_on_a_mirror_terminal() {
    let (mut model, mut mirror) = (screen(12, 4), screen(12, 4));
    let frames = ["日本語ab", "x日本", "████", "", "▄▄ ▀▀"];
    frames.iter().enumerate().for_each(|(i, text)| {
        let color = [Color::Blue, Color::Red, Color::Yellow][i % 3];
        queue!(
            model,
            cursor::MoveTo(i as u16 % 5, i as u16 % 4),
            SetForegroundColor(color),
            Print(text),
            SetAttribute(Attribute::Reset)
        )
        .unwrap();
        mirror.write_all(present(&mut model).as_bytes()).unwrap();
        assert_eq!(mirror.back, model.back, "frame {i}");
    });
}

#[test]
fn clearing_from_a_wide_continuation_also_blanks_its_leading_half() {
    let mut s = screen(4, 1);
    queue!(
        s,
        Print("日"),
        cursor::MoveTo(1, 0),
        terminal::Clear(terminal::ClearType::UntilNewLine)
    )
    .unwrap();
    assert_eq!(row_text(&s, 0), "    ");
}

#[test]
fn invalidate_resends_every_cell() {
    let mut s = screen(3, 1);
    queue!(s, Print("abc")).unwrap();
    present(&mut s);
    s.invalidate();
    assert!(present(&mut s).contains("abc"));
}
