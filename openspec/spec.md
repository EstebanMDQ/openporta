# openporta Specification

Version 1.0. This document is the constitution of the project. The decisions
in it are settled. Changing user-visible behavior or reversing a settled
decision REQUIRES a proposal in `openspec/changes/` reviewed by the
spec-reviewer agent, and a notification to the owner BEFORE implementation.
Engine internals that do not change these requirements need no proposal.

## 1. Purpose

openporta is a software emulation of a classic 4-track cassette portastudio
(in the spirit of the Tascam Porta 424). The constraint IS the product: a
fixed number of tracks, a fixed small control set, a destructive workflow,
and lo-fi tape sound. It is an instrument, not a DAW.

Target users: musicians who want the commitment and character of a cassette
4-track without maintaining 40-year-old hardware.

## 2. Scope

In scope for v1:

- 4 mono tracks, one stereo master output
- Record, overdub, punch-in/out, destructive bounce, undo
- Baked-in tape character with generation loss
- Project persistence and WAV export
- Offline (headless) operation, realtime operation on macOS, then
  Raspberry Pi 4
- Slint UI (last milestone)

Explicitly OUT of v1 (do not implement, do not prepare abstractions for):

- MIDI (input, output, sync)
- Ableton Link or any network sync
- Plugins or effect inserts of any kind
- Variable track counts, track groups, scenes
- Non-destructive editing, clip launching, waveform editing
- Cloud anything

## 3. Definitions

- Cassette: a project. Has a fixed tape length and a fixed TapeCharacter
  chosen at creation.
- Tape: the audio storage. 4 fixed-length mono i16 buffers at 48kHz.
- Record pass: one continuous record engagement on one track, from punch-in
  to punch-out. The unit of undo.
- Bounce: recording the post-fader mono sum of tracks 1-3 onto track 4.
- Generation loss: cumulative degradation from repeated record passes over
  the same material.
- Character chain: the DSP applied to audio on its way to tape.

## 4. Functional requirements

Requirements use RFC 2119 language. Every requirement MUST be verifiable by
`cargo test` without audio hardware unless marked [manual].

### 4.1 Tape and cassette

- REQ-101 A cassette MUST have exactly 4 mono tracks and a fixed tape length
  set at creation (default 15 minutes, max 30).
- REQ-102 Tape audio MUST be stored as i16 at 48kHz. The record path MUST
  apply TPDF dither before quantization.
- REQ-103 A cassette's TapeCharacter (including noise seed) MUST be fixed at
  creation and stored in the project manifest.
- REQ-104 Recording past the end of the tape MUST stop the transport, not
  wrap or extend.

### 4.2 Transport

- REQ-201 The transport MUST implement states Stopped, Playing, Recording
  with a sample-accurate playhead.
- REQ-202 Seek, rewind, and fast-forward MUST be instant (no simulated
  spooling) in v1.
- REQ-203 The playhead position MUST be identical for identical command
  sequences regardless of processing block size.

### 4.3 Recording

- REQ-301 Recording MUST engage only on armed tracks and MUST overwrite
  tape content (destructive).
- REQ-302 Punch-in and punch-out boundaries MUST use a 5ms linear crossfade
  between old tape content and new signal; boundaries MUST NOT produce
  clicks detectable by the testkit click detector.
- REQ-303 Each record pass MUST run the full character chain before
  quantization, so degradation is baked onto tape.
- REQ-304 Wow/flutter modulation MUST be seeded per record pass so
  successive passes are decorrelated.
- REQ-305 Monitoring while recording MUST be post-chain (the user hears what
  the tape receives). [manual for the listening part; routing is testable]
- REQ-306 Unarmed tracks MUST be byte-identical before and after any record
  pass.

### 4.4 Bounce

- REQ-401 Bounce MUST be implemented as a record pass whose input is the
  engine's post-fader mono sum of tracks 1-3, recorded onto track 4.
- REQ-402 Bounce MUST apply the character chain (generation loss compounds).
- REQ-403 Three successive bounce generations of broadband material MUST
  show monotonically decreasing high-frequency band energy and monotonically
  increasing noise floor. This is the product acceptance test.

