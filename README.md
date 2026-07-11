# tapline

[![CI](https://github.com/unhappychoice/tapline/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/unhappychoice/tapline/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/unhappychoice/tapline/branch/main/graph/badge.svg)](https://codecov.io/gh/unhappychoice/tapline)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

A simple rhythm game in your terminal. Plays a built-in chart, or loads BMS
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
tapline --file song.bms                # skip selector, play one chart (auto 4/5/7-lane)
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
(up to 5 levels deep) and picks up `.bms`, `.bml`, `.bme`, `.pms`.

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
| headers       | `#TITLE`, `#SUBTITLE`, `#ARTIST`, `#SUBARTIST`, `#GENRE`, `#MAKER`, `#STAGEFILE`, `#BANNER`, `#BPM`, `#PLAYLEVEL`, `#DIFFICULTY`, `#RANK`, `#TOTAL`, `#VOLWAV`, `#WAVxx`, `#BPMxx`, `#STOPxx`, `#LNOBJ`, `#LNTYPE` |
| channels      | `01` (BGM auto-play), `02` (measure-length change), `03`/`08` (BPM change), `09` (STOP), `11–19` (P1 visible notes), `51–59` (P1 long notes) |
| lane modes    | 4-key, 5-key (`11–15`), 7-key (`11–15 + 18/19`) auto-detect |
| audio formats | WAV / OGG / MP3 (extension fallback on lookup)              |
| polyphony     | multiple `#00101` lines mix concurrently                    |

### Not yet supported (v1)

- Long-note release scoring (LN starts are hittable but the release edge is not judged yet)
- Landmines (`D1–D9`)
- `#RANDOM` / `#IF` / `#SWITCH`
- BGA (channels `04`, `06`, `07`)
- Double-play (channels `21–29` etc.)

## Judgment windows

| tier    | window   | base points |
| ------- | -------- | ----------- |
| PERFECT | ±45 ms   | 300         |
| GREAT   | ±90 ms   | 200         |
| GOOD    | ±140 ms  | 100         |
| MISS    | > 160 ms | 0           |

Combo (up to +50) is added on top of every non-miss hit.

## Keys

DJMax-style layout. In 5K the center lane accepts either `F` or `J` — press
whichever finger is closer.

| lanes | keys                    | notes                             |
| ----- | ----------------------- | --------------------------------- |
| 4     | `S D       K L`         | outer 2+2, index fingers rest     |
| 5     | `S D  F/J  K L`         | center lane bound to both F and J |
| 7     | `S D  F SPACE J  K L`   | spacebar is the center            |

## License

MIT
