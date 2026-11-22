# Architecture

tapline is a Rust terminal rhythm game. The binary coordinates chart selection
and play sessions; the library exposes chart loading, gameplay, audio, and
rendering modules. Start with [the entry point](https://github.com/unhappychoice/tapline/blob/main/src/main.rs) and
[the library module list](https://github.com/unhappychoice/tapline/blob/main/src/lib.rs).

## From startup to results

1. `main.rs` parses and validates `cli::Args` before entering terminal raw mode.
   `--test-tone` takes a separate audio diagnostic path.
2. `cli::choose_chart_with_selector` chooses the built-in chart, a file, or a
   chart from `select.rs`. The session retains `SelectorState` between plays.
3. `cli::load_session` reads the chart bytes, creates a results identity, and
   calls the BMS or bmson loader. Both produce the shared `chart::Chart` model.
4. The session decodes the samples and applies their settings once per loaded
   chart. `runtime::play_loaded_attempt` starts fresh playback, adjusts the
   playback duration, creates `game::Game`, and runs the input and drawing loop.
5. The runtime stops playback, saves completed results, displays the result
   screen, and returns the player's action. Retry reuses the loaded chart,
   identity, and decoded audio. Dismissing results returns to the selector or
   exits the session.
   A selected chart's load error is shown before returning to selection.

[`terminal.rs`](https://github.com/unhappychoice/tapline/blob/main/src/terminal.rs) owns alternate-screen and raw-mode setup and
cleanup.

## Main parts

| Area | Files | Responsibility |
| ---- | ----- | -------------- |
| Options and loading | [cli.rs](https://github.com/unhappychoice/tapline/blob/main/src/cli.rs) | CLI validation, song-directory defaults, chart choice, and session loading |
| Selection | [select.rs](https://github.com/unhappychoice/tapline/blob/main/src/select.rs), [preview.rs](https://github.com/unhappychoice/tapline/blob/main/src/preview.rs) | Recursive discovery, search and lane filters, progress display, and preview audio |
| Chart formats | [bms.rs](https://github.com/unhappychoice/tapline/blob/main/src/bms.rs), [bmson/](https://github.com/unhappychoice/tapline/blob/main/src/bmson/mod.rs) | Format-specific metadata, timing, lanes, and events |
| Shared chart model | [chart.rs](https://github.com/unhappychoice/tapline/blob/main/src/chart.rs) | Notes, background events, sample references, key layouts, and the built-in chart |
| Gameplay | [game.rs](https://github.com/unhappychoice/tapline/blob/main/src/game.rs), [gauge.rs](https://github.com/unhappychoice/tapline/blob/main/src/gauge.rs) | Judgments, held notes, mines, score, combo, gauge, and completion |
| Runtime | [runtime.rs](https://github.com/unhappychoice/tapline/blob/main/src/runtime.rs) | Input events, elapsed time, scheduled audio, drawing, and result persistence |
| Audio | [audio.rs](https://github.com/unhappychoice/tapline/blob/main/src/audio.rs), [audio/](https://github.com/unhappychoice/tapline/tree/main/src/audio) | Sample decoding, synthesis, layered and sliced sounds, mixing, and volume |
| Display | [render.rs](https://github.com/unhappychoice/tapline/blob/main/src/render.rs), [screen.rs](https://github.com/unhappychoice/tapline/blob/main/src/screen.rs) | HUD, lanes, results, and an off-screen terminal cell buffer |
| Artwork | [bga.rs](https://github.com/unhappychoice/tapline/blob/main/src/bga.rs), [bga/](https://github.com/unhappychoice/tapline/tree/main/src/bga), [artwork.rs](https://github.com/unhappychoice/tapline/blob/main/src/artwork.rs) | BGA layers, optional video decoding, and terminal artwork |
| Records | [results.rs](https://github.com/unhappychoice/tapline/blob/main/src/results.rs) | Chart identities, personal bests, latest results, and JSON storage |

## Important boundaries

The parsers normalize format-specific data into `Chart`; gameplay and rendering
operate on that model. BMS selection metadata and full loading use the same
random seed so branch-dependent metadata agrees with the selected play.
`cli::load_session` derives the record identity from the same bytes it parses.

`runtime.rs` supplies elapsed milliseconds and handles terminal input. `Game`
applies judgment and completion rules; `gauge.rs` calculates recovery and damage.
Scheduled BGM and optional automatic keysounds are dispatched by the runtime,
while note judgments still depend on input.

Drawing code writes terminal commands into `screen::Screen`. Its `present`
method emits only changed cells. BGA images and video frames are rendered as
terminal half blocks; video decoding uses an external `ffmpeg` process.

## Where to start a change

- For a CLI option, start in `cli.rs`, then follow its use in `main.rs` or
  `runtime.rs`. CLI integration checks are in [cli_smoke.rs](https://github.com/unhappychoice/tapline/blob/main/tests/cli_smoke.rs)
  and [stderr_errors.rs](https://github.com/unhappychoice/tapline/blob/main/tests/stderr_errors.rs).
- For chart support, start in the relevant parser and `chart.rs`. Parser tests
  live alongside the modules and in [tests/](https://github.com/unhappychoice/tapline/tree/main/tests); reusable chart inputs are
  in [tests/fixtures/](https://github.com/unhappychoice/tapline/tree/main/tests/fixtures).
- For judgment or gauge behavior, start in `game.rs` and `gauge.rs`, then inspect
  [gameplay.rs](https://github.com/unhappychoice/tapline/blob/main/tests/gameplay.rs) and
  [game_property.rs](https://github.com/unhappychoice/tapline/blob/main/tests/game_property.rs).
- For terminal output, start in `render.rs` or `select.rs`; cell buffering and
  ANSI handling live in `screen.rs` and [screen/](https://github.com/unhappychoice/tapline/tree/main/src/screen).

See [Contributing](https://github.com/unhappychoice/tapline/blob/main/CONTRIBUTING.md) for environment setup and check commands,
and [Getting started](getting-started.md) for a first playthrough.
