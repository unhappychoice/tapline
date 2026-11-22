use crate::bga::Presentation;
use crate::chart::VolumeTarget;
use crate::render::{AudioMode, ResultDestination};
use crate::screen::Screen;
use crate::{audio, chart, cli::Args, game, render};
use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute, queue, terminal,
};
use std::collections::HashSet;
use std::io::Stdout;
use std::time::{Duration, Instant};

#[derive(Debug, PartialEq, Eq)]
pub enum ResultAction {
    Retry,
    Dismiss,
}

pub struct PlayOptions {
    pub countdown_ms: f64,
    pub synth_mode: bool,
    pub no_audio: bool,
    pub auto_ks: bool,
    pub audio_lead_ms: f64,
}

impl PlayOptions {
    pub fn from_args(args: &Args) -> Self {
        Self {
            countdown_ms: args.countdown_ms,
            synth_mode: args.synth,
            no_audio: args.no_audio,
            auto_ks: args.auto_ks,
            audio_lead_ms: args.audio_lead_ms,
        }
    }
}

pub fn play_chart(
    out: &mut Stdout,
    args: &Args,
    chart: chart::Chart,
    identity: Option<crate::results::Identity>,
) -> Result<()> {
    play_chart_with_destination(out, args, chart, identity, ResultDestination::Exit)
}

pub fn play_chart_with_destination(
    out: &mut Stdout,
    args: &Args,
    chart: chart::Chart,
    identity: Option<crate::results::Identity>,
    destination: ResultDestination,
) -> Result<()> {
    let audio = audio::ChartAudio::load(&chart, args.no_audio);
    while play_loaded_attempt(out, args, &chart, &audio, identity.as_ref(), destination)?
        == ResultAction::Retry
    {}
    Ok(())
}

pub fn play_attempt(
    out: &mut Stdout,
    args: &Args,
    chart: &chart::Chart,
    identity: Option<&crate::results::Identity>,
    destination: ResultDestination,
) -> Result<ResultAction> {
    let audio = audio::ChartAudio::load(chart, args.no_audio);
    play_loaded_attempt(out, args, chart, &audio, identity, destination)
}

/// Play one attempt using audio prepared for this loaded chart.
pub fn play_loaded_attempt(
    out: &mut Stdout,
    args: &Args,
    chart: &chart::Chart,
    audio: &audio::ChartAudio,
    identity: Option<&crate::results::Identity>,
    destination: ResultDestination,
) -> Result<ResultAction> {
    let opts = PlayOptions::from_args(args);
    let bank = audio.start_attempt();
    let duration_ms =
        bank.chart_duration_ms(chart, opts.auto_ks, opts.synth_mode, opts.audio_lead_ms);
    let mut chart = chart.clone();
    chart.duration_ms = duration_ms;
    let mut game = game::Game::new(chart);
    execute!(out, terminal::Clear(terminal::ClearType::All))?;
    run_game(out, &mut game, &bank, &opts)?;
    after_playback(bank, || {
        let progress = save_result(identity, &game, args.auto_ks);
        show_result(out, &game, &progress, destination)
    })
}

fn save_result(
    identity: Option<&crate::results::Identity>,
    game: &game::Game,
    auto_ks: bool,
) -> String {
    let Some(identity) = identity else {
        return "No chart ID for saving".into();
    };
    let Some(path) = crate::results::data_path() else {
        return "No results directory".into();
    };
    persist_result(&path, identity, game, auto_ks)
}

fn persist_result(
    path: &std::path::Path,
    identity: &crate::results::Identity,
    game: &game::Game,
    auto_ks: bool,
) -> String {
    use crate::results::{self, UpdateError};

    match results::update(path, identity, game, auto_ks) {
        Ok(previous) => render::format_progress(game, previous.as_ref()),
        Err(UpdateError::Read) => "Cannot read saved results".into(),
        Err(UpdateError::Lock) => "Storage error: result unsaved".into(),
        Err(UpdateError::Write(previous)) => {
            let progress = render::format_progress(game, previous.as_ref());
            format!("Storage error: result unsaved | {progress}")
        }
    }
}

fn after_playback<T, R>(playback: T, show_result: impl FnOnce() -> R) -> R {
    drop(playback);
    show_result()
}

const FRAME_DT: Duration = Duration::from_millis(16);
const MAX_INPUT_EVENTS_PER_FRAME: usize = 32;

fn run_game(
    out: &mut Stdout,
    game: &mut game::Game,
    bank: &audio::SampleBank,
    opts: &PlayOptions,
) -> Result<()> {
    let (cols, rows) = terminal::size()?;
    let visible = render::bga_panel_visible(cols, rows, game.chart().lane_count);
    let mut bga = Presentation::new(game.chart(), visible);
    let start = Instant::now();
    let mut next_frame = Instant::now() + FRAME_DT;
    let mut state = LoopState::new(build_auto_notes(game, opts));

    while !state.quit {
        let (cols, rows) = terminal::size()?;
        let now = state.update_size(bank, game, elapsed_ms(start), cols, rows);
        if advance_frame(game, bank, opts, &mut state, now) {
            break;
        }
        let visible = render::bga_panel_visible(cols, rows, game.chart().lane_count);
        bga.prepare_deferred(game.chart(), visible, Presentation::load_assets);
        draw_frame(out, game, bank, opts, &mut state, &bga, now)?;
        pump_input(out, game, bank, opts, &mut state, start, next_frame)?;
        next_frame = advance_deadline(next_frame);
    }
    Ok(())
}

