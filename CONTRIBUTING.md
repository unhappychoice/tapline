# Contributing

See the [architecture guide](docs/architecture.md) for the session flow, module
responsibilities, and starting points for changes.

## Development environment

You need Git and a stable Rust toolchain with Cargo.

On Ubuntu / Debian, install the audio development headers (the audio
dependency is compiled even for checks that don't touch audio):

```sh
sudo apt-get update
sudo apt-get install -y libasound2-dev
```

Video BGA tests additionally need `ffmpeg` on `PATH`:

```sh
sudo apt-get install -y ffmpeg
```

Clone the repository:

```sh
git clone https://github.com/unhappychoice/tapline.git
cd tapline
```

## Running the checks

CI runs these on every push and pull request (Linux, macOS, and Windows for
build & test; Linux for lint and coverage):

```sh
cargo build --locked --release
cargo test --locked --all
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
```

`RUSTFLAGS=-D warnings` is set in CI, so compiler warnings fail the build
too. Coverage is measured with `cargo llvm-cov` but isn't required to pass
locally before opening a pull request.

## Opening a pull request

- Keep changes focused; prefer several small pull requests over one large one.
- Add or update tests for behavior you change.
- Write commit messages and pull request titles as
  [Conventional Commits](https://www.conventionalcommits.org/) (`fix:`,
  `feat:`, `docs:`, `refactor:`, ...), matching the existing commit history.
- Make sure `cargo fmt`, `cargo clippy`, and `cargo test` pass before
  submitting.
