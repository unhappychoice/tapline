use crate::chart::VolumeTarget;
use crate::{audio, chart, cli::Args, game, render};
use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute, terminal,
};
use std::collections::HashSet;
use std::io::Stdout;
use std::time::{Duration, Instant};

pub struct PlayOptions {
    pub countdown_ms: f64,
    pub synth_mode: bool,
    pub auto_ks: bool,
    pub audio_lead_ms: f64,
}

impl PlayOptions {
    pub fn from_args(args: &Args) -> Self {
        Self {
            countdown_ms: args.countdown_ms,
            synth_mode: args.synth,
            auto_ks: args.auto_ks,
            audio_lead_ms: args.audio_lead_ms,
        }
    }
}

pub fn play_chart(out: &mut Stdout, args: &Args, mut chart: chart::Chart) -> Result<()> {
    let opts = PlayOptions::from_args(args);
    let bank = if args.no_audio {
        audio::SampleBank::silent()
    } else {
        audio::SampleBank::with_settings(&chart.wav_paths, &chart.sample_settings)
            .with_slices(&chart.sample_slices)
            .with_layers(&chart.sound_layers)
    };
    chart.duration_ms = bank.chart_duration_ms(&chart, opts.auto_ks, opts.audio_lead_ms);
    let mut game = game::Game::new(chart);
    execute!(out, terminal::Clear(terminal::ClearType::All))?;
    run_game(out, &mut game, &bank, &opts)?;
    after_playback(bank, || show_result(out, &game))
}

fn after_playback<T, R>(playback: T, show_result: impl FnOnce() -> R) -> R {
    drop(playback);
    show_result()
}

const FRAME_DT: Duration = Duration::from_millis(16);

fn run_game(
    out: &mut Stdout,
    game: &mut game::Game,
    bank: &audio::SampleBank,
    opts: &PlayOptions,
) -> Result<()> {
    let start = Instant::now();
    let mut next_frame = Instant::now() + FRAME_DT;
    let mut state = LoopState::new(build_auto_notes(game, opts));

    while !state.quit {
        let now = elapsed_ms(start);
        if game.advance_completion(now) {
            break;
        }
        draw_frame(out, game, bank, opts, &mut state, now)?;
        pump_input(out, game, bank, opts, &mut state, start, next_frame)?;
        next_frame = advance_deadline(next_frame);
    }
    Ok(())
}

struct LoopState {
    quit: bool,
    pressed_keys: HashSet<char>,
    bgm_cursor: usize,
    note_snd_cursor: usize,
    prev_mode: u8,
    auto_notes: Vec<(f64, usize, Option<u32>)>,
}

impl LoopState {
    fn new(auto_notes: Vec<(f64, usize, Option<u32>)>) -> Self {
        Self {
            quit: false,
            pressed_keys: HashSet::new(),
            bgm_cursor: 0,
            note_snd_cursor: 0,
            prev_mode: 0,
            auto_notes,
        }
    }
}

fn build_auto_notes(game: &game::Game, opts: &PlayOptions) -> Vec<(f64, usize, Option<u32>)> {
    if !opts.auto_ks {
        return Vec::new();
    }
    let mut v: Vec<_> = game
        .chart
        .notes
        .iter()
        .map(|n| (n.time_ms, n.lane, n.keysound))
        .collect();
    v.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    v
}

fn draw_frame(
    out: &mut Stdout,
    game: &mut game::Game,
    bank: &audio::SampleBank,
    opts: &PlayOptions,
    state: &mut LoopState,
    now: f64,
) -> Result<()> {
    fire_scheduled_audio(bank, opts, state, game, now);
    let mode: u8 = if now < opts.countdown_ms { 1 } else { 2 };
    if mode != state.prev_mode {
        execute!(out, terminal::Clear(terminal::ClearType::All))?;
        state.prev_mode = mode;
    }
    if mode == 1 {
        render::draw_intro_full(
            out,
            game,
            opts.countdown_ms - now,
            bank.enabled,
            bank.sample_count(),
            bank.decode_failures,
        )?;
        return Ok(());
    }
    game.check_misses(now);
    render::draw(out, game, now)
}