struct LoopState {
    quit: bool,
    pressed_keys: HashSet<char>,
    deferred_releases: Vec<usize>,
    bgm_cursor: usize,
    note_snd_cursor: usize,
    prev_mode: u8,
    auto_notes: Vec<(f64, usize, Option<u32>)>,
    screen: Screen,
    intro_artwork: render::IntroArtwork,
    paused: bool,
    clock_ms: f64,
    last_wall_ms: f64,
    audio_end_ms: f64,
}

impl LoopState {
    fn advance_completion(&self, game: &mut game::Game, now: f64) -> bool {
        !self.paused && now >= self.audio_end_ms && game.advance_completion(now)
    }

    fn track_playback(&mut self, now: f64, duration: Option<Duration>) {
        if let Some(duration) = duration {
            let end = now + duration.as_secs_f64() * 1000.0 + 32.0;
            self.audio_end_ms = self.audio_end_ms.max(end);
        }
    }

    fn update_size(
        &mut self,
        bank: &audio::SampleBank,
        game: &mut game::Game,
        wall_ms: f64,
        cols: u16,
        rows: u16,
    ) -> f64 {
        let paused = !render::playfield_fits(cols, rows, game.chart().lane_count);
        if !self.paused && !paused {
            self.clock_ms += wall_ms - self.last_wall_ms;
        }
        self.last_wall_ms = wall_ms;
        if paused != self.paused {
            bank.set_paused(paused);
        }
        if self.paused && !paused && !self.deferred_releases.is_empty() {
            game.check_misses(self.clock_ms);
            self.deferred_releases
                .drain(..)
                .for_each(|lane| game.release(lane, self.clock_ms));
        }
        self.paused = paused;
        self.clock_ms
    }

    fn new(auto_notes: Vec<(f64, usize, Option<u32>)>) -> Self {
        Self {
            quit: false,
            pressed_keys: HashSet::new(),
            deferred_releases: Vec::new(),
            bgm_cursor: 0,
            note_snd_cursor: 0,
            prev_mode: 0,
            auto_notes,
            screen: Screen::new(),
            intro_artwork: render::IntroArtwork::default(),
            paused: false,
            clock_ms: 0.0,
            last_wall_ms: 0.0,
            audio_end_ms: 0.0,
        }
    }
}

fn build_auto_notes(game: &game::Game, opts: &PlayOptions) -> Vec<(f64, usize, Option<u32>)> {
    if !opts.auto_ks {
        return Vec::new();
    }
    let mut v: Vec<_> = game
        .chart()
        .notes
        .iter()
        .map(|n| (n.time_ms, n.lane, n.keysound))
        .collect();
    v.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    v
}

fn advance_frame(
    game: &mut game::Game,
    bank: &audio::SampleBank,
    opts: &PlayOptions,
    state: &mut LoopState,
    now: f64,
) -> bool {
    if !state.paused {
        fire_scheduled_audio(bank, opts, state, game, now);
    }
    state.advance_completion(game, now)
}

fn draw_frame(
    out: &mut Stdout,
    game: &mut game::Game,
    bank: &audio::SampleBank,
    opts: &PlayOptions,
    state: &mut LoopState,
    bga: &Presentation,
    now: f64,
) -> Result<()> {
    let (cols, rows) = terminal::size()?;
    state.screen.resize(cols, rows);
    let mode: u8 = if now < opts.countdown_ms { 1 } else { 2 };
    if mode != state.prev_mode {
        if mode == 2 {
            state.intro_artwork = render::IntroArtwork::default();
        }
        queue!(state.screen, terminal::Clear(terminal::ClearType::All))?;
        state.prev_mode = mode;
    }
    if state.paused {
        state.intro_artwork = render::IntroArtwork::default();
        render::draw_size_warning(&mut state.screen, game.chart().lane_count)?;
        state.prev_mode = 0;
    } else {
        draw_mode(state, game, bank, opts, bga, (cols, rows), now)?;
    }
    state.screen.present(out)?;
    Ok(())
}

fn draw_mode(
    state: &mut LoopState,
    game: &mut game::Game,
    bank: &audio::SampleBank,
    opts: &PlayOptions,
    bga: &Presentation,
    size: (u16, u16),
    now: f64,
) -> Result<()> {
    if now < opts.countdown_ms {
        let audio_mode = match (opts.no_audio, opts.synth_mode) {
            (true, _) => AudioMode::Silent,
            (_, true) => AudioMode::Synth,
            _ => AudioMode::Samples,
        };
        state.intro_artwork.prepare(game.chart(), size);
        return render::draw_prepared_intro(
            &mut state.screen,
            game,
            opts.countdown_ms - now,
            audio_mode,
            bank.enabled,
            (bank.sample_count(), bank.decode_failures),
            &state.intro_artwork,
        );
    }
    game.check_misses(now);
    render::draw(&mut state.screen, game, bga, now)
}

