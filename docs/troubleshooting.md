# Troubleshooting

## Tapline plays the practice chart instead of opening my song directory

Run `tapline --dir ~/bms` with the path to your collection. Tapline looks for
`.bms`, `.bml`, `.bme`, `.pms`, and `.bmson` files up to five levels deep.
If the initial scan finds no playable charts, tapline plays the built-in
chart. For automatic directory selection, see the
[CLI reference](cli-reference.md#chart-selection).

If the selector is open but shows no matches, press `Enter` or `Esc` to
finish editing the search if it is active, then press `c` to clear the search
and lane filters. When the selector reopens after a song, it rescans the
directory. If the list is still empty, check that the chart files are still
accessible, then restart tapline to scan again.

## A large chart is missing from the selector

Tapline skips chart files larger than 16 MiB (16,777,216 bytes) during
discovery. Opening the same file with `tapline --file path/to/chart.bms`
reports the size limit. Use a chart file at or below the limit. Referenced
audio and artwork files are separate from the chart file and do not count
toward its size.

## The playfield disappeared after I resized my terminal

Enlarge the terminal to the dimensions shown in the size notice. Gameplay
needs 13 rows and at least `7 × lane count + 1` columns. Chart time, judgments,
and audio pause until the terminal fits again.

## I cannot hear the game

Run `tapline --test-tone` to check whether tapline can open an audio output.
If it prints `audio backend unavailable`, check that an output device is
available to the terminal session. You can still play silently with
`tapline --built-in --no-audio`.

If the test tone plays but your chart is missing its recorded sounds, check
that its referenced audio files are present. Run your chart without
`--no-audio` for sound and without `--synth` to hear recorded note sounds.
For sounds that arrive late, see
[Fixing audio delay](https://github.com/unhappychoice/tapline/blob/main/README.md#fixing-audio-delay).

## A chart's background video is missing

Make sure `ffmpeg` is on `PATH` in the terminal where you run tapline and
that the chart's video file is present and decodable. Make the terminal at
least 17 rows tall and wide enough to leave room to the left of the lanes
for the background animation.