fn fire_scheduled_audio(
    bank: &audio::SampleBank,
    opts: &PlayOptions,
    state: &mut LoopState,
    game: &game::Game,
    now: f64,
) {
    let horizon = now + opts.audio_lead_ms;
    while state.bgm_cursor < game.chart.bgm.len()
        && game.chart.bgm[state.bgm_cursor].time_ms <= horizon
    {
        let event = &game.chart.bgm[state.bgm_cursor];
        let gain = game.volume.gain(VolumeTarget::Bgm, event.time_ms);
        bank.play_with_gain(event.keysound, gain);
        state.bgm_cursor += 1;
    }
    while state.note_snd_cursor < state.auto_notes.len()
        && state.auto_notes[state.note_snd_cursor].0 <= horizon
    {
        let (time_ms, lane, ks) = state.auto_notes[state.note_snd_cursor];
        let gain = game.volume.gain(VolumeTarget::Key, time_ms);
        bank.play_hit_with_gain(lane, ks, opts.synth_mode, gain);
        state.note_snd_cursor += 1;
    }
}

fn pump_input(
    out: &mut Stdout,
    game: &mut game::Game,
    bank: &audio::SampleBank,
    opts: &PlayOptions,
    state: &mut LoopState,
    start: Instant,
    next_frame: Instant,
) -> Result<()> {
    loop {
        let inst = Instant::now();
        if inst >= next_frame {
            return Ok(());
        }
        if !event::poll(next_frame - inst)? {
            return Ok(());
        }
        let input = event::read()?;
        if matches!(input, Event::Resize(..)) {
            execute!(out, terminal::Clear(terminal::ClearType::All))?;
        }
        let Event::Key(k) = input else {
            continue;
        };
        if k.kind == KeyEventKind::Repeat {
            continue;
        }
        if handle_key(k.code, k.kind, game, bank, opts, state, elapsed_ms(start)) {
            return Ok(());
        }
        let _ = out;
    }
}

fn handle_key(
    code: KeyCode,
    kind: KeyEventKind,
    game: &mut game::Game,
    bank: &audio::SampleBank,
    opts: &PlayOptions,
    state: &mut LoopState,
    now: f64,
) -> bool {
    match code {
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') if kind == KeyEventKind::Press => {
            state.quit = true;
            true
        }
        KeyCode::Char(c) => {
            if now < opts.countdown_ms {
                return false;
            }
            let Some(lane) = lane_for_key(c, &game.chart.keys) else {
                return false;
            };
            match kind {
                KeyEventKind::Press => {
                    state.pressed_keys.insert(c.to_ascii_uppercase());
                    let ks = game.hit(lane, now);
                    if !opts.auto_ks {
                        let gain = game.volume.gain(VolumeTarget::Key, now);
                        bank.play_hit_with_gain(lane, ks, opts.synth_mode, gain);
                    }
                }
                KeyEventKind::Release
                    if state.pressed_keys.remove(&c.to_ascii_uppercase())
                        && !state
                            .pressed_keys
                            .iter()
                            .any(|&key| lane_for_key(key, &game.chart.keys) == Some(lane)) =>
                {
                    game.release(lane, now);
                }
                _ => {}
            }
            false
        }
        _ => false,
    }
}

fn advance_deadline(next_frame: Instant) -> Instant {
    let bumped = next_frame + FRAME_DT;
    let now = Instant::now();
    if bumped < now {
        now
    } else {
        bumped
    }
}

fn show_result(out: &mut Stdout, game: &game::Game) -> Result<()> {
    execute!(out, terminal::Clear(terminal::ClearType::All))?;
    render::draw_result(out, game)?;
    loop {
        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(k) = event::read()? {
                if k.kind == KeyEventKind::Press {
                    return Ok(());
                }
            }
        }
    }
}

