use crate::chart::Mine;
use crate::scroll::{ScrollFrame, ScrollMap};

/// Immutable bounds over mines in their original drawing order.
pub struct MineIndex {
    bounds: Vec<(f64, f64, f64)>,
    leaves: usize,
    #[cfg(test)]
    visits: std::cell::Cell<usize>,
}

impl MineIndex {
    pub fn new(mines: &[Mine], scroll: &ScrollMap) -> Self {
        let leaves = mines.len().next_power_of_two();
        let mut bounds = vec![(f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY); leaves * 2];
        mines.iter().enumerate().for_each(|(i, mine)| {
            let coordinate = scroll.coordinate(mine.time_ms);
            bounds[leaves + i] = (coordinate, coordinate, mine.time_ms);
        });
        for i in (1..leaves).rev() {
            let (a, b) = (bounds[i * 2], bounds[i * 2 + 1]);
            bounds[i] = (a.0.min(b.0), a.1.max(b.1), a.2.max(b.2));
        }
        Self {
            bounds,
            leaves,
            #[cfg(test)]
            visits: std::cell::Cell::new(0),
        }
    }

    pub fn visible<'a>(
        &self,
        mines: &'a [Mine],
        frame: &ScrollFrame<'_>,
        now: f64,
        approach: f64,
    ) -> Vec<&'a Mine> {
        let mut pending = vec![1];
        let mut visible = Vec::new();
        while let Some(node) = pending.pop() {
            #[cfg(test)]
            self.visits.set(self.visits.get() + 1);
            let (min, max, latest) = self.bounds[node];
            if latest < now - 60.0 || latest == f64::NEG_INFINITY {
                continue;
            }
            let (a, b) = (frame.transform(min), frame.transform(max));
            if a.min(b) > approach || a.max(b) < -60.0 {
                continue;
            }
            if node < self.leaves {
                pending.extend([node * 2 + 1, node * 2]);
            } else {
                let mine = &mines[node - self.leaves];
                if !mine.exploded && (-60.0..=approach).contains(&a) {
                    visible.push(mine);
                }
            }
        }
        visible
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chart::{Chart, ScrollEvent};

    fn mine(time_ms: f64) -> Mine {
        Mine {
            time_ms,
            lane: 0,
            damage: 1,
            exploded: false,
        }
    }

    #[test]
    fn indexed_selection_matches_full_scan_with_scroll_effects_and_rewinds() {
        for rate in [1.0, 0.5, 0.0, -1.0] {
            for speed in [1.0, 0.5, 0.0, -1.0] {
                let chart = Chart {
                    scroll: vec![
                        ScrollEvent {
                            time_ms: 0.0,
                            multiplier: rate,
                        },
                        ScrollEvent {
                            time_ms: 5000.0,
                            multiplier: -rate,
                        },
                    ],
                    speed: vec![ScrollEvent {
                        time_ms: 0.0,
                        multiplier: speed,
                    }],
                    ..Chart::default()
                };
                let scroll = ScrollMap::new(&chart.scroll);
                // Include unsorted input, duplicate timestamps and exact clipping boundaries.
                let mut mines: Vec<_> = [
                    10000.0, -60.001, -60.0, 0.0, 0.0, 1000.0, 1000.001, 1800.0, 5000.0, 9000.0,
                ]
                .into_iter()
                .map(mine)
                .collect();
                mines[3].exploded = true;
                let index = MineIndex::new(&mines, &scroll);
                for now in [0.0, 6000.0, 10000.0, 0.0] {
                    let frame = scroll.frame(&chart, now);
                    let expected: Vec<_> = mines
                        .iter()
                        .filter(|mine| {
                            !mine.exploded
                                && mine.time_ms >= now - 60.0
                                && (-60.0..=1000.0).contains(&frame.distance(mine.time_ms))
                        })
                        .map(|mine| mine as *const Mine)
                        .collect();
                    let actual: Vec<_> = index
                        .visible(&mines, &frame, now, 1000.0)
                        .into_iter()
                        .map(|mine| mine as *const Mine)
                        .collect();
                    assert_eq!(actual, expected, "rate={rate}, speed={speed}, now={now}");
                }
            }
        }
    }

    #[test]
    fn distant_mines_only_add_logarithmic_index_visits() {
        for count in [100usize, 1000, 10000] {
            let mines: Vec<_> = (0..count).map(|i| mine(i as f64 * 10000.0)).collect();
            let chart = Chart::default();
            let scroll = ScrollMap::new(&[]);
            let index = MineIndex::new(&mines, &scroll);
            let frame = scroll.frame(&chart, 10000.0);
            assert_eq!(index.visible(&mines, &frame, 10000.0, 1000.0).len(), 1);
            assert!(index.visits.get() <= 4 * (count.ilog2() as usize + 2));
        }
    }

    #[test]
    fn empty_and_expired_frozen_mines_are_pruned() {
        let chart = Chart::default();
        let scroll = ScrollMap::new(&[ScrollEvent {
            time_ms: 0.0,
            multiplier: 0.0,
        }]);
        let frame = scroll.frame(&chart, 100000.0);
        for count in [0, 10000] {
            let mines = vec![mine(0.0); count];
            let index = MineIndex::new(&mines, &scroll);
            assert!(index.visible(&mines, &frame, 100000.0, 1000.0).is_empty());
            assert_eq!(index.visits.get(), 1);
        }
    }
}
