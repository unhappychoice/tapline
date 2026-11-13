use crossterm::{
    cursor, queue,
    style::{Attribute, Color, Print, ResetColor, SetAttribute, SetForegroundColor},
};
use std::io::Write;

const EIGHTHS: u8 = 8;
const LOWER_BLOCKS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
pub(super) const NOTE_WIDTH: usize = 4;

/// Draws a vertical bar covering rows `[top, bottom)` (fractional screen rows)
/// with 1/8-row precision, so notes glide instead of jumping whole cells.
pub(super) fn draw_span(
    out: &mut impl Write,
    (top, bottom): (f64, f64),
    x: u16,
    color: Color,
) -> anyhow::Result<()> {
    span_cells(top, bottom)
        .into_iter()
        .try_for_each(|(row, cell)| draw_cell(out, x, row, cell, color))
}

/// The part of one terminal cell a span covers, in eighths of a row.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Cell {
    Lower(u8),
    Upper(u8),
}

pub(super) fn span_cells(top: f64, bottom: f64) -> Vec<(u16, Cell)> {
    let to_eighths = |row: f64| (row.max(0.0) * EIGHTHS as f64).round() as u32;
    let (start, end) = (to_eighths(top), to_eighths(bottom));
    if end <= start {
        return Vec::new();
    }
    (start / 8..=(end - 1) / 8)
        .map(|row| (row as u16, cell_for(row * 8, start, end)))
        .collect()
}

fn cell_for(cell_top: u32, start: u32, end: u32) -> Cell {
    let (from, to) = (start.max(cell_top), end.min(cell_top + 8));
    let filled = (to - from) as u8;
    if from == cell_top && to < cell_top + 8 {
        Cell::Upper(filled)
    } else {
        Cell::Lower(filled)
    }
}

fn draw_cell(
    out: &mut impl Write,
    x: u16,
    row: u16,
    cell: Cell,
    color: Color,
) -> anyhow::Result<()> {
    let (glyph, reverse) = cell_glyph(cell);
    let text: String = std::iter::repeat_n(glyph, NOTE_WIDTH).collect();
    queue!(out, cursor::MoveTo(x, row), SetForegroundColor(color))?;
    if reverse {
        queue!(out, SetAttribute(Attribute::Reverse))?;
    }
    queue!(out, Print(text), SetAttribute(Attribute::Reset), ResetColor)?;
    Ok(())
}

/// Upper fills have no glyphs of their own, so they are drawn as the
/// complementary lower block in reverse video.
fn cell_glyph(cell: Cell) -> (char, bool) {
    match cell {
        Cell::Lower(n) | Cell::Upper(n) if n >= EIGHTHS => (LOWER_BLOCKS[7], false),
        Cell::Lower(n) => (LOWER_BLOCKS[n.max(1) as usize - 1], false),
        Cell::Upper(n) => (LOWER_BLOCKS[(EIGHTHS - n) as usize - 1], true),
    }
}

#[cfg(test)]
mod tests;
