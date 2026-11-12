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
tapline --file song.bms                # skip selector, play one chart (auto lane detection)
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

Encoding is auto-detected (UTF-8 → fallback Shift-JIS). Object IDs default to
case-insensitive base 36; `#BASE 62` enables case-sensitive base-62 IDs.

### Supported

| category      | items                                                       |
| ------------- | ----------------------------------------------------------- |
| headers       | `#PLAYER`, `#TITLE`, `#SUBTITLE`, `#ARTIST`, `#SUBARTIST`, `#GENRE`, `#MAKER`, `#STAGEFILE`, `#BANNER`, `#BACKBMP`, `#COMMENT`, `#BPM`, `#PLAYLEVEL`, `#DIFFICULTY`, `#RANK`, `#DEFEXRANK`, `#EXRANKxx`, `#TOTAL`, `#VOLWAV`, `#WAVxx`, `#EXWAVxx`, `#WAVCMD`, `#BPMxx` / `#EXBPMxx`, `#STOPxx`, `#STP` (millisecond stops), `#SCROLLxx`, `#SPEEDxx`, `#BMPxx`, `#TEXTxx` / `#SONGxx`, `#LNOBJ`, `#LNTYPE`, `#LNMODE` |
| channels      | `01` (BGM auto-play), `02` (measure-length change), `03`/`08` (BPM change), `04`/`06`/`07` (BGA base/poor/overlay), `09` (STOP), `SC` (scroll factor), `SP` (spacing factor), `97`/`98` (BGM/keysound volume), `99` (song text), `A0` (judgment-width change), `11–19` (P1 visible notes), `21–29` (P2 visible notes), `31–39` / `41–49` (P1/P2 invisible keysound changes), `51–59` (P1 long notes), `61–69` (P2 long notes), `D1–D9` (P1 landmines), `E1–E9` (P2 landmines) |
| lane modes    | 4K, 5K (`11–15`), 7K (`11–15 + 18/19`), 9K PMS (`.pms`: `11–15 + 22–25`); 10K, 14K, and asymmetric double-play (`#PLAYER 3` or playable P2 channels); scratch (`16` / `26`) adds a lane on each populated side |
| audio formats | WAV / OGG / MP3 (extension fallback on lookup)              |
| polyphony     | multiple `#00101` lines mix concurrently                    |
| control flow  | `#RANDOM`, `#SETRANDOM`, `#ENDRANDOM`, `#IF`, `#ELSEIF`, `#ELSE`, `#ENDIF`, `#SWITCH`, `#SETSWITCH`, `#CASE`, `#SKIP`, `#DEF`, `#ENDSW` (nesting and switch fallthrough supported) |

PMS mode is selected by the `.pms` extension (case-insensitive) and always uses
nine lanes. Long notes (`51–55 + 62–65`) and landmines (`D1–D5 + E2–E5`)
follow the same button order as visible notes.

`#RANDOM` branches are drawn afresh for each play. The selector and chart loader
share the same draw so displayed metadata matches gameplay. Set
`TAPLINE_RANDOM_SEED` to an unsigned 64-bit integer for reproducible branches.

Song text from `#TEXTxx` (or `#SONGxx`) appears at the top of the HUD when
channel `99` triggers its object ID (base 36 by default, or base 62 with `#BASE 62`). It remains until the next defined text
event; an empty definition clears it. Undefined IDs and `00` are ignored.
Terminal control characters are removed, and long text is clipped to fit.

The selector displays the selected chart’s `#BANNER` and `#COMMENT` when the
terminal is at least 16 rows tall. During the countdown, `#STAGEFILE` appears
above the song information. PNG, JPEG, BMP, and GIF artwork is rendered with
true-color half-block characters; missing or invalid images are skipped.
`#BACKBMP` is stored as metadata for future background rendering.

Scratch channels `16/26`, `56/66` (long notes), and `D6/E6` (landmines)
are playable. Free-zone / pedal channels `17/27` and their long-note /
landmine counterparts are ignored. Scratch is the last lane on each side.
Channel-based inference uses populated channels; empty channels do not count.
Asymmetric DP uses the layout for its total lane count, including the scratch
layouts in the Keys section for totals of 8, 12, or 16 and the PMS layout for 9 lanes.

`#VOLWAV` scales sample and synth playback by a percentage (default 100).
Channels `97` (BGM) and `98` (keysounds) set independent volume multipliers
for sounds triggered at or after each event: hexadecimal `01`–`FF`, with
`FF` at full volume and `00` leaving the volume unchanged. These multipliers
combine with `#VOLWAV`; already playing sounds retain their starting gain.

Invisible notes update the lane keysound for empty presses without being drawn,
judged, counted as notes, or played automatically.

### BGA

BMP, PNG, JPEG, and GIF still images are loaded before playback and drawn with
true-color half-block characters to the left of the lanes when space permits.
Channel `04` selects the base; `07` overlays it with pure black treated as
transparent. Channel `06` replaces both for 500 ms after each MISS. Missing or
undecodable images are skipped. Narrow terminals hide BGA without moving the
lanes. Animated images use their first frame. Video BGA (including MP4 and WebM)
uses an optional `ffmpeg` executable on PATH, with no build-time dependency.
Frames are decoded in the background at 30 fps and start at each layer event,
including repeated events for the same file. Playback holds the last frame at
the end; video audio is ignored. Missing decoders and unsupported or corrupt
videos are silently skipped. Sixel and Kitty graphics are not supported.

