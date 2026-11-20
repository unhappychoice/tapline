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
If that directory contains no charts, the built-in chart plays; discovery does
not continue to later directories. The same fallback applies to an empty or
missing explicit `--dir`.

Direct-file and built-in play exit after the result screen. Playing from the
selector returns there after the result screen.

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

## Audio

All of these flags default to off.

| Option | Behavior |
| ------ | -------- |
| `--no-audio` | Disable gameplay audio and selector previews. |
| `--synth` | Use per-lane synthesized beeps for note hits instead of chart keysounds; BGM still plays. Synth is also used automatically when no samples are loaded. |
| `--auto-ks` | Play note sounds at their scheduled times instead of on key presses. Judgments still require player input. |
| `--test-tone` | Check the audio backend, play a short sequence of beeps, and exit without entering the selector or game. |

`--test-tone` takes precedence over chart selection and uses its own audio path,
so `--no-audio` does not silence it. If the audio backend is unavailable, it
prints `audio backend unavailable` to standard error and exits with status `1`.

## Help and version

| Option | Behavior |
| ------ | -------- |
| `-h, --help` | Print help and exit. |
| `-V, --version` | Print the package version and exit. |

For key bindings, chart-format support, random seeds, and personal-best storage
settings, see the [README](https://github.com/unhappychoice/tapline/blob/main/README.md).
