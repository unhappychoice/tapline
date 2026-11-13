use super::*;

#[test]
fn row_aligned_note_fills_a_single_cell() {
    assert_eq!(span_cells(10.0, 11.0), [(10, Cell::Lower(8))]);
}

#[test]
fn note_between_rows_splits_into_lower_and_upper_parts() {
    assert_eq!(
        span_cells(10.25, 11.25),
        [(10, Cell::Lower(6)), (11, Cell::Upper(2))]
    );
}

#[test]
fn long_span_has_partial_ends_and_full_middle() {
    assert_eq!(
        span_cells(3.5, 6.375),
        [
            (3, Cell::Lower(4)),
            (4, Cell::Lower(8)),
            (5, Cell::Lower(8)),
            (6, Cell::Upper(3)),
        ]
    );
}

#[test]
fn empty_or_inverted_span_draws_nothing() {
    assert!(span_cells(5.0, 5.0).is_empty());
    assert!(span_cells(6.0, 5.0).is_empty());
}

#[test]
fn every_position_covers_exactly_one_row_of_eighths() {
    (0..64).map(|i| 10.0 + i as f64 / 64.0).for_each(|top| {
        let total: u32 = span_cells(top, top + 1.0)
            .iter()
            .map(|(_, cell)| match cell {
                Cell::Lower(n) | Cell::Upper(n) => *n as u32,
            })
            .sum();
        assert_eq!(total, 8, "top={top}");
    });
}

#[test]
fn glyphs_use_lower_blocks_and_reverse_video_for_upper_fills() {
    assert_eq!(cell_glyph(Cell::Lower(8)), ('█', false));
    assert_eq!(cell_glyph(Cell::Lower(3)), ('▃', false));
    assert_eq!(cell_glyph(Cell::Upper(2)), ('▆', true));
    assert_eq!(cell_glyph(Cell::Upper(8)), ('█', false));
}

#[test]
fn upper_cells_are_written_in_reverse_video() {
    let mut out = Vec::new();
    draw_span(&mut out, (10.5, 11.5), 4, Color::Blue).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("▄▄▄▄"));
    assert!(text.contains("\x1b[7m"));
}
