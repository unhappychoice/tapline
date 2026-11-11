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
