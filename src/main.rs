mod audio;
mod bms;
mod chart;
mod game;
mod render;

use anyhow::Result;
use clap::Parser;
use crossterm::{cursor, event::{self, Event, KeyCode, KeyEventKind}, execute, terminal};
use std::io::stdout;
use std::path::PathBuf;
use std::time::{Duration, Instant};

#[derive(Parser, Debug)]
#[command(version, about = "A simple rhythm game in your terminal")]
struct Args {
    /// Load a BMS chart from file (auto-detects Shift-JIS/UTF-8).
    #[arg(short, long)]
    file: Option<PathBuf>,

    /// BPM for the built-in chart (ignored when --file is given).
    #[arg(short, long, default_value_t = 120.0)]
    bpm: f64,

    /// Countdown before the song starts.
    #[arg(long, default_value_t = 2000.0)]
    countdown_ms: f64,

    /// Disable audio even if a device is available.
    #[arg(long)]
    no_audio: bool,

    /// Play a short beep at startup to verify audio, then exit.
    #[arg(long)]
    test_tone: bool,

    /// Play a synthesized beep on every note hit (per-lane pitch).
    /// Used automatically if no BMS keysound is loaded.
    #[arg(long)]
    synth: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();

    if args.test_tone {
        let bank = audio::SampleBank::new(&Default::default());
        if !bank.enabled {
            eprintln!("audio backend unavailable");
            std::process::exit(1);
        }
        eprintln!("audio backend ready, playing test tone...");
        for freq in [261.63f32, 329.63, 392.00, 523.25] {
            bank.play_synth(freq, 200);
            std::thread::sleep(Duration::from_millis(220));
        }
        std::thread::sleep(Duration::from_millis(400));
        return Ok(());
    }

    let chart = match &args.file {
        Some(path) => bms::load(path, args.countdown_ms)?,
        None => chart::built_in(args.bpm, args.countdown_ms),
    };
    let synth_mode = args.synth || chart.wav_paths.is_empty();
    let bank = if args.no_audio {
        audio::SampleBank::silent()
    } else {
        audio::SampleBank::new(&chart.wav_paths)
    };
    let mut game = game::Game::new(chart);

    let mut out = stdout();
    terminal::enable_raw_mode()?;
    execute!(out, terminal::EnterAlternateScreen, cursor::Hide)?;

    let result = run_game(&mut out, &mut game, &bank, args.countdown_ms, synth_mode);

    execute!(out, cursor::Show, terminal::LeaveAlternateScreen)?;
    terminal::disable_raw_mode()?;
    result?;
    Ok(())
}

fn run_game(out: &mut std::io::Stdout, game: &mut game::Game, bank: &audio::SampleBank, countdown_ms: f64, synth_mode: bool) -> Result<()> {
    let start = Instant::now();
    let frame_dt = Duration::from_millis(16);
    let mut quit = false;
    let mut bgm_cursor: usize = 0;
    let mut prev_mode: u8 = 0;

    while !quit {
        let now = elapsed_ms(start);
        if now >= game.chart.duration_ms { break; }

        let mode: u8 = if now < countdown_ms { 1 } else { 2 };
        if mode != prev_mode {
            execute!(out, terminal::Clear(terminal::ClearType::All))?;
            prev_mode = mode;
        }

        if mode == 1 {
            render::draw_intro(out, game, countdown_ms - now, bank.enabled)?;
        } else {
            game.check_misses(now);
            while bgm_cursor < game.chart.bgm.len() && game.chart.bgm[bgm_cursor].time_ms <= now {
                let id = game.chart.bgm[bgm_cursor].keysound;
                bank.play(id);
                bgm_cursor += 1;
            }
            render::draw(out, game, now)?;
        }

        while event::poll(Duration::from_millis(0))? {
            if let Event::Key(k) = event::read()? {
                if k.kind == KeyEventKind::Release { continue; }
                match k.code {
                    KeyCode::Esc => quit = true,
                    KeyCode::Char('q') | KeyCode::Char('Q') => quit = true,
                    KeyCode::Char(c) => {
                        if now >= countdown_ms {
                            if let Some(lane) = lane_for_key(c, &game.chart.keys) {
                                let ks = game.hit(lane, elapsed_ms(start));
                                bank.play_hit(lane, ks, synth_mode);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        std::thread::sleep(frame_dt);
    }

    execute!(out, terminal::Clear(terminal::ClearType::All))?;
    render::draw_result(out, game)?;
    loop {
        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(k) = event::read()? {
                if k.kind == KeyEventKind::Press { break; }
            }
        }
    }
    Ok(())
}

fn lane_for_key(c: char, keys: &[char]) -> Option<usize> {
    let up = c.to_ascii_uppercase();
    keys.iter().position(|k| k.to_ascii_uppercase() == up)
}

fn elapsed_ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}
