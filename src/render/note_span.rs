use crossterm::{
    cursor, queue,
    style::{Attribute, Color, Print, ResetColor, SetAttribute, SetForegroundColor},
};
use std::collections::BTreeMap;
use std::io::Write;

const EIGHTHS: u8 = 8;
const LOWER_BLOCKS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/// A vertical bar covering rows `[top, bottom)` (fractional screen rows).
pub(super) struct Bar {
    pub(super) x: u16,
    pub(super) width: u16,
    pub(super) span: (f64, f64),
    pub(super) color: Color,
}

/// Draws bars with 1/8-row precision, so notes glide instead of jumping whole
/// cells. Bars sharing a cell are merged so neither part is overwritten.
pub(super) fn draw_bars(out: &mut impl Write, bars: &[Bar]) -> anyhow::Result<()> {
    merged_cells(bars)
        .into_iter()
        .try_for_each(|((x, row), (cell, color))| draw_cell(out, x, row, cell, color))
}

type Painted = (Cell, Color);

fn merged_cells(bars: &[Bar]) -> BTreeMap<(u16, u16), Painted> {
    bars.iter()
        .flat_map(|bar| {
            span_cells(bar.span.0, bar.span.1)
                .into_iter()
                .flat_map(move |(row, cell)| {
                    (bar.x..bar.x + bar.width).map(move |x| ((x, row), (cell, bar.color)))
                })
        })
        .fold(BTreeMap::new(), |mut cells, (pos, painted)| {
            cells
                .entry(pos)
                .and_modify(|current| *current = merge(*current, painted))
                .or_insert(painted);
            cells
        })
}

/// A top part and a bottom part that meet fill the cell; otherwise the cell
/// can show only one partial fill, so the larger one wins.
fn merge(a: Painted, b: Painted) -> Painted {
    match (a.0, b.0) {
        (Cell::Upper(x), Cell::Lower(y)) | (Cell::Lower(y), Cell::Upper(x)) if x + y >= EIGHTHS => {
            (Cell::Lower(EIGHTHS), a.1)
        }
        _ if filled(b.0) > filled(a.0) => b,
        _ => a,
    }
}

fn filled(cell: Cell) -> u8 {
    match cell {
        Cell::Lower(n) | Cell::Upper(n) => n,
    }
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
    queue!(out, cursor::MoveTo(x, row), SetForegroundColor(color))?;
    if reverse {
        queue!(out, SetAttribute(Attribute::Reverse))?;
    }
    queue!(
        out,
        Print(glyph),
        SetAttribute(Attribute::Reset),
        ResetColor
    )?;
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
