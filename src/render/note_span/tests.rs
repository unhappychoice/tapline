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
    let bar = Bar {
        x: 4,
        width: 4,
        span: (10.5, 11.5),
        color: Color::Blue,
    };
    draw_bars(&mut out, &[bar]).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert_eq!(text.matches("▄").count(), 8);
    assert!(text.contains("\x1b[7m"));
}

fn bar(top: f64) -> Bar {
    Bar {
        x: 2,
        width: 4,
        span: (top, top + 1.0),
        color: Color::Blue,
    }
}

#[test]
fn adjacent_notes_sharing_a_cell_fill_it_completely() {
    let cells = merged_cells(&[bar(10.25), bar(11.25)]);
    assert_eq!(cells[&(2, 11)].0, Cell::Lower(8));
    assert_eq!(cells[&(2, 10)].0, Cell::Lower(6));
    assert_eq!(cells[&(2, 12)].0, Cell::Upper(2));
}

#[test]
fn overlapping_partials_keep_the_larger_fill_regardless_of_order() {
    let a = (Cell::Lower(3), Color::Blue);
    let b = (Cell::Lower(6), Color::Red);
    assert_eq!(merge(a, b), b);
    assert_eq!(merge(b, a), b);
}

#[test]
fn variable_width_fractional_bars_merge_only_overlapping_columns() {
    let narrow = bar(10.25);
    let wide = Bar {
        x: 1,
        width: 6,
        span: (11.25, 12.25),
        color: Color::Red,
    };
    let cells = merged_cells(&[narrow, wide]);
    (1..7).for_each(|x| {
        assert_eq!(cells[&(x, 12)], (Cell::Upper(2), Color::Red));
        assert_eq!(
            cells[&(x, 11)].0,
            if (2..6).contains(&x) {
                Cell::Lower(8)
            } else {
                Cell::Lower(6)
            }
        );
    });
    assert!(!cells.keys().any(|(x, _)| *x == 0 || *x == 7));
}

#[test]
fn output_uses_each_bars_width_for_full_and_fractional_rows() {
    for width in [4, 6] {
        for (span, glyph, rows) in [((10.0, 11.0), "█", 1), ((10.5, 11.5), "▄", 2)] {
            let mut out = Vec::new();
            draw_bars(
                &mut out,
                &[Bar {
                    x: 2,
                    width,
                    span,
                    color: Color::Red,
                }],
            )
            .unwrap();
            let text = String::from_utf8(out).unwrap();
            assert_eq!(text.matches(glyph).count(), width as usize * rows);
        }
    }
}
