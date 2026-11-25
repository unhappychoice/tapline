use crate::chart::Mine;
use crate::interval_index::{SegmentTree, SegmentValue};
use crate::scroll::{ScrollFrame, ScrollMap};

impl SegmentValue for (f64, f64, f64) {
    const EMPTY: Self = (f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    fn merge(a: Self, b: Self) -> Self {
        (a.0.min(b.0), a.1.max(b.1), a.2.max(b.2))
    }
}

/// Immutable bounds over mines in their original drawing order. Each leaf
/// carries the mine's scroll coordinate (repeated as min/max, since a mine
/// is a point) and its time, so stale mines can be pruned without a scroll
/// frame.
pub struct MineIndex {
    tree: SegmentTree<(f64, f64, f64)>,
}

impl MineIndex {
    pub fn new(mines: &[Mine], scroll: &ScrollMap) -> Self {
        let bounds = mines.iter().map(|mine| {
            let coordinate = scroll.coordinate(mine.time_ms);
            (coordinate, coordinate, mine.time_ms)
        });
        Self {
            tree: SegmentTree::new(bounds),
        }
    }

    pub fn visible<'a>(
        &self,
        mines: &'a [Mine],
        frame: &ScrollFrame<'_>,
        now: f64,
        approach: f64,
    ) -> Vec<&'a Mine> {
        self.tree
            .traverse(|(min, max, latest)| {
                if latest < now - 60.0 || latest == f64::NEG_INFINITY {
                    return true;
                }
                let (a, b) = (frame.transform(min), frame.transform(max));
                a.min(b) > approach || a.max(b) < -60.0
            })
            .into_iter()
            .map(|i| &mines[i])
            .filter(|mine| !mine.exploded)
            .collect()
    }

    #[cfg(test)]
    fn visits(&self) -> usize {
        self.tree.visits.get()
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
            assert!(index.visits() <= 4 * (count.ilog2() as usize + 2));
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
            assert_eq!(index.visits(), 1);
        }
    }
}