fn fire_scheduled_audio(
    bank: &audio::SampleBank,
    opts: &PlayOptions,
    state: &mut LoopState,
    game: &game::Game,
    now: f64,
) {
    let horizon = now + opts.audio_lead_ms;
    loop {
        let bgm = game
            .chart()
            .bgm
            .get(state.bgm_cursor)
            .filter(|event| event.time_ms <= horizon);
        let note = state
            .auto_notes
            .get(state.note_snd_cursor)
            .filter(|note| note.0 <= horizon);
        // BGM dispatches first at equal timestamps.
        match (bgm, note) {
            (Some(event), note) if note.is_none_or(|note| event.time_ms <= note.0) => {
                let gain = game.volume.gain(VolumeTarget::Bgm, event.time_ms);
                state.track_playback(now, bank.start_sound(event.keysound, gain));
                state.bgm_cursor += 1;
            }
            (_, Some(&(time_ms, lane, ks))) => {
                let gain = game.volume.gain(VolumeTarget::Key, time_ms);
                state.track_playback(now, bank.start_hit(lane, ks, opts.synth_mode, gain));
                state.note_snd_cursor += 1;
            }
            _ => break,
        }
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
    process_frame_input(
        next_frame,
        |timeout| {
            if event::poll(timeout)? {
                Ok(Some(event::read()?))
            } else {
                Ok(None)
            }
        },
        |input| {
            let (cols, rows) = terminal::size()?;
            let now = state.update_size(bank, game, elapsed_ms(start), cols, rows);
            if matches!(input, Event::Resize(..)) {
                execute!(out, terminal::Clear(terminal::ClearType::All))?;
                state.screen.invalidate();
            }
            let Event::Key(k) = input else {
                return Ok(false);
            };
            Ok(k.kind != KeyEventKind::Repeat
                && handle_key(k.code, k.kind, game, bank, opts, state, now))
        },
    )
}

fn process_frame_input(
    next_frame: Instant,
    mut read_ready: impl FnMut(Duration) -> Result<Option<Event>>,
    mut handle_event: impl FnMut(Event) -> Result<bool>,
) -> Result<()> {
    for _ in 0..MAX_INPUT_EVENTS_PER_FRAME {
        let timeout = next_frame.saturating_duration_since(Instant::now());
        let Some(input) = read_ready(timeout)? else {
            break;
        };
        if handle_event(input)? {
            break;
        }
    }
    Ok(())
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
            let Some(lane) = lane_for_key(c, &game.chart().keys) else {
                return false;
            };
            match kind {
                KeyEventKind::Press if !state.paused => {
                    game.check_misses(now);
                    if game.gameplay_ended(now) {
                        return false;
                    }
                    state.pressed_keys.insert(c.to_ascii_uppercase());
                    let ks = game.hit(lane, now);
                    if !opts.auto_ks {
                        let gain = game.volume.gain(VolumeTarget::Key, now);
                        state.track_playback(now, bank.start_hit(lane, ks, opts.synth_mode, gain));
                    }
                }
                KeyEventKind::Release
                    if state.pressed_keys.remove(&c.to_ascii_uppercase())
                        && !state
                            .pressed_keys
                            .iter()
                            .any(|&key| lane_for_key(key, &game.chart().keys) == Some(lane)) =>
                {
                    if state.paused {
                        state.deferred_releases.push(lane);
                    } else {
                        game.check_misses(now);
                        game.release(lane, now);
                    }
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

fn show_result(
    out: &mut Stdout,
    game: &game::Game,
    progress: &str,
    destination: ResultDestination,
) -> Result<ResultAction> {
    wait_for_result_action(terminal::size()?, event::read, |cols, rows| {
        queue!(out, terminal::Clear(terminal::ClearType::All))?;
        render::draw_result_content(out, game, cols, rows, progress, destination)
    })
}

fn wait_for_result_action(
    (cols, rows): (u16, u16),
    mut read: impl FnMut() -> std::io::Result<Event>,
    mut draw: impl FnMut(u16, u16) -> Result<()>,
) -> Result<ResultAction> {
    draw(cols, rows)?;
    loop {
        match read()? {
            Event::Resize(cols, rows) => draw(cols, rows)?,
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                return Ok(match key.code {
                    KeyCode::Char('r' | 'R') => ResultAction::Retry,
                    _ => ResultAction::Dismiss,
                });
            }
            _ => {}
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
    fn overdue_frames_process_lane_input_and_escape() {
        let mut game = ready_game();
        let mut state = LoopState::new(Vec::new());
        let bank = SampleBank::silent();
        let options = opts(false);
        let mut keys = [KeyCode::Char('s'), KeyCode::Esc, KeyCode::Char('s')].into_iter();
        let deadline = Instant::now() - Duration::from_millis(40);

        process_frame_input(
            deadline,
            |timeout| {
                assert_eq!(timeout, Duration::ZERO);
                Ok(keys
                    .next()
                    .map(|code| Event::Key(event::KeyEvent::new(code, event::KeyModifiers::NONE))))
            },
            |input| {
                let Event::Key(key) = input else {
                    unreachable!();
                };
                Ok(handle_key(
                    key.code, key.kind, &mut game, &bank, &options, &mut state, 3000.0,
                ))
            },
        )
        .unwrap();

        assert_eq!(game.perfect, 1);
        assert!(state.quit);
        assert_eq!(keys.next(), Some(KeyCode::Char('s')));
    }

    #[test]
    fn busy_overdue_frames_bound_input_and_resume_on_the_next_frame() {
        let deadline = Instant::now() - Duration::from_millis(40);
        let mut handled = 0;
        for frame in 1..=2 {
            process_frame_input(
                deadline,
                |timeout| {
                    assert_eq!(timeout, Duration::ZERO);
                    Ok(Some(Event::FocusGained))
                },
                |_| {
                    handled += 1;
                    assert!(handled <= frame * MAX_INPUT_EVENTS_PER_FRAME);
                    Ok(false)
                },
            )
            .unwrap();
            assert_eq!(handled, frame * MAX_INPUT_EVENTS_PER_FRAME);
        }
    }

    #[test]
    fn on_time_frames_wait_for_input_until_the_deadline() {
        let deadline = Instant::now() + Duration::from_secs(60);
        let mut polls = 0;
        process_frame_input(
            deadline,
            |timeout| {
                assert!(timeout > Duration::ZERO && timeout <= Duration::from_secs(60));
                polls += 1;
                Ok(None)
            },
            |_| unreachable!(),
        )
        .unwrap();
        assert_eq!(polls, 1);
    }

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

    #[test]
    fn insufficient_dimensions_freeze_startup_and_resize_judgments() {
        let bank = SampleBank::silent();
        let mut game = Game::new(base_chart(
            14,
            vec![Note {
                time_ms: 3000.0,
                lane: 13,
                ..Note::default()
            }],
        ));
        let mut state = LoopState::new(Vec::new());
        [
            (5000.0, 80, 24, 0.0),
            (6000.0, 99, 24, 0.0),
            (9000.0, 99, 24, 3000.0),
            (9010.0, 80, 24, 3000.0),
            (19000.0, 80, 24, 3000.0),
            (20000.0, 99, 12, 3000.0),
            (21000.0, 99, 24, 3000.0),
        ]
        .into_iter()
        .for_each(|(wall, cols, rows, expected)| {
            let now = state.update_size(&bank, &mut game, wall, cols, rows);
            assert_eq!(now, expected);
            if !state.paused {
                game.check_misses(now);
            }
            if state.paused {
                handle_key(
                    KeyCode::Char('m'),
                    KeyEventKind::Press,
                    &mut game,
                    &bank,
                    &opts(false),
                    &mut state,
                    now,
                );
            }
            assert_eq!((game.perfect, game.miss), (0, 0));
            assert!(!game.chart().notes[0].hit);
        });
        let now = state.update_size(&bank, &mut game, 21010.0, 99, 24);
        handle_key(
            KeyCode::Char('m'),
            KeyEventKind::Press,
            &mut game,
            &bank,
            &opts(false),
            &mut state,
            now,
        );
        assert_eq!((game.perfect, game.miss), (1, 0));
    }

    #[test]
    fn revealing_bga_keeps_audio_and_input_on_time_while_loading() {
        use std::sync::mpsc;

        let times = [3000.0, 3100.0];
        let mut chart = base_chart(
            4,
            times
                .into_iter()
                .map(|time_ms| Note {
                    time_ms,
                    keysound: Some(1),
                    ..Note::default()
                })
                .collect(),
        );
        chart.bgm = times
            .into_iter()
            .map(|time_ms| BgmEvent {
                time_ms,
                keysound: 1,
            })
            .collect();
        let mut game = Game::new(chart);
        let bank = SampleBank::silent();
        let options = opts(true);
        let mut state = LoopState::new(build_auto_notes(&game, &options));
        let mut bga = Presentation::new(game.chart(), render::bga_panel_visible(40, 24, 4));
        let now = state.update_size(&bank, &mut game, 2900.0, 40, 24);
        assert_eq!(now, 2900.0);

        let (started, loading) = mpsc::channel();
        let (release, blocked) = mpsc::channel();
        let visible = render::bga_panel_visible(80, 24, 4);
        bga.prepare_deferred(game.chart(), visible, move |chart| {
            started.send(()).unwrap();
            blocked.recv_timeout(Duration::from_secs(5)).unwrap();
            Presentation::load_assets(chart)
        });
        loading.recv_timeout(Duration::from_secs(5)).unwrap();

        // Keep the loader held while two separate audio events and key presses are due.
        for (index, wall_ms) in times.into_iter().enumerate() {
            bga.prepare_deferred(game.chart(), visible, Presentation::load_assets);
            let now = state.update_size(&bank, &mut game, wall_ms - 1.0, 80, 24);
            fire_scheduled_audio(&bank, &options, &mut state, &game, now);
            assert_eq!((state.bgm_cursor, state.note_snd_cursor), (index, index));

            let now = state.update_size(&bank, &mut game, wall_ms, 80, 24);
            fire_scheduled_audio(&bank, &options, &mut state, &game, now);
            assert_eq!(
                (state.bgm_cursor, state.note_snd_cursor),
                (index + 1, index + 1)
            );
            handle_key(
                KeyCode::Char('s'),
                KeyEventKind::Press,
                &mut game,
                &bank,
                &options,
                &mut state,
                now,
            );
            assert_eq!((game.perfect, game.miss), (index as u32 + 1, 0));
        }
        release.send(()).unwrap();
    }

    #[test]
    fn resize_pause_reconciles_only_lanes_that_lose_their_last_binding() {
        let notes = [0, 1, 2].map(|lane| Note {
            time_ms: 3000.0,
            end_ms: Some(5000.0),
            lane,
            ..Note::default()
        });
        let mut game = Game::new(base_chart(5, notes.to_vec()));
        let mut state = LoopState::new(Vec::new());
        let bank = SampleBank::silent();
        let options = opts(false);
        let now = state.update_size(&bank, &mut game, 3000.0, 80, 24);
        for key in ['s', 'd', 'f', 'j'] {
            handle_key(
                KeyCode::Char(key),
                KeyEventKind::Press,
                &mut game,
                &bank,
                &options,
                &mut state,
                now,
            );
        }

        state.update_size(&bank, &mut game, 4000.0, 80, 24);
        let now = state.update_size(&bank, &mut game, 7000.0, 20, 24);
        for (key, kind) in [
            ('S', KeyEventKind::Release),
            ('F', KeyEventKind::Release),
            ('s', KeyEventKind::Press),
        ] {
            handle_key(
                KeyCode::Char(key),
                kind,
                &mut game,
                &bank,
                &options,
                &mut state,
                now,
            );
        }
        assert_eq!((game.perfect, game.miss), (3, 0));
        assert!(game.chart().notes.iter().all(|note| !note.hit));

        let now = state.update_size(&bank, &mut game, 9000.0, 80, 24);
        assert_eq!(now, 4000.0);
        assert_eq!(game.flash.last_judgment_at, now);
        assert_eq!((game.perfect, game.miss), (3, 1));
        assert!(game.chart().notes[0].hit);
        assert_eq!(game.chart().notes[1].held_since, Some(3000.0));
        assert_eq!(game.chart().notes[2].held_since, Some(3000.0));

        let now = state.update_size(&bank, &mut game, 10200.0, 80, 24);
        game.advance_completion(now);
        assert_eq!((game.perfect, game.miss), (5, 1));
    }

    pub(super) fn base_chart(lane_count: usize, notes: Vec<Note>) -> Chart {
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

    pub(super) fn opts(auto_ks: bool) -> PlayOptions {
        PlayOptions {
            countdown_ms: 2000.0,
            synth_mode: true,
            no_audio: false,
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
    fn overdue_final_bgm_plays_before_gameplay_completes() {
        let (bank, mut output) = audio::playback_test_bank();
        let options = PlayOptions {
            synth_mode: false,
            ..opts(false)
        };
        let mut chart = base_chart(
            4,
            vec![Note {
                time_ms: 3000.0,
                keysound: Some(2),
                ..Note::default()
            }],
        );
        chart.bgm.push(BgmEvent {
            time_ms: 4000.0,
            keysound: 2,
        });
        chart.duration_ms = bank.chart_duration_ms(&chart, false, false, 0.0);
        assert_eq!(chart.duration_ms, 4112.0);
        let mut game = Game::new(chart);
        let mut state = LoopState::new(Vec::new());
        game.hit(0, 3000.0);
        assert_eq!(game.perfect, 1);

        let now = state.update_size(&bank, &mut game, 3999.0, 80, 24);
        assert!(!advance_frame(&mut game, &bank, &options, &mut state, now));
        assert_eq!(state.bgm_cursor, 0);
        assert_eq!(output.next(), Some(0.0));

        let now = state.update_size(&bank, &mut game, 5000.0, 80, 24);
        assert!(game.gameplay_ended(now));
        assert!(!advance_frame(&mut game, &bank, &options, &mut state, now));
        assert_eq!(state.bgm_cursor, 1);
        assert!(output.by_ref().take(2048).any(|sample| sample > 0.0));
        assert_eq!(state.audio_end_ms, 5112.0);

        let now = state.update_size(&bank, &mut game, 5079.0, 80, 24);
        assert!(!advance_frame(&mut game, &bank, &options, &mut state, now));
        assert!(!game.completed);

        let now = state.update_size(&bank, &mut game, 5112.0, 80, 24);
        assert!(advance_frame(&mut game, &bank, &options, &mut state, now));
        assert!(game.completed);
    }

    #[test]
    fn completion_waits_for_an_invisible_keysound_played_by_a_manual_press() {
        let (bank, _output) = audio::playback_test_bank();
        let bank = bank
            .with_slices(
                &[(
                    1,
                    chart::SampleSlice {
                        start_ms: 2000.0,
                        duration_ms: None,
                    },
                )]
                .into(),
            )
            .with_layers(&[(3, vec![2, 1, 99])].into());
        for auto_ks in [false, true] {
            let mut chart = base_chart(
                4,
                vec![Note {
                    time_ms: 2000.0,
                    lane: 0,
                    keysound: Some(2),
                    ..Note::default()
                }],
            );
            chart.invisible_notes.push(chart::InvisibleNote {
                time_ms: 4000.0,
                lane: 0,
                keysound: 3,
            });
            let opts = PlayOptions {
                synth_mode: false,
                ..opts(auto_ks)
            };
            chart.duration_ms = bank.chart_duration_ms(&chart, auto_ks, false, 0.0);
            assert_eq!(chart.duration_ms, 6500.0);
            let mut game = Game::new(chart);
            let mut state = LoopState::new(build_auto_notes(&game, &opts));
            for now in [2000.0, 6400.0] {
                fire_scheduled_audio(&bank, &opts, &mut state, &game, now);
                handle_key(
                    KeyCode::Char('s'),
                    KeyEventKind::Press,
                    &mut game,
                    &bank,
                    &opts,
                    &mut state,
                    now,
                );
            }
            assert_eq!((game.perfect, game.miss), (1, 0));
            assert_eq!(state.advance_completion(&mut game, 6500.0), auto_ks);
            if !auto_ks {
                let audio_end = state.audio_end_ms;
                for now in [6500.0, 7500.0, 8500.0, 16_399.0] {
                    handle_key(
                        KeyCode::Char('s'),
                        KeyEventKind::Press,
                        &mut game,
                        &bank,
                        &opts,
                        &mut state,
                        now,
                    );
                    assert_eq!(state.audio_end_ms, audio_end);
                    assert!(!game.completed);
                }
                assert!(!state.advance_completion(&mut game, 16_399.0));
                assert!(!game.completed);
                assert!(state.advance_completion(&mut game, 16_432.0));
            }
        }
    }

    #[test]
    fn lane_presses_can_judge_pending_notes_at_chart_end() {
        let mut chart = ready_game().chart().clone();
        chart.duration_ms = 3000.0;
        let mut game = Game::new(chart);
        let bank = SampleBank::silent();
        let mut state = LoopState::new(Vec::new());
        handle_key(
            KeyCode::Char('s'),
            KeyEventKind::Press,
            &mut game,
            &bank,
            &opts(false),
            &mut state,
            3000.0,
        );
        assert_eq!(game.perfect, 1);
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
    fn later_bgm_replaces_automatic_keysound_in_one_or_two_frames() {
        for frames in [&[1010.0][..], &[1000.0, 1010.0]] {
            assert_shared_sample_gain(1010.0, frames, 1.0);
        }
    }

    #[test]
    fn simultaneous_automatic_keysound_replaces_bgm() {
        assert_shared_sample_gain(1000.0, &[1000.0], 128.0 / 255.0);
    }

    fn assert_shared_sample_gain(bgm_time: f64, frames: &[f64], expected_gain: f32) {
        let mut chart = base_chart(
            4,
            vec![Note {
                time_ms: 1000.0,
                keysound: Some(2),
                ..Note::default()
            }],
        );
        chart.bgm.push(BgmEvent {
            time_ms: bgm_time,
            keysound: 2,
        });
        chart.volume_events.push(crate::chart::VolumeEvent {
            time_ms: 1000.0,
            target: VolumeTarget::Key,
            volume: 128,
        });
        let game = Game::new(chart);
        let options = PlayOptions {
            synth_mode: false,
            ..opts(true)
        };
        let mut state = LoopState::new(build_auto_notes(&game, &options));
        let (bank, mut output) = audio::playback_test_bank();
        for &now in frames {
            fire_scheduled_audio(&bank, &options, &mut state, &game, now);
        }
        // The mixer can emit startup silence and a few buffered samples from the old voice.
        let surviving_sample = output
            .by_ref()
            .take(2048)
            .filter(|&sample| sample != 0.0)
            .last();
        assert_eq!(
            surviving_sample,
            Some(0.25 * expected_gain),
            "frames: {frames:?}"
        );
    }

    #[test]
    fn scheduled_audio_plays_during_countdown_at_each_lead_time() {
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
            for (now, expected) in [
                (1919.0, 0),
                (1920.0, 1),
                (1936.0, 1),
                (1952.0, 2),
                (1968.0, 2),
                (1984.0, 3),
                (2000.0, 3),
            ] {
                assert!(!advance_frame(&mut game, &bank, &options, &mut state, now));
                assert_eq!(state.bgm_cursor, expected, "BGM at {now} ms");
                assert_eq!(
                    state.note_snd_cursor,
                    if auto_ks { expected } else { 0 },
                    "keysounds at {now} ms (auto_ks={auto_ks})"
                );
                assert_eq!(game.miss, 0);
                assert!(game.chart().notes.iter().all(|note| !note.hit));
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
    fn last_key_release_judges_both_overlapping_bms_long_notes() {
        let mut chart = crate::bms::load_bytes(
            std::path::Path::new("overlap.bms"),
            b"#BPM 120\n#LNMODE 1\n#LNOBJ 03\n#00151:01000100\n#00111:00020003\n",
            0.0,
            0,
        )
        .unwrap();
        chart.keys[0] = vec!['F', 'J'];
        let mut game = Game::new(chart);
        let mut state = LoopState::new(Vec::new());
        let bank = SampleBank::silent();
        let options = PlayOptions {
            countdown_ms: 0.0,
            ..opts(false)
        };
        let mut send_key = |game: &mut Game, key, kind, now| {
            handle_key(
                KeyCode::Char(key),
                kind,
                game,
                &bank,
                &options,
                &mut state,
                now,
            );
        };

        send_key(&mut game, 'f', KeyEventKind::Press, 2000.0);
        send_key(&mut game, 'j', KeyEventKind::Press, 2500.0);
        send_key(&mut game, 'f', KeyEventKind::Release, 2600.0);
        assert_eq!((game.perfect, game.miss), (2, 0));
        assert!(game
            .chart()
            .notes
            .iter()
            .all(|note| note.held_since.is_some()));

        send_key(&mut game, 'j', KeyEventKind::Release, 2750.0);
        assert_eq!((game.perfect, game.miss), (2, 2));
        assert!(game
            .chart()
            .notes
            .iter()
            .all(|note| note.hit && note.held_since.is_none()));
        game.check_misses(3500.0);
        assert_eq!((game.perfect, game.miss), (2, 2));
    }

    #[test]
    fn shared_lane_long_note_waits_for_the_last_key_release() {
        [
            (5, 2, 'f', 'j'),
            (5, 2, 'j', 'f'),
            (6, 5, 'a', ';'),
            (6, 5, ';', 'A'),
            (8, 7, 'a', ';'),
            (8, 7, ';', 'A'),
        ]
        .into_iter()
        .for_each(|(count, lane, held, other)| {
            [4000.0, 5000.0, 5100.0].into_iter().for_each(|release_ms| {
                let note = Note {
                    time_ms: 3000.0,
                    end_ms: Some(5000.0),
                    lane,
                    ..Note::default()
                };
                let mut game = Game::new(base_chart(count, vec![note]));
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
                            assert!(!game.chart().notes[0].hit);
                            assert_eq!(game.chart().notes[0].held_since, Some(3000.0));
                            assert_eq!(
                                (game.perfect, game.great, game.good, game.miss),
                                (1, 0, 0, 0)
                            );
                        } else {
                            assert!(game.chart().notes[0].hit);
                            assert_eq!(game.chart().notes[0].held_since, None);
                            let expected = match release_ms {
                                4000.0 => (1, 0, 0, 1),
                                5000.0 => (2, 0, 0, 0),
                                _ => (1, 0, 1, 0),
                            };
                            assert_eq!((game.perfect, game.great, game.good, game.miss), expected);
                            assert_eq!(game.flash.last_judgment_at, release_ms);
                        }
                    });
            });
        });
    }

    #[test]
    fn remapped_scratch_holds_until_both_aliases_are_released() {
        let lanes = chart::Lane::sides(6, Some(7), true, false);
        let mut chart = base_chart(
            lanes.len(),
            vec![Note {
                time_ms: 3000.0,
                end_ms: Some(5000.0),
                lane: 5,
                ..Note::default()
            }],
        );
        chart.lanes = lanes;
        let path = std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/keymap.json"
        ));
        crate::cli::Keymap::load(Some(path))
            .unwrap()
            .apply(&mut chart);
        let mut game = Game::new(chart);
        let mut state = LoopState::new(Vec::new());
        let bank = SampleBank::silent();
        let options = opts(false);
        let mut key = |game: &mut Game, c, kind, now| {
            handle_key(
                KeyCode::Char(c),
                kind,
                game,
                &bank,
                &options,
                &mut state,
                now,
            );
        };

        key(&mut game, 'a', KeyEventKind::Press, 3000.0);
        assert_eq!(game.perfect, 0);
        key(&mut game, 'z', KeyEventKind::Press, 3000.0);
        key(&mut game, '/', KeyEventKind::Press, 3200.0);
        key(&mut game, 'Z', KeyEventKind::Release, 3400.0);
        assert_eq!(game.chart().notes[0].held_since, Some(3000.0));
        key(&mut game, '/', KeyEventKind::Release, 5000.0);
        assert_eq!((game.perfect, game.miss), (2, 0));
    }

    #[test]
    fn double_play_scratch_long_notes_release_independently() {
        [12, 16].into_iter().for_each(|count| {
            let lanes = [count / 2 - 1, count - 1];
            let notes = lanes.map(|lane| Note {
                time_ms: 3000.0,
                end_ms: Some(5000.0),
                lane,
                ..Note::default()
            });
            let mut game = Game::new(base_chart(count, notes.to_vec()));
            let mut state = LoopState::new(Vec::new());
            let bank = SampleBank::silent();
            let options = opts(false);
            [
                ('A', KeyEventKind::Press, 3000.0),
                (';', KeyEventKind::Press, 3000.0),
                ('a', KeyEventKind::Release, 4000.0),
            ]
            .into_iter()
            .for_each(|(key, kind, now)| {
                handle_key(
                    KeyCode::Char(key),
                    kind,
                    &mut game,
                    &bank,
                    &options,
                    &mut state,
                    now,
                );
            });
            assert!(game.chart().notes[0].hit);
            assert_eq!(game.chart().notes[1].held_since, Some(3000.0));
            assert_eq!((game.perfect, game.miss), (2, 1));
            handle_key(
                KeyCode::Char(';'),
                KeyEventKind::Release,
                &mut game,
                &bank,
                &options,
                &mut state,
                5000.0,
            );
            assert!(game.chart().notes[1].hit);
            assert_eq!((game.perfect, game.miss), (3, 1));
        });
    }

    #[test]
    fn quitting_stops_active_audio_before_results_without_further_input() {
        [KeyCode::Esc, KeyCode::Char('q'), KeyCode::Char('Q')]
            .into_iter()
            .for_each(|code| {
                let mut game = ready_game();
                let mut state = LoopState::new(Vec::new());
                let (bank, mut output) = audio::playback_test_bank();
                assert!(bank.start_sound(2, 1.0).is_some());
                assert!(bank.start_hit(0, None, true, 1.0).is_some());
                assert!(output.by_ref().take(2048).any(|sample| sample != 0.0));
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

                after_playback(bank, || {
                    assert!(output.skip(1000).take(100).all(|sample| sample == 0.0));
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
    fn lane_press_applies_only_misses_strictly_past_the_deadline() {
        for (now, misses, combo) in [(1160.0, 0, 2), (1162.0, 1, 1)] {
            let notes = [(800.0, 0), (1000.0, 0), (1165.0, 1)].map(|(time_ms, lane)| Note {
                time_ms,
                lane,
                ..Note::default()
            });
            let mut game = Game::new(base_chart(4, notes.to_vec()));
            let mut state = LoopState::new(Vec::new());
            let bank = SampleBank::silent();
            let options = PlayOptions {
                countdown_ms: 0.0,
                ..opts(false)
            };
            game.hit(0, 800.0);
            game.check_misses(1158.0);
            let score = game.score;
            handle_key(
                KeyCode::Char('d'),
                KeyEventKind::Press,
                &mut game,
                &bank,
                &options,
                &mut state,
                now,
            );
            assert_eq!((game.perfect, game.miss, game.combo), (2, misses, combo));
            assert_eq!(game.score - score, game::Judgment::Perfect.points() + combo);
        }
    }

    #[test]
    fn lane_release_applies_overdue_charge_note_tail_once() {
        let note = Note {
            time_ms: 3000.0,
            end_ms: Some(5000.0),
            ..Note::default()
        };
        let mut game = Game::new(base_chart(4, vec![note]));
        let mut state = LoopState::new(Vec::new());
        let bank = SampleBank::silent();
        let options = opts(false);
        handle_key(
            KeyCode::Char('s'),
            KeyEventKind::Press,
            &mut game,
            &bank,
            &options,
            &mut state,
            3000.0,
        );
        game.check_misses(5160.0);
        assert_eq!(game.perfect, 1);
        handle_key(
            KeyCode::Char('s'),
            KeyEventKind::Release,
            &mut game,
            &bank,
            &options,
            &mut state,
            5162.0,
        );
        game.check_misses(5170.0);
        assert_eq!((game.perfect, game.miss, game.combo), (2, 0, 2));
        assert!(game.chart().notes[0].hit);
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

#[cfg(test)]
mod result_tests {
    use super::*;

    #[test]
    fn result_redraws_at_each_size_until_a_key_press() {
        use crossterm::event::{KeyEvent, KeyModifiers};

        let game = game::Game::new(chart::built_in(120.0, 0.0));
        let mut events = [
            Event::Resize(80, 24),
            Event::Key(KeyEvent::new_with_kind(
                KeyCode::Esc,
                KeyModifiers::NONE,
                KeyEventKind::Release,
            )),
            Event::Resize(29, 5),
            Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            Event::Resize(80, 24),
        ]
        .into_iter();
        let mut frames = Vec::new();
        let action = wait_for_result_action(
            (80, 5),
            || Ok(events.next().expect("expected a dismissal key press")),
            |cols, rows| {
                let mut frame = Vec::new();
                render::draw_result_content(
                    &mut frame,
                    &game,
                    cols,
                    rows,
                    "Interrupted",
                    ResultDestination::Selector,
                )?;
                frames.push(String::from_utf8(frame)?);
                Ok(())
            },
        )
        .unwrap();

        assert_eq!(action, ResultAction::Dismiss);
        assert_eq!(frames.len(), 3);
        assert!(!frames[0].contains("score"));
        assert!(frames[1].contains("score"));
        assert!(!frames[2].contains("score"));
        assert!(frames[2].contains("R: retry / other: selector"));
        assert_eq!(events.next(), Some(Event::Resize(80, 24)));
    }

    #[test]
    fn retry_requires_a_press_and_accepts_either_case() {
        use crossterm::event::{KeyEvent, KeyModifiers};

        for key in ['r', 'R'] {
            let mut kinds = [
                KeyEventKind::Release,
                KeyEventKind::Repeat,
                KeyEventKind::Press,
            ]
            .into_iter();
            let action = wait_for_result_action(
                (80, 24),
                || {
                    let kind = kinds.next().expect("expected a retry key press");
                    Ok(Event::Key(KeyEvent::new_with_kind(
                        KeyCode::Char(key),
                        KeyModifiers::NONE,
                        kind,
                    )))
                },
                |_, _| Ok(()),
            )
            .unwrap();
            assert_eq!(action, ResultAction::Retry);
            assert_eq!(kinds.next(), None);
        }
    }

    #[test]
    fn storage_failures_are_non_fatal_and_corrupt_records_are_preserved() {
        let dir =
            std::env::temp_dir().join(format!("tapline-runtime-results-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("results.json");
        let id = crate::results::Identity::built_in(120.0);
        let mut game = game::Game::new(chart::built_in(120.0, 0.0));
        assert!(persist_result(&path, &id, &game, false).contains("Interrupted"));
        assert!(!path.exists());
        game.completed = true;
        std::fs::write(&path, "corrupt").unwrap();
        assert!(persist_result(&path, &id, &game, false).contains("Cannot read"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "corrupt");
        let blocked = path.join("results.json");
        let status = persist_result(&blocked, &id, &game, false);
        assert!(
            status == "Cannot read saved results" || status.starts_with("Storage error:"),
            "unexpected result status: {status}"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "corrupt");
        std::fs::remove_file(&path).unwrap();
        game.score = 100;
        assert!(persist_result(&path, &id, &game, false).contains("First completed"));
        game.score = 200;
        assert!(persist_result(&path, &id, &game, false).contains("score +100"));
        assert!(crate::results::Store::load(&path)
            .unwrap()
            .get(&id)
            .is_some());
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[cfg(test)]
mod video_tests;
