use anyhow::Result;
use clap::Parser;
use crossterm::{
    cursor,
    event::{KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags},
    execute, terminal,
};
use std::io::{stdout, Stdout};
use std::time::Duration;
use tapline::cli::Args;
use tapline::{audio, cli, runtime};

fn main() -> Result<()> {
    let args = Args::parse();
    if args.test_tone {
        return run_test_tone();
    }
    with_alt_screen(|out| run(out, &args))
}

fn run(out: &mut Stdout, args: &Args) -> Result<()> {
    loop {
        let choice = cli::choose_chart(out, args)?;
        let Some(chart) = cli::load_chart(&choice, args)? else {
            return Ok(());
        };
        runtime::play_chart(out, args, chart)?;
        if args.file.is_some() || args.built_in {
            return Ok(());
        }
    }
}

fn run_test_tone() -> Result<()> {
    let bank = audio::SampleBank::new(&Default::default());
    if !bank.enabled {
        eprintln!("audio backend unavailable");
        std::process::exit(1);
    }
    if std::env::var_os("TAPLINE_TEST_TONE_QUIET").is_some() {
        eprintln!("audio backend ready (quiet mode, skipping tone)");
        return Ok(());
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
    // Best-effort: ask the terminal to report key up events so long-note
    // releases can be judged. Terminals that don't understand the kitty
    // keyboard protocol will ignore this — the runtime falls back to
    // auto-releasing LNs when they cross the miss window.
    let pushed = execute!(
        out,
        PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::REPORT_EVENT_TYPES)
    )
    .is_ok();
    let result = f(&mut out);
    if pushed {
        let _ = execute!(out, PopKeyboardEnhancementFlags);
    }
    execute!(out, cursor::Show, terminal::LeaveAlternateScreen)?;
    terminal::disable_raw_mode()?;
    result
}
