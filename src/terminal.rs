use anyhow::Result;
use crossterm::{
    cursor,
    event::{KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags},
    execute, terminal,
};
use std::io::{self, Write};

pub fn with_alt_screen<W: Write, F: FnOnce(&mut W) -> Result<()>>(out: &mut W, f: F) -> Result<()> {
    terminal::enable_raw_mode()?;
    let mut raw_mode = RawModeGuard(true);
    execute!(out, terminal::EnterAlternateScreen, cursor::Hide)?;
    // Best-effort: ask the terminal to report key up events so long-note
    // releases can be judged. Terminals that don't understand the kitty
    // keyboard protocol will ignore this — the runtime falls back to
    // auto-releasing LNs when they cross the miss window.
    let pushed = execute!(
        out,
        PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::REPORT_EVENT_TYPES)
    )
    .is_ok();
    let result = f(out);
    if pushed {
        let _ = execute!(out, PopKeyboardEnhancementFlags);
    }
    execute!(out, cursor::Show, terminal::LeaveAlternateScreen)?;
    raw_mode.restore()?;
    result
}

pub fn show_load_error(out: &mut impl Write, error: &anyhow::Error) -> Result<()> {
    draw_load_error(out, error)?;
    loop {
        if let crossterm::event::Event::Key(key) = crossterm::event::read()? {
            if key.kind == crossterm::event::KeyEventKind::Press {
                return Ok(());
            }
        }
    }
}

fn draw_load_error(out: &mut impl Write, error: &anyhow::Error) -> Result<()> {
    let message: String = format!("{error:#}")
        .chars()
        .filter(|c| !c.is_control())
        .collect();
    execute!(
        out,
        terminal::Clear(terminal::ClearType::All),
        cursor::MoveTo(0, 0),
        crossterm::style::Print(format!(
            "Failed to load chart: {message}\r\n\r\nPress any key to return to the selector."
        ))
    )?;
    Ok(())
}

struct RawModeGuard(bool);

impl RawModeGuard {
    fn restore(&mut self) -> io::Result<()> {
        terminal::disable_raw_mode()?;
        self.0 = false;
        Ok(())
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        if self.0 {
            let _ = terminal::disable_raw_mode();
        }
    }
}

#[cfg(all(test, unix))]
#[path = "terminal_tests.rs"]
mod tests;

#[cfg(test)]
#[test]
fn load_error_screen_shows_error_and_return_hint() {
    let mut out = Vec::new();
    draw_load_error(&mut out, &anyhow::anyhow!("Unequal lengths\x07")).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("Failed to load chart: Unequal lengths"));
    assert!(text.contains("Press any key to return to the selector."));
    assert!(!text.contains('\x07'));
}
