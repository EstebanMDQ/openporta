# openporta

A software emulation of a 4-track cassette portastudio.

Four mono tracks, a fixed set of controls, and a destructive workflow.
You record over things. You bounce three tracks down to one to free them
up, and the bounce costs you a generation. The constraint is the point:
this is an instrument, not a DAW.

## What it does

- **Four mono tracks, one stereo master.** No more, ever.
- **Destructive recording.** Recording over a track erases it. Bouncing
  tracks 1-3 onto track 4 overwrites track 4.
- **Real generation loss.** Tape character is printed at record time, so
  every bounce saturates, dulls, and wobbles the material again, and the
  noise floor climbs. Three generations sound like three generations.
- **Undo anyway.** The destructive feel is real, but a hidden journal
  keeps every record pass, so a mistake is recoverable. There is no
  history browser; just undo and redo.
- **Cassette character**: tape saturation, an 11kHz top end, wow and
  flutter that decorrelates between passes, and hiss printed inside the
  passband so it accumulates the way real hiss does. Bitcrush is
  available and off by default.

Out of scope, deliberately: MIDI, network sync, plugins, variable track
counts, non-destructive editing. See `openspec/spec.md`.

## Status

The engine is complete and headlessly tested. The Slint UI drives it
through the command queue, with a silent timer standing in for the
audio thread until the realtime adapter is wired into it (M5.5).

| Milestone | State |
|-----------|-------|
| M0 scaffolding, CI, test instruments | done |
| M1 tape engine: transport, record, punch, undo, persistence | done |
| M2 lo-fi DSP and generation loss | done |
| M3 bounce, mixdown, WAV export, CLI | done |
| M4 realtime audio (cpal) | verified on macOS hardware |
| M5 Slint UI: transport, track strips, meters, save/undo/export, cassette new/load | done; real audio in the UI is M5.5 |
| M6 Raspberry Pi deployment | not started |

Today you drive it through session scripts, the CLI, or the UI.
`TASKS.md` is the queue.

## Try it

```bash
# make a cassette and record something onto track 1
cargo run -p porta-app -- new mytape.porta --minutes 5
cargo run -p porta-app -- script session.json
cargo run -p porta-app -- render mytape.porta --out mix.wav --bits 24
```

A session script is a list of ops:

```json
{"ops": [
  {"op": "new", "dir": "mytape.porta", "minutes": 5, "seed": 1979},
  {"op": "arm", "track": 0},
  {"op": "record", "input_wav": "guitar.wav"},
  {"op": "arm", "track": 0, "on": false},
  {"op": "fader", "track": 0, "db": -3.0},
  {"op": "pan", "track": 0, "value": -0.4},
  {"op": "bounce"},
  {"op": "seek", "seconds": 0},
  {"op": "export", "out": "discard.wav"},
  {"op": "play", "seconds": 30},
  {"op": "export", "out": "mix.wav"},
  {"op": "save"}
]}
```

`export` writes whatever the machine has played since the previous
export, which is why the example throws one away before the take it
wants. `character` on the `new` op accepts `cassette` (default) or
`clean`, the latter being useful when you want the mechanics without the
colour.

With real audio hardware:

```bash
cargo run -p porta-app --features realtime -- devices
cargo run -p porta-app --features realtime -- live mytape.porta --period 256
```

The Slint UI (no real audio yet - see Status above):

```bash
cargo run -p porta-app --features ui -- ui mytape.porta
```

## Releases

Tagged releases (`vX.Y.Z`) build `porta-app` for macOS (Apple Silicon
and Intel), Linux (x86_64 and aarch64), and Windows, with both the
`realtime` and `ui` features on - see the Actions tab, or
`.github/workflows/release.yml`.

## Layout

```
crates/porta-dsp/      tape character: saturation, bandwidth, flutter, hiss
crates/porta-engine/   tape, transport, record passes, undo, mixer, projects
crates/porta-testkit/  test instruments: generators, meters, FFT, click detector
crates/porta-app/      CLI, session scripts, WAV export, realtime adapter
openspec/spec.md       the settled requirements
docs/manual-checklist.md  what only a human with hardware can verify
```

The engine knows nothing about audio hardware: buffers in, buffers out.
That is what lets the whole thing be tested without a sound card, and
what will let it run on a Pi without the engine noticing.

## Development

```bash
cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
```

On a machine with no Rust toolchain, `scripts/cargo-docker.sh` runs the
same commands in a container.

Audio correctness is verified by rendering offline and measuring:
RMS windows, band energy, total harmonic distortion, pitch deviation in
cents, and a click detector that catches discontinuities no listener is
present to hear. One golden render pins the exact sound of a full
session; if it changes, something changed, and the reason belongs in
`TASKS.md` before it is blessed.

## A cassette on disk

```
mytape.porta/
  manifest.json        tape length, character and seed, mixer settings
  tape/track{0..3}.raw raw 16-bit samples, saved in 5-second chunks
  undo/                the journal that makes undo possible
```

Saves rewrite only the chunks that changed, and never happen while the
tape is rolling.
