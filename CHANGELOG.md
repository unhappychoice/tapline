# Changelog

All notable changes to tapline are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
This project intends to follow [Semantic Versioning](https://semver.org/)
once it makes its first tagged release.

## [Unreleased]

### Added

- BMS, PMS, and bmson chart playback with keysounds, background music, and
  terminal-rendered BGA artwork (still images and, with `ffmpeg` on `PATH`,
  video).
- A chart selector with recursive directory discovery, live search, and
  lane-count filtering, with preview audio for charts that provide one.
- A built-in four-lane practice chart with adjustable tempo (`--bpm`).
- Custom gameplay key bindings loaded from a JSON file (`--keymap`).
- Judgment windows scaled by BMS `#RANK` / `#DEFEXRANK` / `#EXRANKxx`, long
  notes (LN / CN / HCN), and landmines.
- A NORMAL health gauge with format-specific recovery and a CLEAR / FAILED
  result.
- Personal best tracking per chart (score, accuracy, max combo, and clear
  status), stored locally in `results.json`.
- Audio delay compensation flags (`--auto-ks`, `--audio-lead-ms`) for
  high-latency backends such as WSL2.

[Unreleased]: https://github.com/unhappychoice/tapline/commits/main