pub fn lane_for_key(c: char, keys: &[Vec<char>]) -> Option<usize> {
    let up = c.to_ascii_uppercase();
    keys.iter()
        .position(|ks| ks.iter().any(|k| k.to_ascii_uppercase() == up))
}

pub fn elapsed_ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chart::keys_for;

    #[test]
    fn lane_for_key_matches_case_insensitively() {
        let keys = keys_for(4);
        assert_eq!(lane_for_key('s', &keys), Some(0));
        assert_eq!(lane_for_key('S', &keys), Some(0));
        assert_eq!(lane_for_key('L', &keys), Some(3));
        assert_eq!(lane_for_key('l', &keys), Some(3));
    }

    #[test]
    fn lane_for_key_returns_none_for_unmapped_char() {
        let keys = keys_for(4);
        assert_eq!(lane_for_key('Q', &keys), None);
        assert_eq!(lane_for_key('a', &keys), None);
        assert_eq!(lane_for_key(' ', &keys), None);
    }

    #[test]
    fn lane_for_key_5k_center_accepts_both_f_and_j() {
        let keys = keys_for(5);
        assert_eq!(lane_for_key('F', &keys), Some(2));
        assert_eq!(lane_for_key('J', &keys), Some(2));
        assert_eq!(lane_for_key('f', &keys), Some(2));
        assert_eq!(lane_for_key('j', &keys), Some(2));
    }

    #[test]
    fn lane_for_key_7k_maps_space_to_center_lane() {
        let keys = keys_for(7);
        assert_eq!(lane_for_key(' ', &keys), Some(3));
        assert_eq!(lane_for_key('F', &keys), Some(2));
        assert_eq!(lane_for_key('J', &keys), Some(4));
    }

    #[test]
    fn lane_for_key_returns_first_matching_lane() {
        let keys = vec![vec!['A'], vec!['A', 'B']];
        assert_eq!(lane_for_key('A', &keys), Some(0));
        assert_eq!(lane_for_key('B', &keys), Some(1));
    }

    #[test]
    fn advance_deadline_bumps_forward_when_on_time() {
        let base = Instant::now() + Duration::from_secs(60);
        let next = advance_deadline(base);
        assert_eq!(next, base + FRAME_DT);
    }

    #[test]
    fn advance_deadline_clamps_to_now_when_far_behind() {
        let long_ago = Instant::now() - Duration::from_secs(60);
        let bumped = advance_deadline(long_ago);
        let now = Instant::now();
        let diff = now
            .checked_duration_since(bumped)
            .unwrap_or_else(|| bumped.duration_since(now));
        assert!(
            diff < Duration::from_secs(1),
            "advance_deadline should re-anchor to ~now instead of staying in the past"
        );
    }

    #[test]
    fn elapsed_ms_grows_monotonically() {
        let start = Instant::now();
        let first = elapsed_ms(start);
        std::thread::sleep(Duration::from_millis(5));
        let second = elapsed_ms(start);
        assert!(second > first);
    }

    // ---------- LoopState + build_auto_notes ----------

    use crate::chart::{Chart, Note};
    use crate::game::Game;

    fn base_chart(lane_count: usize, notes: Vec<Note>) -> Chart {
        Chart {
            title: "t".into(),
            bpm: 120.0,
            notes,
            duration_ms: 30_000.0,
            lane_count,
            keys: keys_for(lane_count),
            ..Chart::default()
        }
    }

    fn opts(auto_ks: bool) -> PlayOptions {
        PlayOptions {
            countdown_ms: 2000.0,
            synth_mode: true,
            auto_ks,
            audio_lead_ms: 0.0,
        }
    }

    #[test]
    fn completion_applies_final_rank_four_miss_before_result() {
        let mut chart = base_chart(
            4,
            vec![
                Note {
                    time_ms: 1000.0,
                    lane: 0,
                    ..Note::default()
                },
                Note {
                    time_ms: 2000.0,
                    lane: 0,
                    ..Note::default()
                },
            ],
        );
        chart.rank = Some(4);
        chart.total = Some(120.0);
        chart.duration_ms = 2192.0;
        let mut game = Game::new(chart);
        game.hit(0, 1000.0);
        assert!(game.gauge.is_clear());
        assert!(!game.advance_completion(2192.0));
        assert!(!game.advance_completion(2240.0));
        assert_eq!(game.miss, 0);
        assert!(game.advance_completion(2241.0));
        assert_eq!(game.miss, 1);
        assert!(!game.gauge.is_clear());
    }

    #[test]
    fn completion_waits_for_held_tail_and_audio_duration() {
        let mut chart = base_chart(
            4,
            vec![Note {
                time_ms: 1000.0,
                lane: 0,
                end_ms: Some(2000.0),
                ..Note::default()
            }],
        );
        chart.rank = Some(4);
        chart.duration_ms = 2192.0;
        let mut game = Game::new(chart);
        game.hit(0, 1000.0);
        assert!(!game.advance_completion(2240.0));
        assert!(game.advance_completion(2241.0));
        assert_eq!(game.perfect, 2);

        let mut game = Game::new(base_chart(4, vec![]));
        assert!(!game.advance_completion(29999.0));
        assert!(game.advance_completion(30000.0));
    }

    #[test]
    fn loop_state_new_zeroes_cursors_and_flags() {
        let s = LoopState::new(Vec::new());
        assert!(!s.quit);
        assert_eq!(s.bgm_cursor, 0);
        assert_eq!(s.note_snd_cursor, 0);
        assert_eq!(s.prev_mode, 0);
        assert!(s.auto_notes.is_empty());
    }

    #[test]
    fn build_auto_notes_is_empty_when_auto_ks_is_off() {
        let notes = vec![
            Note {
                time_ms: 1000.0,
                lane: 0,
                hit: false,
                keysound: Some(1),
                end_ms: None,
                held_since: None,
            },
            Note {
                time_ms: 500.0,
                lane: 1,
                hit: false,
                keysound: Some(2),
                end_ms: None,
                held_since: None,
            },
        ];
        let game = Game::new(base_chart(4, notes));
        let auto = build_auto_notes(&game, &opts(false));
        assert!(auto.is_empty(), "no auto notes without --auto-ks");
    }

    #[test]
    fn build_auto_notes_sorts_by_time_when_auto_ks_is_on() {
        let notes = vec![
            Note {
                time_ms: 1000.0,
                lane: 0,
                hit: false,
                keysound: Some(1),
                end_ms: None,
                held_since: None,
            },
            Note {
                time_ms: 500.0,
                lane: 1,
                hit: false,
                keysound: Some(2),
                end_ms: None,
                held_since: None,
            },
            Note {
                time_ms: 800.0,
                lane: 2,
                hit: false,
                keysound: Some(3),
                end_ms: None,
                held_since: None,
            },
        ];
        let game = Game::new(base_chart(4, notes));
        let auto = build_auto_notes(&game, &opts(true));
        let times: Vec<f64> = auto.iter().map(|(t, _, _)| *t).collect();
        assert_eq!(times, vec![500.0, 800.0, 1000.0]);
        // Lane + keysound survive the sort.
        assert_eq!(auto[0].1, 1);
        assert_eq!(auto[0].2, Some(2));
    }

    #[test]
    fn play_options_from_args_carries_over_the_synth_flag() {
        use crate::cli::Args;
        use clap::Parser;
        [false, true].into_iter().for_each(|synth| {
            let mut args = Args::parse_from(["tapline"]);
            args.synth = synth;
            assert_eq!(PlayOptions::from_args(&args).synth_mode, synth);
        });
    }

    #[test]
    fn play_options_from_args_carries_over_the_audio_lead() {
        use crate::cli::Args;
        use clap::Parser;
        let args = Args::parse_from(["tapline", "--audio-lead-ms", "72"]);
        let opts = PlayOptions::from_args(&args);
        assert_eq!(opts.audio_lead_ms, 72.0);
    }

    #[test]
    fn play_options_from_args_carries_over_the_auto_ks_flag() {
        use crate::cli::Args;
        use clap::Parser;
        let args = Args::parse_from(["tapline", "--auto-ks"]);
        let opts = PlayOptions::from_args(&args);
        assert!(opts.auto_ks);
    }

    // ---------- fire_scheduled_audio ----------

    use crate::audio::SampleBank;
    use crate::chart::BgmEvent;

    fn chart_with_bgm(bgm: Vec<BgmEvent>) -> Chart {
        Chart {
            bpm: 120.0,
            bgm,
            duration_ms: 30_000.0,
            ..Chart::default()
        }
    }

    #[test]
    fn draw_frame_schedules_countdown_audio_at_each_lead_time() {
        for auto_ks in [false, true] {
            let times = [2000.0, 2032.0, 2064.0];
            let notes = times
                .iter()
                .map(|&time_ms| Note {
                    time_ms,
                    lane: 0,
                    hit: false,
                    keysound: Some(1),
                    end_ms: None,
                    held_since: None,
                })
                .collect();
            let mut chart = base_chart(4, notes);
            chart.bgm = times
                .iter()
                .map(|&time_ms| BgmEvent {
                    time_ms,
                    keysound: 1,
                })
                .collect();
            let mut game = Game::new(chart);
            let bank = SampleBank::silent();
            let mut options = opts(auto_ks);
            options.audio_lead_ms = 80.0;
            let mut state = LoopState::new(build_auto_notes(&game, &options));
            let mut out = std::io::stdout();

            for (now, expected) in [
                (1919.0, 0),
                (1920.0, 1),
                (1936.0, 1),
                (1952.0, 2),
                (1968.0, 2),
                (1984.0, 3),
                (2000.0, 3),
            ] {
                // Rendering can fail without a terminal; audio must still be scheduled.
                let _ = draw_frame(&mut out, &mut game, &bank, &options, &mut state, now);
                assert_eq!(state.bgm_cursor, expected, "BGM at {now} ms");
                assert_eq!(
                    state.note_snd_cursor,
                    if auto_ks { expected } else { 0 },
                    "keysounds at {now} ms (auto_ks={auto_ks})"
                );
                assert_eq!(game.miss, 0);
                assert!(game.chart.notes.iter().all(|note| !note.hit));
            }
        }
    }

    #[test]
    fn scheduled_audio_waits_for_negative_lead_delay() {
        let mut chart = base_chart(
            4,
            vec![Note {
                time_ms: 2000.0,
                keysound: Some(1),
                ..Default::default()
            }],
        );
        chart.bgm.push(BgmEvent {
            time_ms: 2000.0,
            keysound: 1,
        });
        let game = Game::new(chart);
        let mut options = opts(true);
        options.audio_lead_ms = -5000.0;
        let bank = SampleBank::silent();
        let mut state = LoopState::new(build_auto_notes(&game, &options));
        fire_scheduled_audio(&bank, &options, &mut state, &game, 6999.0);
        assert_eq!((state.bgm_cursor, state.note_snd_cursor), (0, 0));
        fire_scheduled_audio(&bank, &options, &mut state, &game, 7000.0);
        assert_eq!((state.bgm_cursor, state.note_snd_cursor), (1, 1));
    }

    #[test]
    fn fire_scheduled_audio_advances_bgm_cursor_up_to_horizon() {
        let bgm = vec![
            BgmEvent {
                time_ms: 500.0,
                keysound: 1,
            },
            BgmEvent {
                time_ms: 1000.0,
                keysound: 2,
            },
            BgmEvent {
                time_ms: 1500.0,
                keysound: 3,
            },
        ];
        let game = Game::new(chart_with_bgm(bgm));
        let bank = SampleBank::silent();
        let mut state = LoopState::new(Vec::new());
        fire_scheduled_audio(&bank, &opts(false), &mut state, &game, 1100.0);
        assert_eq!(
            state.bgm_cursor, 2,
            "should have fired the first two BGM events"
        );
    }

    #[test]
    fn preplayed_audio_uses_event_time_for_volume() {
        for target in [VolumeTarget::Bgm, VolumeTarget::Key] {
            let mut chart = chart_with_bgm(vec![]);
            chart.vol_wav = Some(50);
            chart.volume_events = vec![crate::chart::VolumeEvent {
                time_ms: 1000.0,
                target,
                volume: 128,
            }];
            chart.bgm = if target == VolumeTarget::Bgm {
                vec![BgmEvent {
                    time_ms: 1050.0,
                    keysound: 1,
                }]
            } else {
                vec![]
            };
            let game = game::Game::new(chart);
            let auto_notes = if target == VolumeTarget::Key {
                vec![(1050.0, 0, Some(1))]
            } else {
                vec![]
            };
            let mut state = LoopState::new(auto_notes);
            let mut options = opts(true);
            options.audio_lead_ms = 250.0;
            fire_scheduled_audio(&SampleBank::silent(), &options, &mut state, &game, 800.0);
            assert_eq!(
                game.volume.last_query.get(),
                Some((target, 1050.0, 0.5 * 128.0 / 255.0))
            );
        }
    }

    #[test]
    fn fire_scheduled_audio_uses_audio_lead_to_pre_play_early() {
        let bgm = vec![
            BgmEvent {
                time_ms: 500.0,
                keysound: 1,
            },
            BgmEvent {
                time_ms: 1000.0,
                keysound: 2,
            },
        ];
        let game = Game::new(chart_with_bgm(bgm));
        let bank = SampleBank::silent();
        let mut state = LoopState::new(Vec::new());
        let mut o = opts(false);
        o.audio_lead_ms = 250.0;
        // horizon = 800 + 250 = 1050 → both events fire
        fire_scheduled_audio(&bank, &o, &mut state, &game, 800.0);
        assert_eq!(state.bgm_cursor, 2);
    }

    #[test]
    fn fire_scheduled_audio_advances_note_cursor_only_when_auto_ks_is_on() {
        let notes = vec![
            Note {
                time_ms: 200.0,
                lane: 0,
                hit: false,
                keysound: Some(1),
                end_ms: None,
                held_since: None,
            },
            Note {
                time_ms: 400.0,
                lane: 1,
                hit: false,
                keysound: Some(2),
                end_ms: None,
                held_since: None,
            },
        ];
        let game = Game::new(base_chart(4, notes));
        let bank = SampleBank::silent();

        // Without auto-ks the buffer is empty, so the cursor stays at 0.
        let mut off = LoopState::new(build_auto_notes(&game, &opts(false)));
        fire_scheduled_audio(&bank, &opts(false), &mut off, &game, 1000.0);
        assert_eq!(off.note_snd_cursor, 0);

        // With auto-ks all notes ≤ 1000 ms fire.
        let mut on = LoopState::new(build_auto_notes(&game, &opts(true)));
        fire_scheduled_audio(&bank, &opts(true), &mut on, &game, 1000.0);
        assert_eq!(on.note_snd_cursor, 2);
    }

    #[test]
    fn fire_scheduled_audio_never_rewinds_the_cursor() {
        let bgm = vec![BgmEvent {
            time_ms: 500.0,
            keysound: 1,
        }];
        let game = Game::new(chart_with_bgm(bgm));
        let bank = SampleBank::silent();
        let mut state = LoopState::new(Vec::new());
        fire_scheduled_audio(&bank, &opts(false), &mut state, &game, 600.0);
        assert_eq!(state.bgm_cursor, 1);
        // A later call at an earlier `now` still doesn't rewind because the
        // cursor is monotonic — the event has already been consumed.
        fire_scheduled_audio(&bank, &opts(false), &mut state, &game, 100.0);
        assert_eq!(state.bgm_cursor, 1);
    }

    // ---------- handle_key ----------

    fn ready_game() -> Game {
        // Two notes on lane 0 well past the countdown window.
        let notes = vec![
            Note {
                time_ms: 3000.0,
                lane: 0,
                hit: false,
                keysound: Some(1),
                end_ms: None,
                held_since: None,
            },
            Note {
                time_ms: 3200.0,
                lane: 0,
                hit: false,
                keysound: Some(2),
                end_ms: None,
                held_since: None,
            },
        ];
        Game::new(base_chart(4, notes))
    }

    #[test]
    fn shared_lane_long_note_waits_for_the_last_key_release() {
        [('f', 'j'), ('j', 'f')]
            .into_iter()
            .for_each(|(held, other)| {
                [4000.0, 5000.0, 5100.0].into_iter().for_each(|release_ms| {
                    let note = Note {
                        time_ms: 3000.0,
                        end_ms: Some(5000.0),
                        lane: 2,
                        ..Note::default()
                    };
                    let mut game = Game::new(base_chart(5, vec![note]));
                    let mut state = LoopState::new(Vec::new());
                    let bank = SampleBank::silent();
                    let options = opts(false);
                    let events = [
                        (held, KeyEventKind::Press, 3000.0),
                        (other, KeyEventKind::Press, 3500.0),
                        (other, KeyEventKind::Repeat, 3550.0),
                        (other.to_ascii_uppercase(), KeyEventKind::Release, 3600.0),
                        (other, KeyEventKind::Release, 3700.0),
                        ('s', KeyEventKind::Press, 3800.0),
                        (held, KeyEventKind::Release, release_ms),
                        (held, KeyEventKind::Release, release_ms + 10.0),
                    ];
                    events
                        .into_iter()
                        .enumerate()
                        .for_each(|(index, (key, kind, now))| {
                            handle_key(
                                KeyCode::Char(key),
                                kind,
                                &mut game,
                                &bank,
                                &options,
                                &mut state,
                                now,
                            );
                            if index < 6 {
                                assert!(!game.chart.notes[0].hit);
                                assert_eq!(game.chart.notes[0].held_since, Some(3000.0));
                                assert_eq!(
                                    (game.perfect, game.great, game.good, game.miss),
                                    (1, 0, 0, 0)
                                );
                            } else {
                                assert!(game.chart.notes[0].hit);
                                assert_eq!(game.chart.notes[0].held_since, None);
                                let expected = match release_ms {
                                    4000.0 => (1, 0, 0, 1),
                                    5000.0 => (2, 0, 0, 0),
                                    _ => (1, 0, 1, 0),
                                };
                                assert_eq!(
                                    (game.perfect, game.great, game.good, game.miss),
                                    expected
                                );
                                assert_eq!(game.flash.last_judgment_at, release_ms);
                            }
                        });
                });
            });
    }

    #[test]
    fn quitting_stops_active_audio_before_results_without_further_input() {
        [KeyCode::Esc, KeyCode::Char('q'), KeyCode::Char('Q')]
            .into_iter()
            .for_each(|code| {
                let mut game = ready_game();
                let mut state = LoopState::new(Vec::new());
                let bank = SampleBank::silent();
                let (sinks, mut outputs): (Vec<_>, Vec<_>) = (0..2)
                    .map(|_| {
                        let (sink, mut output) = rodio::Sink::new_idle();
                        sink.append(rodio::buffer::SamplesBuffer::new(
                            1,
                            44_100,
                            vec![0.5; 44_100 * 60],
                        ));
                        assert_eq!(output.next(), Some(0.5));
                        (sink, output)
                    })
                    .unzip();
                assert!(handle_key(
                    code,
                    KeyEventKind::Press,
                    &mut game,
                    &bank,
                    &opts(false),
                    &mut state,
                    3000.0,
                ));
                assert!(state.quit);

                after_playback((bank, sinks), || {
                    outputs.iter_mut().for_each(|output| {
                        assert!(output.by_ref().skip(1000).take(100).all(|s| s == 0.0));
                        assert!(output.by_ref().take(100).all(|s| s == 0.0));
                    });
                });
            });
    }

    #[test]
    fn handle_key_esc_marks_quit_and_returns_true() {
        let mut g = ready_game();
        let mut s = LoopState::new(Vec::new());
        let bank = SampleBank::silent();
        // Give the game a start that pretends we're already past countdown.
        let start = Instant::now() - Duration::from_millis(3000);
        let handled = handle_key(
            KeyCode::Esc,
            KeyEventKind::Press,
            &mut g,
            &bank,
            &opts(false),
            &mut s,
            elapsed_ms(start),
        );
        assert!(handled, "Esc should short-circuit the input loop");
        assert!(s.quit);
    }

    #[test]
    fn handle_key_q_uppercase_or_lowercase_both_quit() {
        for code in [KeyCode::Char('q'), KeyCode::Char('Q')] {
            let mut g = ready_game();
            let mut s = LoopState::new(Vec::new());
            let bank = SampleBank::silent();
            let start = Instant::now();
            let handled = handle_key(
                code,
                KeyEventKind::Press,
                &mut g,
                &bank,
                &opts(false),
                &mut s,
                elapsed_ms(start),
            );
            assert!(handled);
            assert!(s.quit);
        }
    }

    #[test]
    fn handle_key_ignores_letters_during_countdown() {
        let mut g = ready_game();
        let mut s = LoopState::new(Vec::new());
        let bank = SampleBank::silent();
        // Fresh start ⇒ elapsed_ms ≈ 0 ≪ countdown_ms (2000).
        let start = Instant::now();
        let handled = handle_key(
            KeyCode::Char('S'),
            KeyEventKind::Press,
            &mut g,
            &bank,
            &opts(false),
            &mut s,
            elapsed_ms(start),
        );
        assert!(!handled);
        assert!(!s.quit);
        // No note has been consumed.
        assert_eq!(g.perfect + g.great + g.good, 0);
    }

    #[test]
    fn handle_key_registers_a_perfect_hit_when_pressed_on_beat() {
        let mut g = ready_game();
        let mut s = LoopState::new(Vec::new());
        let bank = SampleBank::silent();
        // Pretend we're currently at 3000 ms so 'S' (lane 0) is spot-on.
        let start = Instant::now() - Duration::from_millis(3000);
        let handled = handle_key(
            KeyCode::Char('S'),
            KeyEventKind::Press,
            &mut g,
            &bank,
            &opts(false),
            &mut s,
            elapsed_ms(start),
        );
        assert!(!handled, "note hits should not exit the loop");
        assert_eq!(g.perfect, 1);
        assert_eq!(g.combo, 1);
    }

    #[test]
    fn handle_key_treats_unbound_letters_as_noops() {
        let mut g = ready_game();
        let mut s = LoopState::new(Vec::new());
        let bank = SampleBank::silent();
        let start = Instant::now() - Duration::from_millis(3000);
        let handled = handle_key(
            KeyCode::Char('X'),
            KeyEventKind::Press,
            &mut g,
            &bank,
            &opts(false),
            &mut s,
            elapsed_ms(start),
        );
        assert!(!handled);
        assert_eq!(g.perfect + g.great + g.good + g.miss, 0);
    }

    #[test]
    fn handle_key_ignores_non_char_keys_like_arrows() {
        let mut g = ready_game();
        let mut s = LoopState::new(Vec::new());
        let bank = SampleBank::silent();
        let start = Instant::now() - Duration::from_millis(3000);
        for code in [
            KeyCode::Up,
            KeyCode::Down,
            KeyCode::Left,
            KeyCode::Right,
            KeyCode::Enter,
            KeyCode::Tab,
        ] {
            let handled = handle_key(
                code,
                KeyEventKind::Press,
                &mut g,
                &bank,
                &opts(false),
                &mut s,
                elapsed_ms(start),
            );
            assert!(!handled);
        }
        assert!(!s.quit);
    }
}
