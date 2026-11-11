use crate::chart::{Chart, Note};

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
            Judgment::Great => 200,
            Judgment::Good => 100,
            Judgment::Miss => 0,
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
    lane_keysounds: Vec<Option<u32>>,
    keysound_events: std::iter::Peekable<std::vec::IntoIter<Note>>,
}

pub const WINDOW_PERFECT: f64 = 45.0;
pub const WINDOW_GREAT: f64 = 90.0;
pub const WINDOW_GOOD: f64 = 140.0;
pub const MISS_AFTER: f64 = 160.0;
/// A mine detonates only if the player presses its lane within this many ms
/// of the mine's own timestamp. Kept tight so hitting a legitimate note that
/// happens to sit close to a mine doesn't accidentally punish the player.
pub const MINE_WINDOW: f64 = 40.0;

impl Game {
    pub fn new(chart: Chart) -> Self {
        let lane_count = chart.lane_count;
        let mut events = chart.notes.clone();
        events.sort_by(|a, b| a.time_ms.total_cmp(&b.time_ms));
        let mut lane_keysounds = vec![None; lane_count];
        events.iter().rev().for_each(|note| {
            if let Some(sound) = lane_keysounds.get_mut(note.lane) {
                *sound = note.keysound;
            }
        });
        Self {
            lane_keysounds,
            keysound_events: events.into_iter().peekable(),
            chart,
            score: 0,
            combo: 0,
            max_combo: 0,
            perfect: 0,
            great: 0,
            good: 0,
            miss: 0,
            flash: FlashState {
                last_judgment: None,
                last_lane_hit: vec![-9999.0; lane_count],
                last_judgment_at: -9999.0,
            },
        }
    }

    pub fn hit(&mut self, lane: usize, now_ms: f64) -> Option<u32> {
        if lane >= self.flash.last_lane_hit.len() {
            return None;
        }
        self.update_keysounds(now_ms);
        self.trigger_mines(lane, now_ms);
        let mut best: Option<(usize, f64)> = None;
        for (i, n) in self.chart.notes.iter().enumerate() {
            if n.hit || n.held_since.is_some() || n.lane != lane {
                continue;
            }
            let dt = (n.time_ms - now_ms).abs();
            if dt > WINDOW_GOOD * self.chart.judgment_scale(n.time_ms) {
                continue;
            }
            if best.is_none_or(|(_, b)| dt < b) {
                best = Some((i, dt));
            }
        }
        self.flash.last_lane_hit[lane] = now_ms;
        if let Some((i, dt)) = best {
            let keysound = self.chart.notes[i].keysound;
            let is_ln = self.chart.notes[i].end_ms.is_some();
            let scale = self.chart.judgment_scale(self.chart.notes[i].time_ms);
            let j = if dt <= WINDOW_PERFECT * scale {
                Judgment::Perfect
            } else if dt <= WINDOW_GREAT * scale {
                Judgment::Great
            } else {
                Judgment::Good
            };
            if is_ln {
                self.chart.notes[i].held_since = Some(now_ms);
            } else {
                self.chart.notes[i].hit = true;
            }
            self.apply(j, now_ms);
            return keysound;
        }
        self.lane_keysounds[lane]
    }

    fn update_keysounds(&mut self, now_ms: f64) {
        while let Some(note) = self.keysound_events.next_if(|note| note.time_ms <= now_ms) {
            if let Some(sound) = self.lane_keysounds.get_mut(note.lane) {
                *sound = note.keysound;
            }
        }
    }

    fn trigger_mines(&mut self, lane: usize, now_ms: f64) {
        let ids: Vec<usize> = self
            .chart
            .mines
            .iter()
            .enumerate()
            .filter(|(_, m)| {
                !m.exploded && m.lane == lane && (m.time_ms - now_ms).abs() <= MINE_WINDOW
            })
            .map(|(i, _)| i)
            .collect();
        for i in ids {
            self.chart.mines[i].exploded = true;
            self.apply(Judgment::Miss, now_ms);
        }
    }

    /// Release the LN currently held on `lane`, judging against its end_ms.
    /// No-op if the lane isn't holding anything.
    pub fn release(&mut self, lane: usize, now_ms: f64) {
        if lane >= self.flash.last_lane_hit.len() {
            return;
        }
        let held = self
            .chart
            .notes
            .iter()
            .position(|n| n.lane == lane && n.held_since.is_some() && !n.hit);
        let Some(i) = held else {
            return;
        };
        let Some(end_ms) = self.chart.notes[i].end_ms else {
            return;
        };
        let dt = (end_ms - now_ms).abs();
        let scale = self.chart.judgment_scale(end_ms);
        let j = if dt <= WINDOW_PERFECT * scale {
            Judgment::Perfect
        } else if dt <= WINDOW_GREAT * scale {
            Judgment::Great
        } else if dt <= WINDOW_GOOD * scale {
            Judgment::Good
        } else {
            Judgment::Miss
        };
        self.chart.notes[i].hit = true;
        self.chart.notes[i].held_since = None;
        self.apply(j, now_ms);
    }

