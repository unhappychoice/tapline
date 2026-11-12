use crate::{chart::Chart, game::Judgment};

#[derive(Debug, Clone, Copy)]
pub enum GaugeKind {
    Normal,
}

pub struct Gauge {
    pub kind: GaugeKind,
    value: f64,
    recovery: f64,
    compensation: f64,
}

impl Gauge {
    pub fn new(kind: GaugeKind, chart: &Chart) -> Self {
        let count: usize = chart
            .notes
            .iter()
            .map(|note| 1 + usize::from(note.end_ms.is_some()))
            .sum();
        let total = chart
            .total
            .filter(|v| v.is_finite() && *v > 0.0)
            .unwrap_or(260.0);
        Self {
            kind,
            value: 20.0,
            compensation: 0.0,
            recovery: if count == 0 {
                0.0
            } else {
                total / count as f64
            },
        }
    }

    pub fn apply(&mut self, judgment: Judgment) {
        let delta = match self.kind {
            GaugeKind::Normal => match judgment {
                Judgment::Perfect | Judgment::Great => self.recovery,
                Judgment::Good => self.recovery * 0.5,
                Judgment::Miss => -6.0,
            },
        };
        self.adjust(delta);
    }

    pub fn apply_mine(&mut self, damage: u32) {
        self.adjust(-f64::from(damage) / 2.0);
    }

    pub fn value(&self) -> f64 {
        self.value
    }

    pub fn is_clear(&self) -> bool {
        match self.kind {
            GaugeKind::Normal => self.value >= 80.0,
        }
    }

    pub fn label(&self) -> &'static str {
        match self.kind {
            GaugeKind::Normal => "NORMAL",
        }
    }
    fn adjust(&mut self, delta: f64) {
        let adjusted = delta - self.compensation;
        let next = self.value + adjusted;
        self.compensation = (next - self.value) - adjusted;
        self.value = next.clamp(0.0, 100.0);
        if self.value != next {
            self.compensation = 0.0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chart::Note;

    fn gauge(total: Option<f64>, count: usize) -> Gauge {
        Gauge::new(
            GaugeKind::Normal,
            &Chart {
                total,
                notes: vec![Note::default(); count],
                ..Chart::default()
            },
        )
    }

    #[test]
    fn total_and_note_count_control_recovery() {
        let mut low = gauge(Some(100.0), 100);
        let mut high = gauge(Some(200.0), 100);
        let mut dense = gauge(Some(200.0), 200);
        low.apply(Judgment::Perfect);
        high.apply(Judgment::Great);
        dense.apply(Judgment::Perfect);
        assert_eq!(low.value(), 21.0);
        assert_eq!(high.value(), 22.0);
        assert_eq!(dense.value(), 21.0);
    }

    #[test]
    fn missing_or_invalid_total_defaults_to_260() {
        [
            None,
            Some(0.0),
            Some(-1.0),
            Some(f64::NAN),
            Some(f64::INFINITY),
        ]
        .into_iter()
        .for_each(|total| {
            let mut g = gauge(total, 100);
            g.apply(Judgment::Perfect);
            assert_eq!(g.value(), 22.6);
        });
    }

    #[test]
    fn normal_starts_at_twenty_and_clears_at_eighty() {
        let mut g = gauge(Some(60.0), 1);
        assert_eq!(g.value(), 20.0);
        assert!(!g.is_clear());
        g.apply(Judgment::Perfect);
        assert_eq!(g.value(), 80.0);
        assert!(g.is_clear());
        g.apply(Judgment::Miss);
        assert_eq!(g.value(), 74.0);
        assert!(!g.is_clear());
    }

    #[test]
    fn fractional_recovery_clears_at_eighty() {
        [Judgment::Perfect, Judgment::Great, Judgment::Good]
            .into_iter()
            .for_each(|judgment| {
                let total = if judgment == Judgment::Good {
                    120.0
                } else {
                    60.0
                };
                let mut g = gauge(Some(total), 100);
                (0..99).for_each(|_| g.apply(judgment));
                assert!(!g.is_clear());
                g.apply(judgment);
                assert_eq!(g.value(), 80.0);
                assert!(g.is_clear());
                g.apply(Judgment::Miss);
                assert_eq!(g.value(), 74.0);
                assert!(!g.is_clear());
            });
    }

    #[test]
    fn recovery_below_eighty_does_not_clear() {
        let mut g = gauge(Some(59.99999999), 100);
        (0..100).for_each(|_| g.apply(Judgment::Perfect));
        assert!(g.value() < 80.0);
        assert!(!g.is_clear());
    }

    #[test]
    fn good_halves_recovery_and_value_is_clamped() {
        let mut g = gauge(Some(20.0), 1);
        g.apply(Judgment::Good);
        assert_eq!(g.value(), 30.0);
        (0..10).for_each(|_| g.apply(Judgment::Miss));
        assert_eq!(g.value(), 0.0);
        (0..10).for_each(|_| g.apply(Judgment::Perfect));
        assert_eq!(g.value(), 100.0);
    }

    #[test]
    fn mine_damage_scales_clamps_and_allows_recovery() {
        let mut g = gauge(Some(80.0), 1);
        g.apply(Judgment::Perfect);
        g.apply_mine(1);
        assert_eq!(g.value(), 99.5);
        g.apply_mine(36);
        assert_eq!(g.value(), 81.5);
        g.apply_mine(1295);
        assert_eq!(g.value(), 0.0);
        g.apply(Judgment::Great);
        assert_eq!(g.value(), 80.0);
    }

    #[test]
    fn empty_chart_never_recovers_or_divides_by_zero() {
        let mut g = gauge(None, 0);
        g.apply(Judgment::Perfect);
        assert_eq!(g.value(), 20.0);
    }

    #[test]
    fn long_note_counts_both_judgments_and_excludes_mines() {
        let chart = Chart {
            total: Some(60.0),
            notes: vec![Note {
                end_ms: Some(1000.0),
                ..Note::default()
            }],
            ..Chart::default()
        };
        let mut g = Gauge::new(GaugeKind::Normal, &chart);
        g.apply(Judgment::Perfect);
        assert_eq!(g.value(), 50.0);
        g.apply(Judgment::Perfect);
        assert_eq!(g.value(), 80.0);
    }
}
