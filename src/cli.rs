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
    if args.built_in {
        return Ok(ChartChoice::BuiltIn);
    }
    if let Some(p) = &args.file {
        return Ok(ChartChoice::File(p.clone()));
    }
    let Some(dir) = args.dir.clone().or_else(default_songs_dir) else {
        return Ok(ChartChoice::BuiltIn);
    };
    let charts = select::scan(&dir);
    if charts.is_empty() {
        return Ok(ChartChoice::BuiltIn);
    }
    Ok(match select::run(out, &charts)? {
        Some(p) => ChartChoice::File(p),
        None => ChartChoice::Cancelled,
    })
}

pub fn load_chart(choice: &ChartChoice, args: &Args) -> Result<Option<crate::chart::Chart>> {
    match choice {
        ChartChoice::File(p) => Ok(Some(bms::load(p, args.countdown_ms)?)),
        ChartChoice::BuiltIn => Ok(Some(crate::chart::built_in(args.bpm, args.countdown_ms))),
        ChartChoice::Cancelled => Ok(None),
    }
}

fn default_songs_dir() -> Option<PathBuf> {
    if let Ok(v) = std::env::var("TAPLINE_SONGS_DIR") {
        let p = PathBuf::from(v);
        if p.is_dir() {
            return Some(p);
        }
    }
    for candidate in ["songs", "tests/fixtures"] {
        let p = PathBuf::from(candidate);
        if p.is_dir() {
            return Some(p);
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        let p = Path::new(&home).join(".tapline").join("songs");
        if p.is_dir() {
            return Some(p);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn args_parse_defaults() {
        let a = Args::parse_from(["tapline"]);
        assert!(a.file.is_none());
        assert!(a.dir.is_none());
        assert_eq!(a.bpm, 120.0);
        assert_eq!(a.countdown_ms, 2000.0);
        assert!(!a.built_in);
        assert!(!a.no_audio);
        assert!(!a.test_tone);
        assert!(!a.synth);
        assert!(!a.auto_ks);
        assert_eq!(a.audio_lead_ms, 0.0);
    }

    #[test]
    fn args_parse_all_flags() {
        let a = Args::parse_from([
            "tapline",
            "--file",
            "song.bms",
            "--dir",
            "/tmp/songs",
            "--bpm",
            "160",
            "--countdown-ms",
            "500",
            "--built-in",
            "--no-audio",
            "--test-tone",
            "--synth",
            "--auto-ks",
            "--audio-lead-ms",
            "80",
        ]);
        assert_eq!(a.file.as_deref(), Some(Path::new("song.bms")));
        assert_eq!(a.dir.as_deref(), Some(Path::new("/tmp/songs")));
        assert_eq!(a.bpm, 160.0);
        assert_eq!(a.countdown_ms, 500.0);
        assert!(a.built_in);
        assert!(a.no_audio);
        assert!(a.test_tone);
        assert!(a.synth);
        assert!(a.auto_ks);
        assert_eq!(a.audio_lead_ms, 80.0);
    }

    #[test]
    fn args_short_flags_are_wired() {
        let a = Args::parse_from(["tapline", "-b", "180", "-f", "song.bms", "-d", "/x"]);
        assert_eq!(a.bpm, 180.0);
        assert_eq!(a.file.as_deref(), Some(Path::new("song.bms")));
        assert_eq!(a.dir.as_deref(), Some(Path::new("/x")));
    }

    fn base_args() -> Args {
        Args::parse_from(["tapline"])
    }

    #[test]
    fn load_chart_built_in_returns_built_in_chart() {
        let chart = load_chart(&ChartChoice::BuiltIn, &base_args()).unwrap();
        let chart = chart.expect("BuiltIn should always yield a chart");
        assert_eq!(chart.title, "Built-in Practice");
        assert_eq!(chart.lane_count, 4);
    }

    #[test]
    fn load_chart_built_in_respects_args_bpm() {
        let mut args = base_args();
        args.bpm = 200.0;
        let chart = load_chart(&ChartChoice::BuiltIn, &args).unwrap().unwrap();
        assert_eq!(chart.bpm, 200.0);
    }

    #[test]
    fn load_chart_cancelled_returns_none() {
        let out = load_chart(&ChartChoice::Cancelled, &base_args()).unwrap();
        assert!(out.is_none());
    }

    #[test]
    fn load_chart_file_reads_bms_from_disk() {
        let dir = tempdir();
        let path = dir.join("song.bms");
        std::fs::write(&path, "#TITLE Loaded\n#ARTIST me\n#BPM 155\n#00111:0100\n").unwrap();
        let chart = load_chart(&ChartChoice::File(path), &base_args())
            .unwrap()
            .unwrap();
        assert_eq!(chart.title, "Loaded");
        assert_eq!(chart.bpm, 155.0);
    }

    fn tempdir() -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let dir = std::env::temp_dir().join(format!("tapline-cli-test-{}-{}", pid, n));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}
