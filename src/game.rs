use crate::chart::Chart;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Judgment {
    Perfect,
    Great,
    Good,
    Miss,
}

impl Judgment {
    pub fn points(self) -> u32 {
        match self {
            Judgment::Perfect => 300,
            Judgment::Great   => 200,
            Judgment::Good    => 100,
            Judgment::Miss    => 0,
        }
    }
}

pub struct FlashState {
    pub last_judgment: Option<Judgment>,
    pub last_lane_hit: Vec<f64>,
    pub last_judgment_at: f64,
}

pub struct Game {
    pub chart: Chart,
    pub score: u32,
    pub combo: u32,
    pub max_combo: u32,
    pub perfect: u32,
    pub great: u32,
    pub good: u32,
    pub miss: u32,
    pub flash: FlashState,
}

pub const WINDOW_PERFECT: f64 = 45.0;
pub const WINDOW_GREAT:   f64 = 90.0;
pub const WINDOW_GOOD:    f64 = 140.0;
pub const MISS_AFTER:     f64 = 160.0;

impl Game {
    pub fn new(chart: Chart) -> Self {
        let lane_count = chart.lane_count;
        Self {
            chart,
            score: 0, combo: 0, max_combo: 0,
            perfect: 0, great: 0, good: 0, miss: 0,
            flash: FlashState {
                last_judgment: None,
                last_lane_hit: vec![-9999.0; lane_count],
                last_judgment_at: -9999.0,
            },
        }
    }

    pub fn hit(&mut self, lane: usize, now_ms: f64) -> Option<u32> {
        if lane >= self.flash.last_lane_hit.len() { return None; }
        let mut best: Option<(usize, f64)> = None;
        for (i, n) in self.chart.notes.iter().enumerate() {
            if n.hit || n.lane != lane { continue; }
            let dt = (n.time_ms - now_ms).abs();
            if dt > WINDOW_GOOD { continue; }
            if best.map_or(true, |(_, b)| dt < b) {
                best = Some((i, dt));
            }
        }
        self.flash.last_lane_hit[lane] = now_ms;
        if let Some((i, dt)) = best {
            self.chart.notes[i].hit = true;
            let keysound = self.chart.notes[i].keysound;
            let j = if dt <= WINDOW_PERFECT { Judgment::Perfect }
                    else if dt <= WINDOW_GREAT { Judgment::Great }
                    else { Judgment::Good };
            self.apply(j, now_ms);
            return keysound;
        }
        None
    }

    pub fn check_misses(&mut self, now_ms: f64) {
        let ids: Vec<usize> = self.chart.notes.iter().enumerate()
            .filter(|(_, n)| !n.hit && n.time_ms + MISS_AFTER < now_ms)
            .map(|(i, _)| i).collect();
        for i in ids {
            self.chart.notes[i].hit = true;
            self.apply(Judgment::Miss, now_ms);
        }
    }

    fn apply(&mut self, j: Judgment, now_ms: f64) {
        match j {
            Judgment::Perfect => self.perfect += 1,
            Judgment::Great   => self.great += 1,
            Judgment::Good    => self.good += 1,
            Judgment::Miss    => self.miss += 1,
        }
        if j == Judgment::Miss {
            self.combo = 0;
        } else {
            self.combo += 1;
            self.max_combo = self.max_combo.max(self.combo);
        }
        self.score += j.points() + self.combo.min(50);
        self.flash.last_judgment = Some(j);
        self.flash.last_judgment_at = now_ms;
    }

    pub fn accuracy(&self) -> f64 {
        let total = self.perfect + self.great + self.good + self.miss;
        if total == 0 { return 100.0; }
        let weighted = self.perfect as f64 * 1.0 + self.great as f64 * 0.65 + self.good as f64 * 0.3;
        weighted / total as f64 * 100.0
    }
}
