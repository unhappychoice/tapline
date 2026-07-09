# tapline

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
tapline                                # built-in 120 BPM chart, 4-lane
tapline --bpm 160                      # built-in, faster
tapline --file song.bms                # BMS chart (auto 5-lane / 7-lane)
tapline --file song.bms --no-audio     # BMS chart, silent
```

## BMS support

Encoding is auto-detected (UTF-8 → fallback Shift-JIS).

### Supported

| category      | items                                                       |
| ------------- | ----------------------------------------------------------- |
| headers       | `#TITLE`, `#ARTIST`, `#GENRE`, `#BPM`, `#PLAYLEVEL`, `#WAVxx` |
| channels      | `01` (BGM auto-play), `11–19` (P1 visible notes)            |
| lane modes    | 4-key, 5-key (`11–15`), 7-key (`11–15 + 18/19`) auto-detect |
| audio formats | WAV / OGG / MP3 (extension fallback on lookup)              |
| polyphony     | multiple `#00101` lines mix concurrently                    |

### Not yet supported (v1)

- BPM changes (channels `03`, `08`)
- STOP (`09`) and measure-length change (`02`)
- Long notes (`51–59`, `LNOBJ`, `LNTYPE`)
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

| lanes | keys              |
| ----- | ----------------- |
| 4     | `D F J K`         |
| 5     | `D F G J K`       |
| 7     | `S D F SPACE J K L` |

## License

MIT
