# CLI reference

Run `tapline [OPTIONS]` in an interactive terminal. Use `tapline --help` for
the installed binary's options, or `tapline --version` for its version.
See [Getting started](getting-started.md) for installation and a first playthrough.

## Chart selection

| Option | Default | Behavior |
| ------ | ------- | -------- |
| `-f, --file <FILE>` | Unset | Load one BMS or bmson chart directly. |
| `-d, --dir <DIR>` | Automatic discovery | Open the selector on a directory, scanning recursively up to five levels deep. |
| `--built-in` | Off | Play the built-in four-lane practice chart. |

Selection uses this priority: `--built-in`, then `--file`, then `--dir`, then
automatic directory discovery. These options can be combined, but only the
highest-priority choice is used.

Automatic discovery chooses the first existing directory from
`$TAPLINE_SONGS_DIR`, `./songs`, `./tests/fixtures`, and `$HOME/.tapline/songs`.
On Windows, `%USERPROFILE%\.tapline\songs` is the final fallback if the
`HOME` directory candidate is unavailable.
If the initial scan finds no playable charts, tapline plays the built-in
chart. Discovery stops at the first existing directory. An empty or missing
explicit `--dir` also leads to the built-in chart on the initial scan.

Direct-file and built-in play exit after the result screen. When you return
from the result screen to the selector, tapline rescans the directory and
shows the selector even if no playable charts remain.

## Gameplay keys

Use `--keymap <FILE>` to load a local JSON file of gameplay bindings at
startup. See the [keymap format and examples](https://github.com/unhappychoice/tapline/blob/main/README.md#custom-gameplay-keys).

## Timing

| Option | Default | Behavior |
| ------ | ------- | -------- |
| `-b, --bpm <BPM>` | `120` | Tempo of the built-in chart; loaded charts use their own timing. |
| `--countdown-ms <COUNTDOWN_MS>` | `2000` | Countdown before a chart starts, in milliseconds; `0` skips the delay. |
| `--audio-lead-ms <AUDIO_LEAD_MS>` | `0` | Schedule BGM and automatic keysounds this many milliseconds earlier. |

BPM must be finite and positive; countdown must be finite and non-negative.
Their combined values must also produce finite built-in chart timing. Validation
runs even when loading a chart file. Audio lead must be finite; negative values
delay scheduled audio. Use the equals form for negative values, for example
`--audio-lead-ms=-50`.

Audio lead does not change judgment timing or advance sounds triggered by key
presses. Pair it with `--auto-ks` to adjust scheduled note sounds:

```sh
tapline --file song.bms --auto-ks --audio-lead-ms 80
tapline --built-in --bpm 90 --countdown-ms 500
```

## Play modifiers

| Option | Default | Behavior |
| ------ | ------- | -------- |
| `--modifier <off\|random\|mirror>` | `off` | Leave the chart's key-lane layout unchanged, shuffle it once per play, or reverse it. |

Both `random` and `mirror` only permute each player's `Key`-role lanes;
scratch lanes and double-play side boundaries are fixed points, and key
bindings and keysounds stay with their lane. Retrying with `R` keeps the
same shuffle; loading a chart again (including from the selector) draws a
new one. The active modifier appears in the HUD. Set `TAPLINE_MODIFIER_SEED`
to reproduce a `random` shuffle, the same way `TAPLINE_RANDOM_SEED`
reproduces a BMS branch draw.

## Note display

Use `--scroll-speed <SCROLL_SPEED>` to set the note scroll multiplier for
built-in and imported charts. The default is `1.0` and the accepted range is
`0.25` to `4.0` inclusive. For example, `--scroll-speed 2` spreads notes
farther apart with less preview time, while `--scroll-speed 0.5` shows notes
earlier. The multiplier combines with chart-authored scroll and spacing
effects. Audio, judgments, and scoring follow the original chart timing.
Retries use the same launch setting.

## Audio

All of these flags default to off.

| Option | Behavior |
| ------ | -------- |
| `--no-audio` | Disable gameplay audio and selector previews. |
| `--synth` | Use per-lane synthesized beeps for note hits instead of chart keysounds; BGM still plays. |
| `--auto-ks` | Play note sounds at their scheduled times instead of on key presses. Judgments still require player input. |
| `--test-tone` | Check the audio backend, play a short sequence of beeps, and exit without entering the selector or game. |

Automatic synth fallback applies only when no chart samples decode. Decoded
background music (BGM) counts toward the sample bank. If only the BGM decodes,
pass `--synth` to hear synthesized note sounds alongside the BGM.

`--test-tone` takes precedence over chart selection and uses its own audio path,
so `--no-audio` does not silence it. If the audio backend is unavailable, it
prints `audio backend unavailable` to standard error and exits with status `1`.

## Help and version

| Option | Behavior |
| ------ | -------- |
| `-h, --help` | Print help and exit. |
| `-V, --version` | Print the package version and exit. |

## Environment variables

| Variable | Behavior |
| -------- | -------- |
| `TAPLINE_SONGS_DIR` | First directory checked during [automatic chart discovery](#chart-selection). If the path is missing or is not a directory, discovery continues with the next candidate. |
| `TAPLINE_DATA_DIR` | Directory for `results.json`, overriding the platform default. |
| `TAPLINE_RANDOM_SEED` | Unsigned 64-bit integer used for BMS branch selection. If unset or invalid, tapline generates a seed. Reuse the same seed with an unchanged chart to reproduce the branch in the selector or with `--file`. |
| `TAPLINE_MODIFIER_SEED` | Unsigned 64-bit integer used for `--modifier random` lane shuffling. If unset or invalid, tapline generates a seed. Reuse the same seed to reproduce the same shuffle. |

Without `TAPLINE_DATA_DIR`, tapline looks for `results.json` in these directories:

- Linux: `$XDG_DATA_HOME/tapline` if `XDG_DATA_HOME` is an absolute path,
  otherwise `$HOME/.local/share/tapline`.
- macOS: `$HOME/Library/Application Support/tapline`.
- Windows: `%LOCALAPPDATA%\tapline`.

For chart-format support and how records are tracked, see the
[README](https://github.com/unhappychoice/tapline/blob/main/README.md).
