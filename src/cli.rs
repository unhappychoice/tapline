use crate::{bms, select};
use anyhow::Result;
use clap::Parser;
use std::io::Stdout;
use std::path::{Path, PathBuf};

#[derive(Parser, Debug)]
#[command(version, about = "A simple rhythm game in your terminal")]
pub struct Args {
    /// Load a BMS chart from file (auto-detects Shift-JIS/UTF-8).
    #[arg(short, long)]
    pub file: Option<PathBuf>,

    /// Open the chart selector on this directory (recursive).
    #[arg(short, long)]
    pub dir: Option<PathBuf>,

    /// BPM for the built-in chart (ignored when a BMS chart is loaded).
    #[arg(short, long, default_value_t = 120.0)]
    pub bpm: f64,

    /// Countdown before the song starts.
    #[arg(long, default_value_t = 2000.0)]
    pub countdown_ms: f64,

    /// Play the built-in chart, skipping the selector.
    #[arg(long)]
    pub built_in: bool,

    /// Disable audio even if a device is available.
    #[arg(long)]
    pub no_audio: bool,

    /// Play a short beep at startup to verify audio, then exit.
    #[arg(long)]
    pub test_tone: bool,

    /// Play a synthesized beep on every note hit (per-lane pitch).
    /// Used automatically if no BMS keysound is loaded.
    #[arg(long)]
    pub synth: bool,

    /// Play note sounds automatically at each note's scheduled time
    /// instead of on key press. Useful on high-latency audio backends
    /// (e.g. WSL2) where you want the audio to match the visuals.
    #[arg(long)]
    pub auto_ks: bool,

    /// Schedule BGM + auto keysound playback this many milliseconds
    /// earlier than the note time. Compensates for audio buffer latency.
    #[arg(long, default_value_t = 0.0)]
    pub audio_lead_ms: f64,
}

pub enum ChartChoice {
    File(PathBuf),
    Cancelled,
    BuiltIn,
}

pub fn choose_chart(out: &mut Stdout, args: &Args) -> Result<ChartChoice> {
    if args.built_in { return Ok(ChartChoice::BuiltIn); }
    if let Some(p) = &args.file { return Ok(ChartChoice::File(p.clone())); }
    let Some(dir) = args.dir.clone().or_else(default_songs_dir) else {
        return Ok(ChartChoice::BuiltIn);
    };
    let charts = select::scan(&dir);
    if charts.is_empty() { return Ok(ChartChoice::BuiltIn); }
    Ok(match select::run(out, &charts)? {
        Some(p) => ChartChoice::File(p),
        None    => ChartChoice::Cancelled,
    })
}

pub fn load_chart(choice: &ChartChoice, args: &Args) -> Result<Option<crate::chart::Chart>> {
    match choice {
        ChartChoice::File(p)   => Ok(Some(bms::load(p, args.countdown_ms)?)),
        ChartChoice::BuiltIn   => Ok(Some(crate::chart::built_in(args.bpm, args.countdown_ms))),
        ChartChoice::Cancelled => Ok(None),
    }
}

fn default_songs_dir() -> Option<PathBuf> {
    if let Ok(v) = std::env::var("TAPLINE_SONGS_DIR") {
        let p = PathBuf::from(v);
        if p.is_dir() { return Some(p); }
    }
    for candidate in ["songs", "tests/fixtures"] {
        let p = PathBuf::from(candidate);
        if p.is_dir() { return Some(p); }
    }
    if let Some(home) = std::env::var_os("HOME") {
        let p = Path::new(&home).join(".tapline").join("songs");
        if p.is_dir() { return Some(p); }
    }
    None
}