Charts with `SC` events use accumulated scroll distance for note positions.
`#SCROLLxx` changes the rate from its event onward (0.5 gives half speed,
0 freezes scrolling, and negative values reverse it). `#SPEEDxx` / `SP`
scales the entire visible spacing at the current time. Both default to 1;
judgment and audio timing are unaffected. Positions outside the terminal
playfield are clipped, including positions below the judgment line.

Mode detection uses the extension, `#PLAYER`, and materialized note channels.
`.bme` reserves seven keys per side even when channels `18`/`19` are unused;
`.bms` and `.bml` infer seven keys from extended key channels, otherwise five.
`#PLAYER 2` (Couple) and `#PLAYER 3` (DP) declare two sides. Populated playable P2 channels also select two-sided playback.
For `.bms` and `.bml`, each side infers its own width, allowing asymmetric DP.
`.pms` identifies nine keys without scratch; BMS scratch usage is detected from
channel `16`/`26` and their long-note/landmine equivalents.

PMS uses the full 9K playback layout. 10K makes all five keys on each side playable,
and sparse undeclared `.bms`/`.bml`
charts retain the legacy 4K fallback. `.bme` DP charts reserve all seven P1
lanes and place P2 notes in lanes 7–13, including sparse DP charts.

### Long notes and landmines

Long-note starts are judged like regular notes. `#LNMODE` selects the hold behavior:

- `1` (LN): holding through the end automatically completes as PERFECT, without
  judging release timing. Releasing before the end is a MISS.
- `2` (CN): the release is judged against the end time as described below.
- `3` (HCN): currently uses CN judgments. Continuous hold gauge gains/losses
  are not implemented yet.

Omitted or invalid `#LNMODE` defaults to CN to preserve existing behavior.
Both long-note channels and `#LNOBJ` notes use this setting.

For CN/HCN on terminals that report key
release events, releasing a held long note is judged separately against its end:
the rank-dependent windows below apply. A release outside the GOOD window
is a MISS and resets the combo.

For CN/HCN, if no release event arrives and the note remains held in the game, it
automatically completes as PERFECT once the current time passes its end plus
the rank-dependent MISS delay. A long note whose start was never hit instead
becomes a MISS after its start plus the same delay.

Pressing a landmine's lane within ±40 ms of its timestamp detonates it once,
subtracts `damage / 2` percentage points from the gauge and resets the combo.
Damage is case-insensitive base 36 (`01`–`ZZ`), regardless of `#BASE`; `00`
means no mine. For example, `0A` subtracts 5 points and `10` subtracts 18.
Each matching mine detonates independently and only once. Mines do not add
MISS judgments or change score, accuracy, or judgment/BGA flashes. `ZZ` (1295)
empties the gauge; it does not end play early, and later notes can recover it.

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
rendered using the same half-block image support as BMS charts.

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

`#DEFEXRANK` overrides `#RANK` with a percentage of NORMAL (50 halves the
windows). `#EXRANKxx` defines percentages selected by channel `A0`, effective
from the event timestamp. Undefined objects and non-positive or non-finite
percentages are ignored. Tap and long-note start windows use the note timestamp;
release windows use the long-note end timestamp. Automatic MISS and held-note
completion deadlines scale too; landmine windows remain fixed.

Combo (up to +50) is added on top of every non-miss hit.

## NORMAL gauge

The gauge starts at 20%, stays between 0% and 100%, and clears at 80% or
higher when the chart ends. Reaching 0% does not end play early. The play HUD
and result screen show the gauge and its current CLEAR / FAILED status.
Quitting early always results in FAILED. Results wait for all pending judgments,
including rank-dependent misses and held long-note endings. Displayed percentages
are truncated to one decimal place so values below 80% never display as 80.0%.

PERFECT and GREAT recover `#TOTAL / note count` percentage points; GOOD
recovers half that amount. Each MISS subtracts
6 percentage points. Missing, non-finite, or non-positive `#TOTAL` defaults
to 260. Long-note starts and ends each count as one note because both are
judged separately; mines and background sounds do not count toward recovery.
An empty chart has no recovery. Mines subtract half their base-36 damage
value in percentage points. Other gauge types are not implemented yet.

## Keys

DJMax-style layout. In 5K the center lane accepts either `F` or `J` — press
whichever finger is closer.

| lanes | keys                    | notes                             |
| ----- | ----------------------- | --------------------------------- |
| 4     | `S D       K L`               | outer 2+2, index fingers rest     |
| 5     | `S D  F/J  K L`               | center lane bound to both F and J |
| 7     | `S D  F SPACE J  K L`         | spacebar is the center            |
| 6     | `S D F/J K L A` | 5K plus scratch on A |
| 8     | `S D F SPACE J K L A` | 7K plus scratch on A |
| 12    | `S D F/J K L A / Z X C V B /` | 5K+scratch per side; P2 scratch uses slash |
| 16    | `S D F SPACE J K L A / Z X C V B N M /` | 7K+scratch per side; P2 scratch uses slash |
| 9     | `A S D F SPACE J K L ;`        | PMS buttons 1–9; spacebar is the center |
| 10    | `S D F K L / Z X C N M` | five keys per side |
| 11, 13, 15 | first N keys of `S D F SPACE J K L Z X C V B N M , A` | asymmetric DP |
| 14    | `S D F SPACE J K L / Z X C V B N M` | 7K SP layout on the home row for P1, ZXCVBNM below for P2 |

## License

MIT
