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
    touched: Vec<usize>,
    queued: Vec<bool>, // Membership in touched, indexed by cell.
    redraw: bool,
    cursor: (u16, u16),
    style: Style,
    pending: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq)]
struct Cell {
    ch: char,
    marks: String,
    style: Style,
}

const BLANK: Cell = Cell {
    ch: ' ',
    marks: String::new(),
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
            touched: Vec::new(),
            queued: Vec::new(),
            redraw: false,
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
        self.front = vec![BLANK; len];
        self.back = vec![BLANK; len];
        self.touched.clear();
        self.queued = vec![false; len];
        self.invalidate();
    }

    /// Forgets what the terminal shows, e.g. after it was cleared directly.
    pub fn invalidate(&mut self) {
        self.redraw = true;
    }

    /// Writes the cells that differ from the last presented frame.
    pub fn present(&mut self, out: &mut impl Write) -> io::Result<()> {
        let diff = self.build_diff();
        if diff.is_empty() {
            return Ok(());
        }
        let result = out.write_all(diff.as_bytes()).and_then(|()| out.flush());
        if result.is_err() {
            self.invalidate();
        }
        result
    }

    fn build_diff(&mut self) -> String {
        let mut buf = String::new();
        let mut pen = Pen::default();
        let redraw = std::mem::take(&mut self.redraw);
        let full_len = if redraw { self.back.len() } else { 0 };
        // Emit in terminal order so adjacent cells share cursor moves.
        let touched = if redraw {
            &[][..]
        } else {
            self.touched.sort_unstable();
            &self.touched
        };
        for i in (0..full_len).chain(touched.iter().copied()) {
            let cell = &self.back[i];
            let front = &mut self.front[i];
            if redraw || cell != front {
                if cell.ch != CONTINUATION {
                    let cols = usize::from(self.cols);
                    pen.draw(((i % cols) as u16, (i / cols) as u16), cell, &mut buf);
                }
                front.clone_from(cell);
            }
        }
        for i in self.touched.drain(..) {
            self.queued[i] = false;
        }
        if !buf.is_empty() {
            buf.push_str("\x1b[0m\x1b[?2026l");
        }
        buf
    }

    fn touch(&mut self, i: usize) {
        if !self.redraw && !self.queued[i] {
            self.queued[i] = true;
            self.touched.push(i);
        }
    }

    fn put(&mut self, ch: char) {
        let Some(width) = ch.width() else {
            return;
        };
        let width = width as u16;
        let (x, y) = self.cursor;
        if width == 0 {
            self.append_mark(x, y, ch);
            return;
        }
        if y >= self.rows || width > self.cols.saturating_sub(x) {
            self.cursor.0 = x.saturating_add(width);
            return;
        }
        self.detach_wide(x, y);
        self.detach_wide(x + width - 1, y);
        let style = self.style;
        self.set(
            x,
            y,
            Cell {
                ch,
                marks: String::new(),
                style,
            },
        );
        if width == 2 {
            self.set(
                x + 1,
                y,
                Cell {
                    ch: CONTINUATION,
                    marks: String::new(),
                    style,
                },
            );
        }
        self.cursor.0 = x + width;
    }

    fn append_mark(&mut self, x: u16, y: u16, mark: char) {
        let Some(i) = x.checked_sub(1).and_then(|prev| self.index(prev, y)) else {
            return;
        };
        let base = i - usize::from(self.back[i].ch == CONTINUATION);
        self.back[base].marks.push(mark);
        self.touch(base);
    }

    /// Blanks the other half of a wide character about to be overwritten.
    fn detach_wide(&mut self, x: u16, y: u16) {
        let partner = match self.char_at(x, y) {
            CONTINUATION => x.checked_sub(1),
            _ if self.char_at(x + 1, y) == CONTINUATION => Some(x + 1),
            _ => None,
        };
        if let Some(px) = partner {
            self.set(px, y, BLANK);
        }
    }

    fn char_at(&self, x: u16, y: u16) -> char {
        self.index(x, y).map_or(BLANK.ch, |i| self.back[i].ch)
    }

    fn set(&mut self, x: u16, y: u16, cell: Cell) {
        if let Some(i) = self.index(x, y) {
            self.back[i] = cell;
            self.touch(i);
        }
    }

    fn index(&self, x: u16, y: u16) -> Option<usize> {
        (x < self.cols && y < self.rows).then(|| y as usize * self.cols as usize + x as usize)
    }

    fn clear_row(&mut self, y: u16, from: u16) {
        self.detach_wide(from, y);
        if let Some(start) = self.index(from, y) {
            self.clear_cells(start..start + usize::from(self.cols - from));
        }
    }

    fn clear_all(&mut self) {
        self.clear_cells(0..self.back.len());
    }

    fn clear_cells(&mut self, cells: std::ops::Range<usize>) {
        for i in cells {
            if self.back[i] != BLANK {
                self.back[i] = BLANK;
                self.touch(i);
            }
        }
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
    fn draw(&mut self, (x, y): (u16, u16), cell: &Cell, buf: &mut String) {
        if self.cursor.is_none() {
            buf.push_str("\x1b[?2026h");
        }
        if self.cursor != Some((x, y)) {
            buf.push_str(&format!("\x1b[{};{}H", y + 1, x + 1));
        }
        if self.style != Some(cell.style) {
            buf.push_str(&cell.style.to_sgr());
            self.style = Some(cell.style);
        }
        buf.push(cell.ch);
        buf.push_str(&cell.marks);
        self.cursor = Some((x + cell.ch.width().unwrap_or(1) as u16, y));
    }
}

#[cfg(test)]
mod tests;
