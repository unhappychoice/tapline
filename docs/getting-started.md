# Getting started

Play the built-in practice chart first, then try the bundled songs or your own
BMS / bmson collection.

## Install from source

You need Git and a stable Rust toolchain with Cargo. CI builds on Linux, macOS,
and Windows using stable Rust. Run the game in an interactive terminal: tapline
uses raw keyboard input and an alternate screen.

On Ubuntu, install the audio development headers before building, as CI does:

```sh
sudo apt-get update
sudo apt-get install -y libasound2-dev
```

The audio dependency is compiled even if you plan to play with `--no-audio`.

Clone the repository and install the binary:

```sh
git clone https://github.com/unhappychoice/tapline.git
cd tapline
cargo install --locked --path .
tapline --help
```

Make sure Cargo's binary directory is on your `PATH`. The commands below assume
you are still in the cloned repository so the bundled `songs/` directory is
available.

## Play your first chart

```sh
tapline --built-in
```

After a two-second countdown, notes fall toward the judgment line. The built-in
chart has four lanes: press `S`, `D`, `K`, and `L` from left to right as the notes
cross the line. Letter keys work in either case. Hits build your score and combo;
misses reset the combo and reduce the gauge.

Finish the chart to see your score, accuracy, and gauge result. A final gauge of
at least 80% clears the chart. Press any key on the result screen to exit.
Pressing `Esc` or `Q` during play ends the chart early with a FAILED result.

For a slower practice chart, or silent play:

```sh
tapline --built-in --bpm 90
tapline --built-in --no-audio
```

`--bpm` changes only the built-in chart; loaded files use their own timing.

## Try the bundled songs

```sh
tapline --dir ./songs
```

Use `Up` / `Down` or `k` / `j` to select a chart, then press `Enter` to play.
Start with `First Steps`, a four-lane beginner chart. The bundled charts use
synthesized sounds and need no separate audio assets. Other charts use different
lane counts; follow the keys shown below the lanes.

After a song, press any key on the result screen to return to the selector.
Use `Esc` or `q` in the selector to exit.

## Load your own charts

Point the selector at your collection, or load a single file directly:

```sh
tapline --dir ~/bms
tapline --file tests/fixtures/minimal.bmson
```

Keep charts with their referenced audio and artwork. The selector scans
subdirectories up to five levels deep for `.bms`, `.bml`, `.bme`, `.pms`, and
`.bmson` files. If the chosen directory has no charts, tapline plays the built-in
chart instead. Loading a file directly exits after its result screen.

See the [README](../README.md) for default song discovery paths, supported chart
features, key layouts, judgment windows, and audio delay adjustments.
