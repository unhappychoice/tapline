const EMPTY: (f64, f64) = (f64::INFINITY, f64::NEG_INFINITY);

/// Bounds over contiguous groups of intervals, queried in insertion order.
pub(crate) struct IntervalIndex {
    bounds: Vec<(f64, f64)>,
    leaves: usize,
    #[cfg(test)]
    pub(crate) visits: std::cell::Cell<usize>,
}

impl IntervalIndex {
    pub(crate) fn new(intervals: impl ExactSizeIterator<Item = (f64, f64)>) -> Self {
        let leaves = intervals.len().next_power_of_two();
        let mut bounds = vec![EMPTY; leaves * 2];
        bounds[leaves..]
            .iter_mut()
            .zip(intervals)
            .for_each(|(slot, interval)| {
                *slot = interval;
            });
        for i in (1..leaves).rev() {
            bounds[i] = merge(bounds[i * 2], bounds[i * 2 + 1]);
        }
        Self {
            bounds,
            leaves,
            #[cfg(test)]
            visits: std::cell::Cell::new(0),
        }
    }

    pub(crate) fn remove(&mut self, index: usize) {
        let mut node = self.leaves + index;
        self.bounds[node] = EMPTY;
        while node > 1 {
            node /= 2;
            self.bounds[node] = merge(self.bounds[node * 2], self.bounds[node * 2 + 1]);
        }
    }

    pub(crate) fn matching(&self, overlaps: impl Fn(f64, f64) -> bool) -> Vec<usize> {
        let mut pending = vec![1];
        let mut matching = Vec::new();
        while let Some(node) = pending.pop() {
            #[cfg(test)]
            self.visits.set(self.visits.get() + 1);
            let (min, max) = self.bounds[node];
            if min > max || !overlaps(min, max) {
                continue;
            }
            if node < self.leaves {
                pending.extend([node * 2 + 1, node * 2]);
            } else {
                matching.push(node - self.leaves);
            }
        }
        matching
    }
}

fn merge(a: (f64, f64), b: (f64, f64)) -> (f64, f64) {
    (a.0.min(b.0), a.1.max(b.1))
}
