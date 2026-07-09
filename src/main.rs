mod audio;
mod bms;
mod chart;
mod cli;
mod game;
mod render;
mod runtime;
mod select;

use anyhow::Result;
use clap::Parser;
use cli::Args;
use crossterm::{cursor, execute, terminal};
use std::io::{stdout, Stdout};
use std::time::Duration;

fn main() -> Result<()> {
    let args = Args::parse();
    if args.test_tone {
        return run_test_tone();
    }
    with_alt_screen(|out| run(out, &args))
}

fn run(out: &mut Stdout, args: &Args) -> Result<()> {
    let choice = cli::choose_chart(out, args)?;
    match cli::load_chart(&choice, args)? {
        Some(chart) => runtime::play_chart(out, args, chart),
        None => Ok(()),
    }
}

fn run_test_tone() -> Result<()> {
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
    Ok(())
}

fn with_alt_screen<F: FnOnce(&mut Stdout) -> Result<()>>(f: F) -> Result<()> {
    let mut out = stdout();
    terminal::enable_raw_mode()?;
    execute!(out, terminal::EnterAlternateScreen, cursor::Hide)?;
    let result = f(&mut out);
    execute!(out, cursor::Show, terminal::LeaveAlternateScreen)?;
    terminal::disable_raw_mode()?;
    result
}
