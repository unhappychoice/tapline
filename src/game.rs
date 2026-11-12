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
    pub last_miss_at: Option<f64>,
}

pub struct Game {
    pub chart: Chart,
    pub bga: crate::bga::ImageBank,
    pub score: u32,
    pub combo: u32,
    pub max_combo: u32,
    pub perfect: u32,
    pub great: u32,
    pub good: u32,
    pub miss: u32,
    pub flash: FlashState,
    next_miss: usize,
    miss_order: Vec<usize>,
    next_deadline: usize,
    max_good: f64,
    lane_notes: Vec<Vec<usize>>,
    held_notes: Vec<usize>,
    #[cfg(test)]
    miss_visits: usize,
    #[cfg(test)]
    render_visits: std::cell::Cell<usize>,
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

#[derive(Clone, Copy)]
struct JudgmentWindows {
    perfect: f64,
    great: f64,
    good: f64,
    miss_after: f64,
}

impl JudgmentWindows {
    fn for_scale(scale: f64) -> Self {
        Self {
            perfect: WINDOW_PERFECT * scale,
            great: WINDOW_GREAT * scale,
            good: WINDOW_GOOD * scale,
            miss_after: MISS_AFTER * scale,
        }
    }

    fn judge(self, dt: f64) -> Judgment {
        if dt <= self.perfect {
            Judgment::Perfect
        } else if dt <= self.great {
            Judgment::Great
        } else if dt <= self.good {
            Judgment::Good
        } else {
            Judgment::Miss
        }
    }
}

impl Game {
    /// Notes must be sorted by time; their timestamps, lanes, and order stay fixed during play.
    pub fn new(chart: Chart) -> Self {
        let lane_count = chart.lane_count;
        let mut lane_notes = vec![Vec::new(); lane_count];
        chart.notes.iter().enumerate().for_each(|(i, note)| {
            if let Some(lane) = lane_notes.get_mut(note.lane) {
                lane.push(i);
            }
        });
        let mut miss_order: Vec<usize> = (0..chart.notes.len()).collect();
        let deadline = |i: usize| {
            let note = &chart.notes[i];
            note.time_ms + JudgmentWindows::for_scale(chart.judgment_scale(note.time_ms)).miss_after
        };
        miss_order.sort_by(|&a, &b| deadline(a).total_cmp(&deadline(b)).then(a.cmp(&b)));
        let max_good = chart
            .notes
            .iter()
            .map(|note| JudgmentWindows::for_scale(chart.judgment_scale(note.time_ms)).good)
            .fold(0.0, f64::max);
        let bga = crate::bga::ImageBank::load(&chart.bmp_paths);
        let mut events = chart.notes.clone();
        events.extend(chart.invisible_notes.iter().map(|note| Note {
            time_ms: note.time_ms,
            lane: note.lane,
            keysound: Some(note.keysound),
            ..Note::default()
        }));
        events.sort_by(|a, b| a.time_ms.total_cmp(&b.time_ms));
        let mut lane_keysounds = vec![None; lane_count];
        events.iter().rev().for_each(|note| {
            if let Some(sound) = lane_keysounds.get_mut(note.lane) {
                *sound = note.keysound;
            }
        });
        Self {
            next_miss: 0,
            miss_order,
            next_deadline: 0,
            max_good,
            lane_notes,
            held_notes: Vec::new(),
            #[cfg(test)]
            miss_visits: 0,
            #[cfg(test)]
            render_visits: std::cell::Cell::new(0),
            lane_keysounds,
            keysound_events: events.into_iter().peekable(),
            chart,
            bga,
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
                last_miss_at: None,
            },
        }
    }

