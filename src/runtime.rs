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
        render::draw_intro(out, game, opts.countdown_ms - now, bank.enabled)?;
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
        if k.kind == KeyEventKind::Release {
            continue;
        }
        if handle_key(k.code, game, bank, opts, state, start) {
            return Ok(());
        }
        let _ = out;
    }
}

fn handle_key(
    code: KeyCode,
    game: &mut game::Game,
    bank: &audio::SampleBank,
    opts: &PlayOptions,
    state: &mut LoopState,
    start: Instant,
) -> bool {
    match code {
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') => {
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
            let ks = game.hit(lane, now);
            if !opts.auto_ks {
                bank.play_hit(lane, ks, opts.synth_mode);
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
}
