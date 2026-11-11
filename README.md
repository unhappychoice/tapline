# tapline

[![CI](https://github.com/unhappychoice/tapline/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/unhappychoice/tapline/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/unhappychoice/tapline/branch/main/graph/badge.svg)](https://codecov.io/gh/unhappychoice/tapline)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

A simple rhythm game in your terminal. Plays a built-in chart, or loads BMS / bmson
files with keysound + BGM playback via `rodio`.

Notes fall down the lanes. Tap the letters as they cross the judgment line.
Chain hits for combo, chase accuracy, panic-quit with Esc.

```
                          Sample Song
                        — tapline test
              score      0    combo   0    acc 100.0%

               │      │      │      │      │      │      │      │
               │      │      │      │      │      │      │      │
               │      │  [==]│      │      │      │      │      │
               │      │      │      │      │      │      │      │
               │  [==]│      │      │      │      │      │      │
               │      │      │      │      │      │  [==]│      │
               │      │      │      │      │      │      │      │
               ═══════════════════════════════════════════════════
                  S      D      F     ___     J      K      L

                              PERFECT!

              keys S D F SPACE J K L  ·  quit Esc / Q
```

## Install

From source:

```sh
cargo install --path .
```

## Play

```sh
tapline                                # open the chart selector (see "Song discovery")
tapline --dir ~/bms                    # open selector on a specific directory
tapline --file song.bms                # skip selector, play one chart (auto 4/5/7/9/14-lane)
tapline --file song.bms --no-audio     # play a chart, silent
tapline --built-in --bpm 160           # play the built-in practice chart
```

Inside the selector: `↑ ↓` / `j k` to move, `Enter` to play, `Esc` / `q` to quit.

### Fixing audio delay

On high-latency backends (WSL2, PulseAudio) the per-hit beep can trail the
visual note. Two flags:

```sh
tapline --auto-ks                          # play note sounds at their scheduled times
tapline --auto-ks --audio-lead-ms 80       # additionally pre-play by 80 ms
tapline --file song.bms --audio-lead-ms 60 # BGM-only pre-play
```

`--auto-ks` decouples the audio from your key press so what you hear stays
locked to what you see. `--audio-lead-ms` shifts every scheduled sound
(BGM + auto keysounds) that many milliseconds earlier — start around
`50–120` on WSL2 and tune by ear.

## Song discovery

Without `--file` or `--built-in`, tapline scans the first directory it finds:

1. `$TAPLINE_SONGS_DIR`
2. `./songs`
3. `./tests/fixtures`
4. `~/.tapline/songs`

If nothing is found, the built-in practice chart plays. Scanning is recursive
(up to 5 levels deep) and picks up `.bms`, `.bml`, `.bme`, `.pms`, `.bmson`.

## Bundled sample charts

The `songs/` directory ships eight hand-written charts across all three
lane counts and difficulty tiers:

| chart                        | lanes | difficulty  | BPM | flavor                                    |
| ---------------------------- | ----- | ----------- | --- | ----------------------------------------- |
| `01-first-steps.bms`         | 4     | BEGINNER 1  | 100 | lane recognition + first chords           |
| `02-steady-rain.bms`         | 5     | NORMAL 4    | 130 | 8ths + 16th runs + syncopation            |
| `03-rolling-thunder.bms`     | 7     | HYPER 8     | 155 | rolls, jacks, dense finales               |
| `04-neon-drive.bms`          | 4     | NORMAL 3    | 128 | synthwave groove, chord shots             |
| `05-circuit-breaker.bms`     | 5     | HYPER 7     | 148 | breakbeat runs, alternating hands         |
| `06-blue-hour.bms`           | 7     | NORMAL 5    | 140 | gentler 7K to learn all seven keys        |
| `07-final-cascade.bms`       | 7     | ANOTHER 11  | 175 | endgame chart: streams, jacks, staircases |
| `08-arcade-hero.bms`         | 4     | HYPER 7     | 165 | chiptune 4K with 16th runs                |

All charts play with the synth fallback — no WAV assets required.

## BMS support

Encoding is auto-detected (UTF-8 → fallback Shift-JIS).

### Supported

| category      | items                                                       |
| ------------- | ----------------------------------------------------------- |
| headers       | `#TITLE`, `#SUBTITLE`, `#ARTIST`, `#SUBARTIST`, `#GENRE`, `#MAKER`, `#STAGEFILE`, `#BANNER`, `#BPM`, `#PLAYLEVEL`, `#DIFFICULTY`, `#RANK`, `#TOTAL`, `#VOLWAV`, `#WAVxx`, `#BPMxx` / `#EXBPMxx`, `#STOPxx`, `#STP` (millisecond stops), `#BMPxx`, `#LNOBJ`, `#LNTYPE` |
| channels      | `01` (BGM auto-play), `02` (measure-length change), `03`/`08` (BPM change), `04`/`06`/`07` (BGA base/poor/overlay), `09` (STOP), `11–19` (P1 visible notes), `21–29` (P2 visible notes), `31–39` / `41–49` (P1/P2 invisible keysound changes), `51–59` (P1 long notes), `61–69` (P2 long notes), `D1–D9` (P1 landmines), `E1–E9` (P2 landmines) |
| lane modes    | 4K, 5K (`11–15`), 7K (`11–15 + 18/19`) SP, 9K PMS (`.pms`: `11–15 + 22–25`), plus 14K double-play when both sides fill 7 lanes each |
| audio formats | WAV / OGG / MP3 (extension fallback on lookup)              |
| polyphony     | multiple `#00101` lines mix concurrently                    |
| control flow  | `#RANDOM`, `#SETRANDOM`, `#ENDRANDOM`, `#IF`, `#ELSEIF`, `#ELSE`, `#ENDIF`, `#SWITCH`, `#SETSWITCH`, `#CASE`, `#SKIP`, `#DEF`, `#ENDSW` (nesting and switch fallthrough supported) |

PMS mode is selected by the `.pms` extension (case-insensitive) and always uses
nine lanes. Long notes (`51–55 + 62–65`) and landmines (`D1–D5 + E2–E5`)
follow the same button order as visible notes.

`#RANDOM` branches are drawn afresh for each play. The selector and chart loader
share the same draw so displayed metadata matches gameplay. Set
`TAPLINE_RANDOM_SEED` to an unsigned 64-bit integer for reproducible branches.

Invisible notes update the lane keysound for empty presses without being drawn,
judged, counted as notes, or played automatically.

### Long notes and landmines

Long-note starts are judged like regular notes. On terminals that report key
release events, releasing a held long note is judged separately against its end:
the rank-dependent windows below apply. A release outside the GOOD window
is a MISS and resets the combo.

If no release event arrives and the note remains held in the game, it
automatically completes as PERFECT once the current time passes its end plus
the rank-dependent MISS delay. A long note whose start was never hit instead
becomes a MISS after its start plus the same delay.

Pressing a landmine's lane within ±40 ms of its timestamp detonates it once,
adds a MISS, and resets the combo. Each matching mine is judged independently.
The parsed `damage` value does not scale the penalty or subtract score.

### Not yet supported (v1)

- Landmine penalties proportional to the parsed `damage` value
- BGA rendering (channels `04` / `06` / `07` and `#BMPxx` are parsed and stored on `Chart.bga` / `Chart.bmp_paths`, but the terminal renderer does not draw them yet)
- Non-canonical double-play like 5K DP or asymmetric side counts (only 7K + 7K is folded into the 14-lane playable pool; other DP shapes are still parsed into `Chart.p2_notes` but not merged)

## bmson support

bmson 1.0.0 (UTF-8 JSON) charts can be selected or loaded with
`tapline --file chart.bmson`. BPM changes, stops, long notes (`l`),
background audio, and keysound slicing (`c`) are supported. Sound slices
use elapsed chart time, including BPM changes and stops, and share decoded
audio buffers. Notes at the same lane and pulse are judged once and play all
layered keysounds. Shared slices have a polyphony of one, and background notes
sharing a playable slice are discarded. Layered notes with unequal lengths are
rejected. Missing audio extensions fall back to WAV / OGG / MP3.

Supported layouts are `beat-5k`, `beat-7k`, `beat-14k`, `popn-5k`, and
`generic-4keys` / `generic-5keys` / `generic-7keys` / `generic-14keys`.
Turntable notes play automatically as background audio. Other layouts and
legacy bmson versions are rejected. BGA base, overlay, and poor events are
stored on `Chart`; BGA rendering remains unsupported.

The format follows the [bmson specification](https://bmson-spec.readthedocs.io/en/master/doc/index.html).
A playable example is available at `tests/fixtures/minimal.bmson`.

## Judgment windows

`#RANK` selects tap and long-note start/release windows. Missing or invalid
ranks use NORMAL, preserving the original behavior. These are tapline's windows,
scaled from NORMAL by 0.5 / 0.75 / 1 / 1.25 / 1.5 respectively.

| #RANK | difficulty | PERFECT | GREAT | GOOD | MISS delay |
| ----- | ---------- | ------- | ----- | ---- | ---------- |
| 0 | VERY HARD | ±22.5 ms | ±45 ms | ±70 ms | > 80 ms |
| 1 | HARD | ±33.75 ms | ±67.5 ms | ±105 ms | > 120 ms |
| 2 (default) | NORMAL | ±45 ms | ±90 ms | ±140 ms | > 160 ms |
| 3 | EASY | ±56.25 ms | ±112.5 ms | ±175 ms | > 200 ms |
| 4 | VERY EASY | ±67.5 ms | ±135 ms | ±210 ms | > 240 ms |

Window boundaries are inclusive. Presses outside GOOD are ignored; unhit notes
become MISS strictly after the MISS delay. Landmine timing remains ±40 ms.
Base points are PERFECT: 300, GREAT: 200, GOOD: 100, MISS: 0.

Combo (up to +50) is added on top of every non-miss hit.

## Keys

DJMax-style layout. In 5K the center lane accepts either `F` or `J` — press
whichever finger is closer.

| lanes | keys                    | notes                             |
| ----- | ----------------------- | --------------------------------- |
| 4     | `S D       K L`               | outer 2+2, index fingers rest     |
| 5     | `S D  F/J  K L`               | center lane bound to both F and J |
| 7     | `S D  F SPACE J  K L`         | spacebar is the center            |
| 9     | `A S D F SPACE J K L ;`        | PMS buttons 1–9; spacebar is the center |
| 14    | `S D F SPACE J K L / Z X C V B N M` | 7K SP layout on the home row for P1, ZXCVBNM below for P2 |

## License

MIT
