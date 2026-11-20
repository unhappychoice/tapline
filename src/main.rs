mod terminal;

use anyhow::Result;
use clap::Parser;
use std::io::{stdout, Stdout};
use std::process::ExitCode;
use std::time::Duration;
use tapline::cli::Args;
use tapline::render::ResultDestination;
use tapline::{audio, cli, runtime};

fn main() -> ExitCode {
    match run_app() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let message: String = format!("{error:#}")
                .chars()
                .filter(|c| !c.is_control())
                .collect();
            eprintln!("Error: {message}");
            ExitCode::FAILURE
        }
    }
}

/// Validate timing before initializing audio or entering terminal raw mode.
fn run_app() -> Result<()> {
    let args = Args::parse();
    args.validate().unwrap_or_else(|err| err.exit());
    if args.test_tone {
        return run_test_tone();
    }
    terminal::with_alt_screen(&mut stdout(), |out| run(out, &args))
}

fn run(out: &mut Stdout, args: &Args) -> Result<()> {
    let mut selector = tapline::select::SelectorState::default();
    run_session(
        out,
        args,
        |out, args| cli::choose_chart_with_selector(out, args, &mut selector),
        runtime::play_attempt,
        terminal::show_load_error,
    )
}

fn run_session(
    out: &mut Stdout,
    args: &Args,
    mut choose: impl FnMut(&mut Stdout, &Args) -> Result<cli::ChartChoice>,
    mut play: impl FnMut(
        &mut Stdout,
        &Args,
        &tapline::chart::Chart,
        Option<&tapline::results::Identity>,
        ResultDestination,
    ) -> Result<runtime::ResultAction>,
    mut report_error: impl FnMut(&mut Stdout, &anyhow::Error) -> Result<()>,
) -> Result<()> {
    loop {
        let choice = choose(out, args)?;
        let destination = match choice {
            cli::ChartChoice::Selected(_) => ResultDestination::Selector,
            _ => ResultDestination::Exit,
        };
        let (chart, identity) = match cli::load_session(&choice, args) {
            Ok(Some(chart)) => chart,
            Ok(None) => return Ok(()),
            Err(error) if destination == ResultDestination::Selector => {
                report_error(out, &error)?;
                continue;
            }
            Err(error) => return Err(error),
        };
        while play(out, args, &chart, identity.as_ref(), destination)?
            == runtime::ResultAction::Retry
        {}

        if destination == ResultDestination::Exit {
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

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
