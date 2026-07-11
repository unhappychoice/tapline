use crate::{audio, chart, cli::Args, game, render};
use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute, terminal,
};
use std::io::Stdout;
use std::time::{Duration, Instant};

pub struct PlayOptions {
    pub countdown_ms: f64,
    pub synth_mode: bool,
    pub auto_ks: bool,
    pub audio_lead_ms: f64,
}

impl PlayOptions {
    pub fn from_args(args: &Args, chart: &chart::Chart) -> Self {
        Self {
            countdown_ms: args.countdown_ms,
            synth_mode: args.synth || chart.wav_paths.is_empty(),
            auto_ks: args.auto_ks,
            audio_lead_ms: args.audio_lead_ms,
        }
    }
}

pub fn play_chart(out: &mut Stdout, args: &Args, chart: chart::Chart) -> Result<()> {
    let opts = PlayOptions::from_args(args, &chart);
    let bank = if args.no_audio {
        audio::SampleBank::silent()
    } else {
        audio::SampleBank::new(&chart.wav_paths)
    };
    let mut game = game::Game::new(chart);
    execute!(out, terminal::Clear(terminal::ClearType::All))?;
    run_game(out, &mut game, &bank, &opts)?;
    show_result(out, &game)
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
        if now >= game.chart.duration_ms {
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
    bgm_cursor: usize,
    note_snd_cursor: usize,
    prev_mode: u8,
    auto_notes: Vec<(f64, usize, Option<u32>)>,
}

impl LoopState {
    fn new(auto_notes: Vec<(f64, usize, Option<u32>)>) -> Self {
        Self {
            quit: false,
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
    fire_scheduled_audio(bank, opts, state, game, now);
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
        bank.play(game.chart.bgm[state.bgm_cursor].keysound);
        state.bgm_cursor += 1;
    }
    while state.note_snd_cursor < state.auto_notes.len()
        && state.auto_notes[state.note_snd_cursor].0 <= horizon
    {
        let (_, lane, ks) = state.auto_notes[state.note_snd_cursor];
        bank.play_hit(lane, ks, opts.synth_mode);
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
        let Event::Key(k) = event::read()? else {
            continue;
        };
        if k.kind == KeyEventKind::Repeat {
            continue;
        }
        if handle_key(k.code, k.kind, game, bank, opts, state, start) {
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
    start: Instant,
) -> bool {
    match code {
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') if kind == KeyEventKind::Press => {
            state.quit = true;
            true
        }
        KeyCode::Char(c) => {
            let now = elapsed_ms(start);
            if now < opts.countdown_ms {
                return false;
            }
            let Some(lane) = lane_for_key(c, &game.chart.keys) else {
                return false;
            };
            match kind {
                KeyEventKind::Press => {
                    let ks = game.hit(lane, now);
                    if !opts.auto_ks {
                        bank.play_hit(lane, ks, opts.synth_mode);
                    }
                }
                KeyEventKind::Release => {
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
                end_ms: None, held_since: None,            },
            Note {
                time_ms: 500.0,
                lane: 1,
                hit: false,
                keysound: Some(2),
                end_ms: None, held_since: None,            },
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
                end_ms: None, held_since: None,            },
            Note {
                time_ms: 500.0,
                lane: 1,
                hit: false,
                keysound: Some(2),
                end_ms: None, held_since: None,            },
            Note {
                time_ms: 800.0,
                lane: 2,
                hit: false,
                keysound: Some(3),
                end_ms: None, held_since: None,            },
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
    fn play_options_from_args_forces_synth_when_no_wavs_are_loaded() {
        use crate::cli::Args;
        use clap::Parser;
        let args = Args::parse_from(["tapline"]);
        let chart = base_chart(4, vec![]);
        let opts = PlayOptions::from_args(&args, &chart);
        assert!(opts.synth_mode, "no WAV paths → synth mode auto-on");
    }

    #[test]
    fn play_options_from_args_carries_over_the_audio_lead() {
        use crate::cli::Args;
        use clap::Parser;
        let args = Args::parse_from(["tapline", "--audio-lead-ms", "72"]);
        let chart = base_chart(4, vec![]);
        let opts = PlayOptions::from_args(&args, &chart);
        assert_eq!(opts.audio_lead_ms, 72.0);
    }

    #[test]
    fn play_options_from_args_carries_over_the_auto_ks_flag() {
        use crate::cli::Args;
        use clap::Parser;
        let args = Args::parse_from(["tapline", "--auto-ks"]);
        let chart = base_chart(4, vec![]);
        let opts = PlayOptions::from_args(&args, &chart);
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
                end_ms: None, held_since: None,            },
            Note {
                time_ms: 400.0,
                lane: 1,
                hit: false,
                keysound: Some(2),
                end_ms: None, held_since: None,            },
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
                end_ms: None, held_since: None,            },
            Note {
                time_ms: 3200.0,
                lane: 0,
                hit: false,
                keysound: Some(2),
                end_ms: None, held_since: None,            },
        ];
        Game::new(base_chart(4, notes))
    }

    #[test]
    fn handle_key_esc_marks_quit_and_returns_true() {
        let mut g = ready_game();
        let mut s = LoopState::new(Vec::new());
        let bank = SampleBank::silent();
        // Give the game a start that pretends we're already past countdown.
        let start = Instant::now() - Duration::from_millis(3000);
        let handled = handle_key(KeyCode::Esc, KeyEventKind::Press, &mut g, &bank, &opts(false), &mut s, start);
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
            let handled = handle_key(code, KeyEventKind::Press, &mut g, &bank, &opts(false), &mut s, start);
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
            start,
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
            start,
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
            start,
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
            let handled = handle_key(code, KeyEventKind::Press, &mut g, &bank, &opts(false), &mut s, start);
            assert!(!handled);
        }
        assert!(!s.quit);
    }
}