### 4.5 Undo

- REQ-501 Every record pass (including bounces) MUST be undoable and
  redoable, restoring the affected tape region byte-exactly.
- REQ-502 The undo journal MUST be bounded (configurable cap, default 32
  passes / 512MB on disk) with oldest-first eviction.
- REQ-503 Undo state MUST survive save/load of the project.
- REQ-504 Undo and redo MUST be rejected while the transport is not Stopped.
- REQ-505 The destructive UX MUST NOT be weakened by the journal: no
  visible history browser, no multi-level "restore take" UI. Undo/redo
  buttons only.

### 4.6 Mixer

- REQ-601 Each track MUST have a volume fader (dB) and pan (equal-power
  law); the master MUST have a volume fader.
- REQ-602 Mixer moves MUST be non-destructive (playback-side only) and MUST
  be smoothed so parameter jumps produce no clicks.
- REQ-603 During bounce, pans MUST be ignored (mono sum), matching the
  reference hardware's bus behavior.

### 4.7 Character chain

- REQ-701 The record path MUST apply, in order: saturation (tanh with drive
  and makeup gain), bandwidth limiting (low-pass near 10kHz, high-pass near
  60Hz), wow/flutter (0.5-5Hz LFO plus random drift on a fractional delay),
  hiss (seeded, filtered noise), then optional bitcrush/sample-rate
  reduction (default off), then TPDF dither to i16.
- REQ-702 All stochastic elements MUST derive from the cassette noise seed
  plus the pass id; two renders of the same session script MUST be
  bit-identical on the same machine and toolchain. Across platforms the
  guarantee is weaker by necessity: libm transcendentals (tanh, sin, exp,
  powf) differ in the last bits between implementations, so renders MAY
  differ by a couple of LSBs. Anything larger is a defect.
- REQ-703 Processors MUST NOT allocate, lock, or perform I/O inside their
  process call.
- REQ-704 The chain MUST be swappable behind the AudioProcessor trait; a
  passthrough chain MUST be available for engine testing.

### 4.8 Persistence and export

- REQ-801 A project MUST be a directory containing a JSON manifest, raw i16
  track files, and the undo journal.
- REQ-802 Saves MUST write only dirty 5-second chunks of tape and MUST occur
  only on explicit save or transport stop, never during recording.
- REQ-803 Export MUST produce a WAV mixdown (16-bit default, 24-bit option)
  whose audio matches engine playback of the same session.
- REQ-804 A JSON session-script format MUST be able to drive the engine
  headlessly (seek, arm, record from WAV input, bounce, undo, set mixer
  params, export).

## 5. Non-functional requirements

- REQ-901 The engine and DSP crates MUST NOT depend on audio hardware,
  cpal, or UI libraries. The realtime adapter and UI are feature-gated in
  the app crate.
- REQ-902 The realtime audio callback MUST NOT allocate, lock, or perform
  disk I/O. Control communication MUST use wait-free queues. Disk work MUST
  be stop-gated.
- REQ-903 Platforms: macOS (Apple Silicon) for development, Raspberry Pi 4
  (aarch64, ALSA, class-compliant USB interface) for deployment. Linux x86
  MUST stay green in CI.
- REQ-904 Resident tape memory MUST stay at or below ~700MB worst case
  (30-minute cassette); the default 15-minute cassette uses ~346MB.
- REQ-905 Realtime operation on the Pi SHOULD target a 128-256 frame period
  at 48kHz; 64 frames is NOT a requirement.
- REQ-906 The full test suite MUST pass headlessly in CI on every commit.

## 6. Acceptance gates per milestone

- M1: scripted record/playback roundtrip; punch boundaries click-free;
  undo restores byte-exactly; unarmed tracks untouched.
- M2: REQ-403 generation-loss test passes; all DSP numeric assertions pass;
  bit-reproducible renders.
- M3: export matches playback; the single golden render passes.
- M4: block-size invariance (REQ-203) under simulated realtime; manual
  smoke test on macOS.
- M5: UI drives the engine through the command queue only.
- M6: on-device Pi smoke test with measured callback headroom documented.