    /// Judge the nearest unhit note in the lane within the inclusive hit window.
    /// Equal timing distances prefer the earlier note in chart order.
    pub fn hit(&mut self, lane: usize, now_ms: f64) -> Option<u32> {
        if lane >= self.flash.last_lane_hit.len() {
            return None;
        }
        self.update_keysounds(now_ms);
        self.trigger_mines(lane, now_ms);
        let mut best: Option<(usize, f64)> = None;
        for &i in self.hit_candidates(lane, now_ms) {
            let n = &self.chart.notes[i];
            if n.hit || n.held_since.is_some() || n.lane != lane {
                continue;
            }
            let dt = (n.time_ms - now_ms).abs();
            if dt > JudgmentWindows::for_scale(self.chart.judgment_scale(n.time_ms)).good {
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
            let j =
                JudgmentWindows::for_scale(self.chart.judgment_scale(self.chart.notes[i].time_ms))
                    .judge(dt);
            if is_ln {
                self.chart.notes[i].held_since = Some(now_ms);
                self.held_notes.push(i);
                self.held_notes.sort_unstable();
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

    /// Detonate each nearby untriggered mine in the pressed lane once.
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
        let Some(position) = self
            .held_notes
            .iter()
            .position(|&i| self.chart.notes[i].lane == lane)
        else {
            return;
        };
        let i = self.held_notes.remove(position);
        let Some(end_ms) = self.chart.notes[i].end_ms else {
            return;
        };
        let dt = (end_ms - now_ms).abs();
        let j = JudgmentWindows::for_scale(self.chart.judgment_scale(end_ms)).judge(dt);
        self.chart.notes[i].hit = true;
        self.chart.notes[i].held_since = None;
        self.apply(j, now_ms);
    }

    /// Advance pending starts once, then judge tails only for currently held LNs.
    pub fn check_misses(&mut self, now_ms: f64) {
        self.update_keysounds(now_ms);
        while let Some(&i) = self.miss_order.get(self.next_deadline) {
            let note = &self.chart.notes[i];
            let miss_after =
                JudgmentWindows::for_scale(self.chart.judgment_scale(note.time_ms)).miss_after;
            if note.time_ms + miss_after >= now_ms {
                break;
            }
            #[cfg(test)]
            {
                self.miss_visits += 1;
            }
            self.next_deadline += 1;
            if !note.hit && note.held_since.is_none() {
                self.chart.notes[i].hit = true;
                self.apply(Judgment::Miss, now_ms);
            }
        }
        while self
            .chart
            .notes
            .get(self.next_miss)
            .is_some_and(|note| note.hit || note.held_since.is_some())
        {
            self.next_miss += 1;
        }
        let mut position = 0;
        while position < self.held_notes.len() {
            #[cfg(test)]
            {
                self.miss_visits += 1;
            }
            let i = self.held_notes[position];
            if self.chart.notes[i].end_ms.is_some_and(|end| {
                end + JudgmentWindows::for_scale(self.chart.judgment_scale(end)).miss_after < now_ms
            }) {
                self.held_notes.remove(position);
                self.chart.notes[i].hit = true;
                self.chart.notes[i].held_since = None;
                self.apply(Judgment::Perfect, now_ms);
            } else {
                position += 1;
            }
        }
    }

    /// Select lane-local note indexes within the inclusive good judgment window.
    fn hit_candidates(&self, lane: usize, now_ms: f64) -> &[usize] {
        let good = self.max_good;
        let ids = &self.lane_notes[lane];
        let start = ids.partition_point(|&i| self.chart.notes[i].time_ms < now_ms - good);
        let end = ids.partition_point(|&i| self.chart.notes[i].time_ms <= now_ms + good);
        &ids[start..end]
    }

    /// Charts must keep their notes in ascending time order throughout play.
    /// Include pending starts since the last miss check and older held LNs.
    pub fn visible_notes(&self, now_ms: f64, approach_ms: f64) -> impl Iterator<Item = &Note> {
        let end = self
            .chart
            .notes
            .partition_point(|n| n.time_ms <= now_ms + approach_ms);
        let start = self.next_miss.min(end);
        self.held_notes
            .iter()
            .copied()
            .filter(move |&i| i < start)
            .chain(start..end)
            .map(|i| {
                #[cfg(test)]
                self.render_visits.set(self.render_visits.get() + 1);
                &self.chart.notes[i]
            })
            .filter(|note| !note.hit)
    }

    /// Record the judgment, updating score, combo, and visual feedback.
    fn apply(&mut self, j: Judgment, now_ms: f64) {
        match j {
            Judgment::Perfect => self.perfect += 1,
            Judgment::Great => self.great += 1,
            Judgment::Good => self.good += 1,
            Judgment::Miss => self.miss += 1,
        }
        if j == Judgment::Miss {
            self.combo = 0;
            self.flash.last_miss_at = Some(now_ms);
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

    fn ranked_game(rank: Option<u8>, long: bool) -> Game {
        let notes = vec![if long {
            ln(1000.0, 2000.0, 0)
        } else {
            note(1000.0, 0)
        }];
        let mut chart = chart_with(notes, 4);
        chart.rank = rank;
        Game::new(chart)
    }

    #[test]
    fn very_hard_downgrades_a_normal_perfect_to_great() {
        for (rank, expected) in [
            (Some(0), Judgment::Great),
            (Some(2), Judgment::Perfect),
            (None, Judgment::Perfect),
        ] {
            let mut game = ranked_game(rank, false);
            game.hit(0, 1030.0);
            assert_eq!(game.flash.last_judgment, Some(expected));
        }
    }

    #[test]
    fn rank_windows_apply_at_both_sides_of_hit_and_release_boundaries() {
        let rows = [
            (Some(0), [22.5, 45.0, 70.0]),
            (Some(1), [33.75, 67.5, 105.0]),
            (Some(2), [45.0, 90.0, 140.0]),
            (Some(3), [56.25, 112.5, 175.0]),
            (Some(4), [67.5, 135.0, 210.0]),
            (None, [45.0, 90.0, 140.0]),
            (Some(255), [45.0, 90.0, 140.0]),
        ];
        for (rank, boundaries) in rows {
            for (boundary, within, outside) in [
                (boundaries[0], Judgment::Perfect, Judgment::Great),
                (boundaries[1], Judgment::Great, Judgment::Good),
                (boundaries[2], Judgment::Good, Judgment::Miss),
            ] {
                for sign in [-1.0, 1.0] {
                    for (offset, expected) in [(boundary, within), (boundary + 0.5, outside)] {
                        for long in [false, true] {
                            let mut game = ranked_game(rank, long);
                            if long {
                                game.hit(0, 1000.0);
                                game.release(0, 2000.0 + sign * offset);
                                assert!(game.chart.notes[0].hit);
                                assert!(game.chart.notes[0].held_since.is_none());
                            } else {
                                game.hit(0, 1000.0 + sign * offset);
                            }
                            let expected = if !long && expected == Judgment::Miss {
                                None
                            } else {
                                Some(expected)
                            };
                            assert_eq!(
                                game.flash.last_judgment, expected,
                                "rank={rank:?}, offset={offset}, sign={sign}, long={long}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn rank_miss_delays_apply_to_taps_unstarted_and_held_long_notes() {
        for (rank, delay) in [
            (Some(0), 80.0),
            (Some(1), 120.0),
            (Some(2), 160.0),
            (Some(3), 200.0),
            (Some(4), 240.0),
            (None, 160.0),
            (Some(255), 160.0),
        ] {
            for (long, held) in [(false, false), (true, false), (true, true)] {
                let mut game = ranked_game(rank, long);
                let time = if held {
                    game.hit(0, 1000.0);
                    2000.0
                } else {
                    1000.0
                };
                game.check_misses(time + delay);
                assert!(!game.chart.notes[0].hit);
                game.check_misses(time + delay + 0.5);
                assert!(game.chart.notes[0].hit);
                assert!(game.chart.notes[0].held_since.is_none());
                assert_eq!(
                    game.flash.last_judgment,
                    Some(if held {
                        Judgment::Perfect
                    } else {
                        Judgment::Miss
                    })
                );
                let score = game.score;
                game.check_misses(time + delay + 1.0);
                assert_eq!(game.score, score);
            }
        }
    }

    fn sound_note(time_ms: f64, lane: usize, keysound: Option<u32>) -> Note {
        Note {
            keysound,
            ..note(time_ms, lane)
        }
    }

    #[test]
    fn rank_windows_preserve_passed_keysounds_on_empty_presses() {
        for (rank, offset, judged) in [(0, 100.0, false), (4, 180.0, true)] {
            let mut chart = chart_with(vec![note(1000.0, 0), sound_note(2000.0, 0, Some(2))], 4);
            chart.rank = Some(rank);
            let mut game = Game::new(chart);
            assert_eq!(game.hit(0, 2000.0 + offset), Some(2));
            assert_eq!(game.chart.notes[1].hit, judged);
            assert_eq!(game.good, u32::from(judged));
            game.check_misses(2500.0);
            assert_eq!(game.miss, if judged { 1 } else { 2 });
            assert_eq!(game.hit(0, 3000.0), Some(2));
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
    fn frame_and_hit_candidates_stay_bounded_with_large_charts() {
        let work = |extra, rank| {
            let mut notes: Vec<_> = (0..extra).map(|i| note(i as f64, 0)).collect();
            notes.extend([ln(200_000.0, 210_000.0, 0), note(205_000.0, 1)]);
            notes.extend((0..extra).map(|i| note(300_000.0 + i as f64, 1)));
            let mut chart = chart_with(notes, 4);
            chart.rank = rank;
            let mut g = Game::new(chart);
            g.check_misses(200_000.0);
            g.hit(0, 200_000.0);
            g.check_misses(204_999.0);
            g.miss_visits = 0;
            g.check_misses(205_000.0);
            let candidates = g.hit_candidates(1, 205_000.0).len();
            let visible: Vec<_> = g
                .visible_notes(205_000.0, 1000.0)
                .map(|n| (n.time_ms, n.lane))
                .collect();
            assert_eq!(visible, vec![(200_000.0, 0), (205_000.0, 1)]);
            g.hit(1, 205_000.0);
            assert_eq!(g.perfect, 2);
            (g.miss_visits, g.render_visits.get(), candidates)
        };
        for rank in [None, Some(0), Some(1), Some(2), Some(3), Some(4)] {
            assert_eq!(work(0, rank), (1, 2, 1));
            assert_eq!(work(100_000, rank), (1, 2, 1));
        }
    }

    #[test]
    fn miss_cursor_preserves_strict_deadlines_and_never_revisits_notes() {
        let mut g = Game::new(chart_with(vec![note(1000.0, 0), note(1000.0, 1)], 4));
        g.check_misses(1160.0);
        assert_eq!(g.next_miss, 0);
        g.check_misses(1160.1);
        assert_eq!(g.miss, 2);
        assert_eq!(g.next_miss, 2);
        g.miss_visits = 0;
        g.check_misses(5000.0);
        assert_eq!(g.miss_visits, 0);
        assert_eq!(g.visible_notes(5000.0, 1000.0).count(), 0);
        assert_eq!(g.render_visits.get(), 0);
    }

    #[test]
    fn lane_search_includes_window_edges_and_keeps_first_note_on_ties() {
        let mut g = Game::new(chart_with(
            vec![note(860.0, 0), note(1000.0, 1), note(1140.0, 0)],
            4,
        ));
        assert_eq!(g.hit_candidates(0, 1000.0), &[0, 2]);
        g.hit(0, 1000.0);
        assert!(g.chart.notes[0].hit);
        assert!(!g.chart.notes[2].hit);
        g.hit(0, 1000.0);
        assert_eq!(g.good, 2);
    }

    #[test]
    fn visible_candidates_keep_chords_and_old_holds_without_duplicates() {
        let mut g = Game::new(chart_with(
            vec![
                ln(1000.0, 5000.0, 0),
                ln(1000.0, 6000.0, 1),
                note(3000.0, 2),
                note(3000.0, 3),
                note(4000.0, 2),
                note(4000.1, 3),
            ],
            4,
        ));
        g.hit(0, 1000.0);
        g.hit(1, 1000.0);
        assert_eq!(g.visible_notes(1000.0, 1000.0).count(), 2);
        g.check_misses(3000.0);
        assert_eq!(g.visible_notes(3000.0, 1000.0).count(), 5);
        g.release(0, 5000.0);
        assert_eq!(g.held_notes, vec![1]);
        g.check_misses(6160.0);
        assert_eq!(g.held_notes, vec![1]);
        g.check_misses(6160.1);
        assert!(g.held_notes.is_empty());
        assert_eq!(g.perfect, 4);
        assert_eq!(g.visible_notes(6160.1, 1000.0).count(), 0);
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
        assert_eq!(g.flash.last_miss_at, Some(1000.0));
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
        assert_eq!(g.flash.last_miss_at, Some(1500.0));
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
