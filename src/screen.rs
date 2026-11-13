use std::io::{self, Write};
use style::Style;
use unicode_width::UnicodeWidthChar;

mod ansi;
mod style;

/// Off-screen terminal model. Drawing code writes ANSI output into it as if it
/// were the terminal; `present` then sends only the cells that changed since
/// the previous frame, keeping per-frame output small.
pub struct Screen {
    cols: u16,
    rows: u16,
    front: Vec<Cell>,
    back: Vec<Cell>,
    cursor: (u16, u16),
    style: Style,
    pending: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Cell {
    ch: char,
    style: Style,
}

const BLANK: Cell = Cell {
    ch: ' ',
    style: Style::DEFAULT,
};
/// Never matches drawn content, forcing a cell to be re-sent.
const UNKNOWN: Cell = Cell {
    ch: '\u{FFFF}',
    style: Style::DEFAULT,
};
/// Right half of a double-width character.
const CONTINUATION: char = '\0';

impl Screen {
    pub fn new() -> Self {
        Self {
            cols: 0,
            rows: 0,
            front: Vec::new(),
            back: Vec::new(),
            cursor: (0, 0),
            style: Style::DEFAULT,
            pending: Vec::new(),
        }
    }

    /// Matches the model to the terminal size; a change forces a full redraw.
    pub fn resize(&mut self, cols: u16, rows: u16) {
        if (cols, rows) == (self.cols, self.rows) {
            return;
        }
        let len = cols as usize * rows as usize;
        (self.cols, self.rows) = (cols, rows);
        self.front = vec![UNKNOWN; len];
        self.back = vec![BLANK; len];
    }

    /// Writes the cells that differ from the last presented frame.
    pub fn present(&mut self, out: &mut impl Write) -> io::Result<()> {
        let mut buf = String::from("\x1b[?2026h");
        let mut pen = Pen::default();
        (0..self.back.len()).for_each(|i| self.emit_cell(i, &mut pen, &mut buf));
        buf.push_str("\x1b[0m\x1b[?2026l");
        self.front.clone_from(&self.back);
        out.write_all(buf.as_bytes())?;
        out.flush()
    }

    fn emit_cell(&self, i: usize, pen: &mut Pen, buf: &mut String) {
        let cell = self.back[i];
        if cell == self.front[i] || cell.ch == CONTINUATION {
            return;
        }
        let pos = (
            (i % self.cols as usize) as u16,
            (i / self.cols as usize) as u16,
        );
        pen.draw(pos, cell, buf);
    }

    fn put(&mut self, ch: char) {
        let width = ch.width().unwrap_or(0) as u16;
        let (x, y) = self.cursor;
        if width == 0 || y >= self.rows || x + width > self.cols {
            self.cursor.0 = x.saturating_add(width);
            return;
        }
        self.detach_wide(x, y);
        self.detach_wide(x + width - 1, y);
        let style = self.style;
        self.set(x, y, Cell { ch, style });
        if width == 2 {
            self.set(
                x + 1,
                y,
                Cell {
                    ch: CONTINUATION,
                    style,
                },
            );
        }
        self.cursor.0 = x + width;
    }

    /// Blanks the other half of a wide character about to be overwritten.
    fn detach_wide(&mut self, x: u16, y: u16) {
        let partner = match self.cell(x, y).ch {
            CONTINUATION => x.checked_sub(1),
            _ if self.cell(x + 1, y).ch == CONTINUATION => Some(x + 1),
            _ => None,
        };
        if let Some(px) = partner {
            self.set(px, y, BLANK);
        }
    }

    fn cell(&self, x: u16, y: u16) -> Cell {
        self.index(x, y).map_or(BLANK, |i| self.back[i])
    }

    fn set(&mut self, x: u16, y: u16, cell: Cell) {
        if let Some(i) = self.index(x, y) {
            self.back[i] = cell;
        }
    }

    fn index(&self, x: u16, y: u16) -> Option<usize> {
        (x < self.cols && y < self.rows).then(|| y as usize * self.cols as usize + x as usize)
    }

    fn clear_row(&mut self, y: u16, from: u16) {
        (from..self.cols).for_each(|x| self.set(x, y, BLANK));
    }

    fn clear_all(&mut self) {
        self.back.fill(BLANK);
    }
}

impl Default for Screen {
    fn default() -> Self {
        Self::new()
    }
}

impl Write for Screen {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.pending.extend_from_slice(bytes);
        let consumed = ansi::feed(self);
        self.pending.drain(..consumed);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Tracks the real terminal's cursor and style while emitting a diff.
#[derive(Default)]
struct Pen {
    cursor: Option<(u16, u16)>,
    style: Option<Style>,
}

impl Pen {
    fn draw(&mut self, (x, y): (u16, u16), cell: Cell, buf: &mut String) {
        if self.cursor != Some((x, y)) {
            buf.push_str(&format!("\x1b[{};{}H", y + 1, x + 1));
        }
        if self.style != Some(cell.style) {
            buf.push_str(&cell.style.to_sgr());
            self.style = Some(cell.style);
        }
        buf.push(cell.ch);
        self.cursor = Some((x + cell.ch.width().unwrap_or(1) as u16, y));
    }
}

#[cfg(test)]
mod tests;
