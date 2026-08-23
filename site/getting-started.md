---
title: Getting started
description: Running openporta from the command line, session scripts, and where to get a build.
date: 2026-08-22
---

***English** · [Español](es/getting-started.html)*

# Getting started

openporta is a Rust workspace. There's no installer - build it, or grab
a release build, and point it at a directory to hold a cassette.

## The three ways to drive it

**Headlessly**, via a session script - the way every test in the
project itself works:

```bash
cargo run -p porta-app -- new mytape.porta --minutes 5
cargo run -p porta-app -- script session.json
cargo run -p porta-app -- render mytape.porta --out mix.wav --bits 24
```

A session script is a plain JSON list of operations:

```json
{"ops": [
  {"op": "new", "dir": "mytape.porta", "minutes": 5, "seed": 1979},
  {"op": "arm", "track": 0},
  {"op": "record", "input_wav": "guitar.wav"},
  {"op": "arm", "track": 0, "on": false},
  {"op": "fader", "track": 0, "db": -3.0},
  {"op": "pan", "track": 0, "value": -0.4},
  {"op": "bounce_arm"},
  {"op": "seek", "seconds": 0},
  {"op": "bounce", "seconds": 30},
  {"op": "bounce_arm", "on": false},
  {"op": "seek", "seconds": 0},
  {"op": "export", "out": "discard.wav"},
  {"op": "play", "seconds": 30},
  {"op": "export", "out": "mix.wav"},
  {"op": "save"}
]}
```

Bouncing is arming the bus and rolling the transport, not a batch
command: the mix prints in real time onto a dedicated stereo bounce
bus, so faders and pans can be ridden while it goes down, and the bus
keeps its own prior content - bounce again and the previous generation
folds forward rather than being replaced.

`export` writes whatever the machine has played since the *previous*
export, which is why the example above throws one away before the take
it actually wants - the tape has to roll past the material once before
there's anything new to capture. The `character` field on the `new` op
accepts `cassette` (the default lo-fi formulation) or `clean`, useful
when you want the transport and record mechanics without the colour.

**With real audio hardware**, via cpal:

```bash
cargo run -p porta-app --features realtime -- devices
cargo run -p porta-app --features realtime -- live mytape.porta --period 256
```

`devices` lists what's actually available and by what name to address
it - worth checking first, since some audio backends enumerate the
same physical interface many times over under nearly identical names.

**Through the UI**, with real audio if the `realtime` feature is on too:

```bash
cargo run -p porta-app --features ui,realtime -- ui mytape.porta

# --kiosk runs full-screen and frameless, for a dedicated touchscreen -
# Escape toggles it back off, or flip the same switch from Settings
cargo run -p porta-app --features ui,realtime -- ui mytape.porta --kiosk
```

The UI remembers the last audio device that connected successfully and
reconnects to it automatically at launch - it's built to behave like an
appliance that's ready the moment it's turned on, not a tool that
starts idle and waits to be configured. See [Raspberry Pi
setup](raspberry-pi.md) for the full kiosk story.

## Prebuilt releases

Tagged releases (`vX.Y.Z`) build for macOS (Apple Silicon and Intel),
Linux (x86_64 and aarch64), and Windows, with both the `realtime` and
`ui` features on - see the project's Actions tab for the build matrix.

## A cassette on disk

```
mytape.porta/
  manifest.json        tape length, character and seed, mixer settings
  tape/track{0..3}.raw raw 16-bit samples, saved in 5-second chunks
  tape/bounce_{l,r}.raw the stereo bounce bus, same chunked format
  undo/                the journal that makes undo possible
```

Saves rewrite only the chunks that actually changed, and never happen
while the tape is rolling - REQ-802, if you're reading the spec
directly.

## Building it yourself

```bash
cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
```

That's the whole gate: format, lint with warnings denied, and the full
test suite - the same thing CI runs on every commit, and the same
thing that has to be green before any change lands. On a machine
without a Rust toolchain, a Docker wrapper script runs the identical
commands in a container.
