mod terminal;

use anyhow::Result;
use clap::Parser;
use std::io::{stdout, Stdout};
use std::time::Duration;
use tapline::cli::Args;
use tapline::{audio, cli, runtime};

/// Validate timing before initializing audio or entering terminal raw mode.
fn main() -> Result<()> {
    let args = Args::parse();
    args.validate().unwrap_or_else(|err| err.exit());
    if args.test_tone {
        return run_test_tone();
    }
    terminal::with_alt_screen(&mut stdout(), |out| run(out, &args))
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
