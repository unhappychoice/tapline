use crate::chart::Note;
use crate::scroll::{ScrollFrame, ScrollMap};

/// Scroll-coordinate bounds over contiguous groups of notes in chart order.
pub struct NoteIndex {
    bounds: Vec<(f64, f64)>,
    leaves: usize,
    #[cfg(test)]
    pub(crate) visits: std::cell::Cell<usize>,
}

impl NoteIndex {
    pub fn new(notes: &[Note], scroll: &ScrollMap) -> Self {
        let leaves = notes.len().next_power_of_two();
        let mut bounds = vec![(f64::INFINITY, f64::NEG_INFINITY); leaves * 2];
        for (i, note) in notes.iter().enumerate() {
            bounds[leaves + i] = match note.end_ms {
                Some(end) => scroll.range(note.time_ms, end),
                None => {
                    let point = scroll.coordinate(note.time_ms);
                    (point, point)
                }
            };
        }
        for i in (1..leaves).rev() {
            let (a, b) = (bounds[i * 2], bounds[i * 2 + 1]);
            bounds[i] = (a.0.min(b.0), a.1.max(b.1));
        }
        Self {
            bounds,
            leaves,
            #[cfg(test)]
            visits: std::cell::Cell::new(0),
        }
    }

    pub fn note_overlaps(&self, index: usize, frame: &ScrollFrame<'_>, approach: f64) -> bool {
        self.overlaps(self.leaves + index, frame, approach)
    }

    pub fn visible(&self, frame: &ScrollFrame<'_>, start: usize, approach: f64) -> Vec<usize> {
        let mut pending = vec![(1, 0, self.leaves)];
        let mut visible = Vec::new();
        while let Some((node, first, end)) = pending.pop() {
            #[cfg(test)]
            self.visits.set(self.visits.get() + 1);
            if end <= start || !self.overlaps(node, frame, approach) {
                continue;
            }
            if node < self.leaves {
                let mid = (first + end) / 2;
                pending.extend([(node * 2 + 1, mid, end), (node * 2, first, mid)]);
            } else {
                visible.push(first);
            }
        }
        visible
    }

    fn overlaps(&self, node: usize, frame: &ScrollFrame<'_>, approach: f64) -> bool {
        let (min, max) = self.bounds[node];
        let (a, b) = (frame.transform(min), frame.transform(max));
        min <= max && !(a > approach && b > approach || a < -60.0 && b < -60.0)
    }
}
