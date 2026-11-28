/// A per-leaf value a [`SegmentTree`] can merge bottom-up and reset to an
/// identity element that never matches a query.
pub(crate) trait SegmentValue: Copy {
    const EMPTY: Self;
    fn merge(a: Self, b: Self) -> Self;
}

impl SegmentValue for (f64, f64) {
    const EMPTY: Self = (f64::INFINITY, f64::NEG_INFINITY);
    fn merge(a: Self, b: Self) -> Self {
        (a.0.min(b.0), a.1.max(b.1))
    }
}

/// Segment tree over per-leaf values, queried in insertion order. Internal
/// nodes hold the merge of their subtree so a top-down traversal can prune
/// whole subtrees a predicate rejects.
pub(crate) struct SegmentTree<T: SegmentValue> {
    bounds: Vec<T>,
    leaves: usize,
    #[cfg(test)]
    pub(crate) visits: std::cell::Cell<usize>,
}

impl<T: SegmentValue> SegmentTree<T> {
    pub(crate) fn new(data: impl ExactSizeIterator<Item = T>) -> Self {
        let leaves = data.len().next_power_of_two();
        let mut bounds = vec![T::EMPTY; leaves * 2];
        bounds[leaves..]
            .iter_mut()
            .zip(data)
            .for_each(|(slot, item)| {
                *slot = item;
            });
        for i in (1..leaves).rev() {
            bounds[i] = T::merge(bounds[i * 2], bounds[i * 2 + 1]);
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
        self.bounds[node] = T::EMPTY;
        while node > 1 {
            node /= 2;
            self.bounds[node] = T::merge(self.bounds[node * 2], self.bounds[node * 2 + 1]);
        }
    }

    /// Top-down traversal pruning subtrees for which `prune` returns true,
    /// returning the leaf indices that survive.
    pub(crate) fn traverse(&self, prune: impl Fn(T) -> bool) -> Vec<usize> {
        let mut pending = vec![1];
        let mut matching = Vec::new();
        while let Some(node) = pending.pop() {
            #[cfg(test)]
            self.visits.set(self.visits.get() + 1);
            if prune(self.bounds[node]) {
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

/// Bounds over contiguous groups of intervals.
pub(crate) type IntervalIndex = SegmentTree<(f64, f64)>;

impl IntervalIndex {
    pub(crate) fn matching(&self, overlaps: impl Fn(f64, f64) -> bool) -> Vec<usize> {
        self.traverse(|(min, max)| min > max || !overlaps(min, max))
    }
}
