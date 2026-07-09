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

    // ---------- default_songs_dir ----------
    //
    // These tests mutate the process's environment and CWD, which are
    // global. Serialize them through a Mutex so they don't clobber each
    // other under `cargo test`'s parallel harness.

    use std::sync::Mutex;

    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    struct EnvSnapshot {
        songs_dir: Option<std::ffi::OsString>,
        home: Option<std::ffi::OsString>,
        cwd: PathBuf,
    }

    impl EnvSnapshot {
        fn capture() -> Self {
            Self {
                songs_dir: std::env::var_os("TAPLINE_SONGS_DIR"),
                home: std::env::var_os("HOME"),
                cwd: std::env::current_dir().unwrap(),
            }
        }
        fn restore(self) {
            match self.songs_dir {
                Some(v) => std::env::set_var("TAPLINE_SONGS_DIR", v),
                None => std::env::remove_var("TAPLINE_SONGS_DIR"),
            }
            match self.home {
                Some(v) => std::env::set_var("HOME", v),
                None => std::env::remove_var("HOME"),
            }
            let _ = std::env::set_current_dir(self.cwd);
        }
    }

    #[test]
    fn default_songs_dir_env_var_wins_when_it_points_at_a_real_directory() {
        let _lock = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let snap = EnvSnapshot::capture();
        let dir = tempdir();
        std::env::set_var("TAPLINE_SONGS_DIR", &dir);
        // Make sure `./songs` / `./tests/fixtures` aren't visible either.
        let isolated = tempdir();
        std::env::set_current_dir(&isolated).unwrap();
        let out = default_songs_dir();
        assert_eq!(out.as_deref(), Some(dir.as_path()));
        snap.restore();
    }

    #[test]
    fn default_songs_dir_env_var_is_ignored_when_missing_from_disk() {
        let _lock = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let snap = EnvSnapshot::capture();
        std::env::set_var(
            "TAPLINE_SONGS_DIR",
            "/nonexistent-tapline-cli-env-branch-xyz",
        );
        let isolated = tempdir();
        std::env::set_current_dir(&isolated).unwrap();
        std::env::remove_var("HOME");
        assert!(
            default_songs_dir().is_none(),
            "env var pointing at a missing dir should not survive"
        );
        snap.restore();
    }

    #[test]
    fn default_songs_dir_falls_back_to_local_songs_when_env_is_empty() {
        let _lock = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let snap = EnvSnapshot::capture();
        std::env::remove_var("TAPLINE_SONGS_DIR");
        std::env::remove_var("HOME");
        let workspace = tempdir();
        std::fs::create_dir_all(workspace.join("songs")).unwrap();
        std::env::set_current_dir(&workspace).unwrap();
        let out = default_songs_dir();
        assert_eq!(out, Some(PathBuf::from("songs")));
        snap.restore();
    }

    #[test]
    fn default_songs_dir_falls_back_to_home_tapline_songs_when_no_local_dir() {
        let _lock = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let snap = EnvSnapshot::capture();
        std::env::remove_var("TAPLINE_SONGS_DIR");
        let isolated = tempdir();
        std::env::set_current_dir(&isolated).unwrap();
        let home = tempdir();
        let songs = home.join(".tapline/songs");
        std::fs::create_dir_all(&songs).unwrap();
        std::env::set_var("HOME", &home);
        let out = default_songs_dir();
        assert_eq!(out.as_deref(), Some(songs.as_path()));
        snap.restore();
    }

    #[test]
    fn default_songs_dir_gives_up_when_nothing_matches() {
        let _lock = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let snap = EnvSnapshot::capture();
        std::env::remove_var("TAPLINE_SONGS_DIR");
        std::env::remove_var("HOME");
        let isolated = tempdir();
        std::env::set_current_dir(&isolated).unwrap();
        assert!(default_songs_dir().is_none());
        snap.restore();
    }
}