    pub fn check_misses(&mut self, now_ms: f64) {
        self.update_keysounds(now_ms);
        let missed: Vec<usize> = self
            .chart
            .notes
            .iter()
            .enumerate()
            .filter(|(_, n)| {
                !n.hit
                    && n.held_since.is_none()
                    && n.time_ms + MISS_AFTER * self.chart.judgment_scale(n.time_ms) < now_ms
            })
            .map(|(i, _)| i)
            .collect();
        for i in missed {
            self.chart.notes[i].hit = true;
            self.apply(Judgment::Miss, now_ms);
        }
        // Long notes still held past their end + miss window → auto-release
        // as Perfect (the player did hold long enough).
        let auto: Vec<usize> = self
            .chart
            .notes
            .iter()
            .enumerate()
            .filter_map(|(i, n)| {
                if n.hit || n.held_since.is_none() {
                    return None;
                }
                let end = n.end_ms?;
                (end + MISS_AFTER * self.chart.judgment_scale(end) < now_ms).then_some(i)
            })
            .collect();
        for i in auto {
            self.chart.notes[i].hit = true;
            self.chart.notes[i].held_since = None;
            self.apply(Judgment::Perfect, now_ms);
        }
    }

    fn apply(&mut self, j: Judgment, now_ms: f64) {
        match j {
            Judgment::Perfect => self.perfect += 1,
            Judgment::Great => self.great += 1,
            Judgment::Good => self.good += 1,
            Judgment::Miss => self.miss += 1,
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
        if total == 0 {
            return 100.0;
        }
        let weighted =
            self.perfect as f64 * 1.0 + self.great as f64 * 0.65 + self.good as f64 * 0.3;
        weighted / total as f64 * 100.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chart::{keys_for, Chart, Note};

    fn chart_with(notes: Vec<Note>, lane_count: usize) -> Chart {
        Chart {
            title: "test".into(),
            bpm: 120.0,
            notes,
            duration_ms: 60_000.0,
            lane_count,
            keys: keys_for(lane_count),
            ..Chart::default()
        }
    }

    fn note(time_ms: f64, lane: usize) -> Note {
        Note {
            time_ms,
            lane,
            hit: false,
            keysound: Some(1),
            end_ms: None,
            held_since: None,
        }
    }

    fn sound_note(time_ms: f64, lane: usize, keysound: Option<u32>) -> Note {
        Note {
            keysound,
            ..note(time_ms, lane)
        }
    }

    #[test]
    fn empty_presses_use_each_lanes_first_sound_without_changing_gameplay() {
        let mut g = Game::new(chart_with(
            vec![
                sound_note(3000.0, 0, Some(3)),
                note(1000.0, 0),
                sound_note(2000.0, 1, Some(2)),
            ],
            4,
        ));
        assert_eq!(g.hit(0, 0.0), Some(1));
        assert_eq!(g.hit(1, 0.0), Some(2));
        assert_eq!(g.hit(2, 0.0), None);
        assert_eq!((g.score, g.combo, g.max_combo), (0, 0, 0));
        assert_eq!((g.perfect, g.great, g.good, g.miss), (0, 0, 0, 0));
        assert_eq!(g.accuracy(), 100.0);
        assert_eq!(g.flash.last_judgment, None);
        assert!(g.chart.notes.iter().all(|n| !n.hit));
    }

    #[test]
    fn empty_presses_use_latest_passed_sound_even_before_miss_window_expires() {
        let mut g = Game::new(chart_with(
            vec![
                note(1000.0, 0),
                sound_note(2000.0, 0, Some(2)),
                sound_note(3000.0, 0, Some(3)),
            ],
            4,
        ));
        g.check_misses(2000.0);
        assert_eq!(g.hit(0, 2145.0), Some(2));
        assert_eq!(g.miss, 1);
        assert!(!g.chart.notes[1].hit);
        g.check_misses(3200.0);
        assert_eq!(g.hit(0, 4000.0), Some(3));
        assert_eq!(g.miss, 3);
        assert_eq!(g.score, 0);
    }

    #[test]
    fn empty_presses_after_hits_preserve_score_combo_and_judgment() {
        let mut g = Game::new(chart_with(
            vec![note(1000.0, 0), sound_note(2000.0, 0, Some(2))],
            4,
        ));
        assert_eq!(g.hit(0, 1000.0), Some(1));
        assert_eq!(g.hit(0, 1500.0), Some(1));
        assert_eq!(g.hit(0, 2000.0), Some(2));
        g.check_misses(2500.0);
        assert_eq!(g.hit(0, 3000.0), Some(2));
        assert_eq!((g.score, g.combo, g.max_combo), (603, 2, 2));
        assert_eq!((g.perfect, g.great, g.good, g.miss), (2, 0, 0, 0));
        assert_eq!(g.flash.last_judgment, Some(Judgment::Perfect));
        assert_eq!(g.flash.last_judgment_at, 2000.0);
    }

    #[test]
    fn passing_notes_updates_only_their_lane_and_can_clear_the_sound() {
        let mut g = Game::new(chart_with(
            vec![
                note(1000.0, 0),
                sound_note(2000.0, 0, Some(2)),
                sound_note(1000.0, 1, Some(3)),
                sound_note(3000.0, 0, None),
            ],
            4,
        ));
        assert_eq!(g.hit(0, 2500.0), Some(2));
        assert_eq!(g.hit(1, 2500.0), Some(3));
        assert_eq!(g.hit(0, 3500.0), None);
        assert_eq!(g.hit(1, 3500.0), Some(3));
    }

    #[test]
    fn early_hit_plays_target_sound_and_passage_updates_held_long_note_sound() {
        let mut held = ln(2000.0, 4000.0, 0);
        held.keysound = Some(2);
        let mut g = Game::new(chart_with(vec![note(1000.0, 0), held], 4));
        assert_eq!(g.hit(0, 1900.0), Some(2));
        g.check_misses(2100.0);
        assert_eq!(g.hit(0, 2500.0), Some(2));
        assert!(g.chart.notes[1].held_since.is_some());
        assert_eq!(g.good, 1);
    }

    #[test]
    fn judgment_points_ladder() {
        assert_eq!(Judgment::Perfect.points(), 300);
        assert_eq!(Judgment::Great.points(), 200);
        assert_eq!(Judgment::Good.points(), 100);
        assert_eq!(Judgment::Miss.points(), 0);
    }

    #[test]
    fn new_game_starts_zeroed() {
        let g = Game::new(chart_with(vec![note(1000.0, 0)], 4));
        assert_eq!(g.score, 0);
        assert_eq!(g.combo, 0);
        assert_eq!(g.max_combo, 0);
        assert_eq!(g.perfect + g.great + g.good + g.miss, 0);
        assert_eq!(g.flash.last_lane_hit.len(), 4);
        assert!(g.flash.last_lane_hit.iter().all(|&v| v < 0.0));
        assert!(g.flash.last_judgment.is_none());
        assert_eq!(g.accuracy(), 100.0);
    }

    #[test]
    fn hit_within_perfect_window_scores_perfect() {
        let mut g = Game::new(chart_with(vec![note(1000.0, 0)], 4));
        let ks = g.hit(0, 1030.0);
        assert_eq!(ks, Some(1));
        assert_eq!(g.perfect, 1);
        assert_eq!(g.combo, 1);
        assert_eq!(g.score, Judgment::Perfect.points() + 1);
        assert_eq!(g.flash.last_judgment, Some(Judgment::Perfect));
    }

    #[test]
    fn hit_just_past_perfect_is_great() {
        let mut g = Game::new(chart_with(vec![note(1000.0, 0)], 4));
        g.hit(0, 1000.0 + WINDOW_PERFECT + 0.5);
        assert_eq!(g.great, 1);
        assert_eq!(g.perfect, 0);
    }

    #[test]
    fn hit_just_past_great_is_good() {
        let mut g = Game::new(chart_with(vec![note(1000.0, 0)], 4));
        g.hit(0, 1000.0 + WINDOW_GREAT + 0.5);
        assert_eq!(g.good, 1);
        assert_eq!(g.great, 0);
    }

    #[test]
    fn hit_outside_good_window_returns_keysound_without_judgment() {
        let mut g = Game::new(chart_with(vec![note(1000.0, 0)], 4));
        let ks = g.hit(0, 1000.0 + WINDOW_GOOD + 5.0);
        assert_eq!(ks, Some(1));
        assert_eq!(g.perfect + g.great + g.good + g.miss, 0);
        assert_eq!(g.score, 0);
    }

    #[test]
    fn hit_only_scores_notes_on_the_pressed_lane() {
        let mut g = Game::new(chart_with(vec![note(1000.0, 1)], 4));
        assert!(g.hit(0, 1000.0).is_none());
        assert_eq!(g.perfect, 0);
        assert!(!g.chart.notes[0].hit);
    }

    #[test]
    fn hit_picks_the_closest_pending_note_in_the_lane() {
        let mut g = Game::new(chart_with(vec![note(1000.0, 0), note(1100.0, 0)], 4));
        g.hit(0, 1090.0);
        assert!(g.chart.notes[1].hit);
        assert!(!g.chart.notes[0].hit);
    }

    #[test]
    fn hit_ignores_already_hit_notes() {
        let mut g = Game::new(chart_with(vec![note(1000.0, 0)], 4));
        g.hit(0, 1000.0);
        assert_eq!(g.perfect, 1);
        let extra = g.hit(0, 1005.0);
        assert_eq!(extra, Some(1));
        assert_eq!(g.perfect, 1);
    }

    #[test]
    fn hit_on_out_of_range_lane_returns_none() {
        let mut g = Game::new(chart_with(vec![note(1000.0, 0)], 4));
        assert!(g.hit(99, 1000.0).is_none());
    }

    #[test]
    fn miss_check_flags_notes_beyond_the_miss_window() {
        let mut g = Game::new(chart_with(vec![note(1000.0, 0), note(2000.0, 0)], 4));
        g.check_misses(1000.0 + MISS_AFTER + 1.0);
        assert_eq!(g.miss, 1);
        assert!(g.chart.notes[0].hit);
        assert!(!g.chart.notes[1].hit);
    }

    #[test]
    fn miss_resets_combo_but_hits_advance_max_combo() {
        let mut g = Game::new(chart_with(
            vec![note(1000.0, 0), note(1100.0, 0), note(1200.0, 0)],
            4,
        ));
        g.hit(0, 1000.0);
        g.hit(0, 1100.0);
        assert_eq!(g.combo, 2);
        assert_eq!(g.max_combo, 2);
        g.check_misses(1200.0 + MISS_AFTER + 1.0);
        assert_eq!(g.combo, 0);
        assert_eq!(g.max_combo, 2);
    }

    #[test]
    fn combo_bonus_caps_at_fifty() {
        let notes: Vec<Note> = (0..60)
            .map(|i| note(1000.0 + i as f64 * 200.0, 0))
            .collect();
        let mut g = Game::new(chart_with(notes, 4));
        for i in 0..60 {
            g.hit(0, 1000.0 + i as f64 * 200.0);
        }
        assert_eq!(g.combo, 60);
        let scored_perfect = 60 * Judgment::Perfect.points();
        let bonus: u32 = (1..=60).map(|c| c.min(50)).sum();
        assert_eq!(g.score, scored_perfect + bonus);
    }

    #[test]
    fn accuracy_weights_tiers() {
        let mut g = Game::new(chart_with(
            vec![
                note(1000.0, 0),
                note(2000.0, 0),
                note(3000.0, 0),
                note(4000.0, 0),
            ],
            4,
        ));
        g.hit(0, 1000.0);
        g.hit(0, 2000.0 + WINDOW_PERFECT + 1.0);
        g.hit(0, 3000.0 + WINDOW_GREAT + 1.0);
        g.check_misses(4000.0 + MISS_AFTER + 1.0);
        let expected = (1.0 + 0.65 + 0.3) / 4.0 * 100.0;
        assert!((g.accuracy() - expected).abs() < 1e-9);
    }

    fn ln(time_ms: f64, end_ms: f64, lane: usize) -> Note {
        Note {
            time_ms,
            lane,
            end_ms: Some(end_ms),
            keysound: Some(1),
            ..Default::default()
        }
    }

    use crate::chart::Mine;

    fn chart_with_mines(mines: Vec<Mine>, lane_count: usize) -> Chart {
        Chart {
            title: "mine".into(),
            bpm: 120.0,
            mines,
            duration_ms: 60_000.0,
            lane_count,
            keys: keys_for(lane_count),
            ..Chart::default()
        }
    }

    #[test]
    fn pressing_a_lane_within_mine_window_detonates_the_mine_as_miss() {
        let mines = vec![Mine {
            time_ms: 1000.0,
            lane: 0,
            damage: 40,
            exploded: false,
        }];
        let mut g = Game::new(chart_with_mines(mines, 4));
        g.hit(0, 1000.0);
        assert_eq!(g.miss, 1, "mine detonation is scored as a Miss");
        assert!(g.chart.mines[0].exploded);
        assert_eq!(g.combo, 0);
    }

    #[test]
    fn pressing_a_lane_outside_mine_window_leaves_the_mine_armed() {
        let mines = vec![Mine {
            time_ms: 1000.0,
            lane: 0,
            damage: 40,
            exploded: false,
        }];
        let mut g = Game::new(chart_with_mines(mines, 4));
        g.hit(0, 1000.0 + MINE_WINDOW + 10.0);
        assert_eq!(g.miss, 0);
        assert!(!g.chart.mines[0].exploded);
    }

    #[test]
    fn pressing_the_wrong_lane_never_triggers_a_mine() {
        let mines = vec![Mine {
            time_ms: 1000.0,
            lane: 0,
            damage: 40,
            exploded: false,
        }];
        let mut g = Game::new(chart_with_mines(mines, 4));
        g.hit(1, 1000.0);
        assert_eq!(g.miss, 0);
        assert!(!g.chart.mines[0].exploded);
    }

    #[test]
    fn a_mine_only_detonates_once_even_on_repeated_presses() {
        let mines = vec![Mine {
            time_ms: 1000.0,
            lane: 0,
            damage: 40,
            exploded: false,
        }];
        let mut g = Game::new(chart_with_mines(mines, 4));
        g.hit(0, 1000.0);
        g.hit(0, 1010.0);
        assert_eq!(g.miss, 1, "the same mine can only punish once");
    }

    #[test]
    fn ln_start_press_marks_it_held_but_not_hit() {
        let mut g = Game::new(chart_with(vec![ln(1000.0, 2000.0, 0)], 4));
        g.hit(0, 1000.0);
        assert_eq!(g.perfect, 1, "start still scores like a tap");
        assert!(!g.chart.notes[0].hit, "hit stays false until release");
        assert!(g.chart.notes[0].held_since.is_some());
    }

    #[test]
    fn ln_release_within_window_scores_and_marks_hit() {
        let mut g = Game::new(chart_with(vec![ln(1000.0, 2000.0, 0)], 4));
        g.hit(0, 1000.0);
        g.release(0, 2000.0);
        assert_eq!(g.perfect, 2, "start + release both perfect");
        assert!(g.chart.notes[0].hit);
        assert!(g.chart.notes[0].held_since.is_none());
    }

    #[test]
    fn ln_release_far_from_end_is_a_miss() {
        let mut g = Game::new(chart_with(vec![ln(1000.0, 2000.0, 0)], 4));
        g.hit(0, 1000.0);
        // Release ~500ms early, well outside WINDOW_GOOD.
        g.release(0, 1500.0);
        assert_eq!(g.miss, 1);
        assert_eq!(g.combo, 0);
    }

    #[test]
    fn hit_will_not_repick_a_held_ln() {
        let mut g = Game::new(chart_with(vec![ln(1000.0, 2000.0, 0)], 4));
        g.hit(0, 1000.0);
        let ks = g.hit(0, 1010.0);
        assert_eq!(ks, Some(1), "second press replays the assigned keysound");
        assert_eq!(g.perfect, 1, "no second Perfect awarded");
    }

    #[test]
    fn check_misses_auto_releases_held_ln_past_end_window_as_perfect() {
        let mut g = Game::new(chart_with(vec![ln(1000.0, 2000.0, 0)], 4));
        g.hit(0, 1000.0);
        g.check_misses(2000.0 + MISS_AFTER + 1.0);
        assert_eq!(g.perfect, 2, "auto-release keeps the perfect tier");
        assert!(g.chart.notes[0].hit);
    }

    #[test]
    fn check_misses_flags_ln_whose_start_was_never_hit_as_miss() {
        let mut g = Game::new(chart_with(vec![ln(1000.0, 2000.0, 0)], 4));
        g.check_misses(1000.0 + MISS_AFTER + 1.0);
        assert_eq!(g.miss, 1);
        assert!(g.chart.notes[0].hit);
    }

    #[test]
    fn release_on_lane_that_isnt_holding_is_a_noop() {
        let mut g = Game::new(chart_with(vec![note(1000.0, 0)], 4));
        g.release(0, 1000.0);
        assert_eq!(g.perfect + g.great + g.good + g.miss, 0);
    }

    #[test]
    fn chord_notes_are_hit_independently_per_lane() {
        let mut g = Game::new(chart_with(vec![note(1000.0, 0), note(1000.0, 1)], 4));
        g.hit(0, 1000.0);
        g.hit(1, 1000.0);
        assert_eq!(g.perfect, 2);
        assert_eq!(g.combo, 2);
        assert!(g.chart.notes.iter().all(|n| n.hit));
    }
}
