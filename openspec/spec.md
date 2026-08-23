# openporta Specification

Version 1.1 (amended by change 001, stereo bounce buss - see
`openspec/changes/001-stereo-repeatable-bounce.md` for the full design
and its 13-revision review history). This document is the constitution
of the project. The decisions
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

- 4 mono tracks, one stereo master output, plus one fixed, mix-only
  stereo bounce buss (not a 5th track: no arm for live input, no pan,
  exists only to receive a printed mix, cannot be added to or removed -
  REQ-101/404)
- Record, overdub, punch-in/out, destructive real-time bounce onto the
  buss, undo
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
- Tape: the audio storage. 4 fixed-length mono i16 buffers plus one
  fixed-length stereo i16 buffer (the bounce buss), all at 48kHz.
- Record pass: one continuous record engagement on one track or the
  bounce buss, from punch-in to punch-out. The unit of undo. A pass onto
  the buss writes both channels atomically as one pass for undo purposes
  (REQ-502).
- Bounce: a real-time record pass onto the dedicated stereo bounce buss,
  whose input is the pre-master-fader sum of tracks 1-4 (at their live
  fader/pan/mute) plus the buss's own existing content (at its own
  fader/mute).
- Generation loss: cumulative degradation from repeated record passes over
  the same material.
- Character chain: the DSP applied to audio on its way to tape.

## 4. Functional requirements

Requirements use RFC 2119 language. Every requirement MUST be verifiable by
`cargo test` without audio hardware unless marked [manual].

### 4.1 Tape and cassette

- REQ-101 A cassette MUST have exactly 4 mono tracks and a fixed tape length
  set at creation (default 15 minutes, max 30), plus one always-stereo
  bounce buss with a different capability set (mix-only input, no arm for
  ordinary recording, mutually exclusive with tracks 1-4's arm state -
  REQ-404/405). The 4-mono-track guarantee for tracks 1-4 themselves is
  unchanged.
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

- REQ-301 Recording MUST engage only on armed tracks or the armed bounce
  buss, and MUST overwrite tape content (destructive).
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
  pass. Symmetrically, the bounce buss MUST be byte-identical across an
  ordinary track pass, and tracks 1-4 MUST be byte-identical across a
  bounce (both already guaranteed by REQ-405's mutual exclusivity).

### 4.4 Bounce

- REQ-401 Bounce MUST be implemented as a real-time record pass onto the
  dedicated stereo bounce buss, whose input is the pre-master-fader sum
  of tracks 1-4 (each at its live fader/pan/mute) plus the buss's own
  existing content (at its own fader/mute). There is no separate
  blocking bounce command: bouncing is arming the buss and pressing
  Record.
- REQ-402 Bounce MUST apply the character chain (generation loss
  compounds): each channel runs its own independent stage set
  (saturation, hiss, bandwidth, optional crush) with wow/flutter
  modulation shared between L and R (one modulation instance, two delay
  lines - the stereo image wobbles together, as one transport would).
- REQ-403 Three successive bounce generations of broadband material MUST
  show monotonically decreasing high-frequency band energy and monotonically
  increasing noise floor. This is the product acceptance test.
- REQ-404 The bounce buss MUST have its own arm-like flag, independent
  of tracks 1-4's armed state, with no ordinary-input recording
  capability.
- REQ-405 Arming the bounce buss and arming any of tracks 1-4 MUST be
  mutually exclusive; arming one MUST clear the other. A direct
  consequence: no track's live input can be monitored while a bounce
  pass is open (input-monitor preview requires an armed track).
  Intended, not an oversight.
- REQ-406 The master fader MUST NOT be baked into any signal written to
  tape (tracks 1-4 or the bounce buss); a bounce pass's input MUST be
  computed before any master-fader multiplication.
- REQ-407 A bounce pass's own prior content at a given tape position
  MUST be read before the pass's new value is written to that position
  (block-local read-before-write; no lookahead).
- REQ-408 While a bounce pass is open, tracks 1-4's own contribution to
  the audible output MUST be silent; the buss's contribution MUST be the
  pass's post-chain printed signal flowing through the buss's own
  smoothed fader/mute, the same mixer path ordinary playback uses. The
  buss's smoothed gain value MUST be computed once per sample and reused
  for both its contribution to the print input (REQ-406) and the monitor
  output at that same sample position - never advanced twice for one
  sample. Track-level metering MUST NOT be silenced by this - it keeps
  reflecting each track's own playback contribution (post-fader,
  pre-pan).
- REQ-409 The bounce buss MUST have its own volume fader and mute,
  independent of tracks 1-4's (REQ-601) - no pan, since it is already
  stereo. Both MUST be smoothed the same way every other mixer control
  is, and both MUST persist in the project manifest. REQ-406's carve-out
  to REQ-602 (controls baked into the print during a bounce) extends to
  the buss's own fader/mute.

### 4.5 Undo

- REQ-501 Every record pass (including bounces) MUST be undoable and
  redoable, restoring the affected tape region byte-exactly.
- REQ-502 The undo journal MUST be bounded (configurable cap, default 32
  passes / 512MB on disk) with oldest-first eviction. A stereo bounce
  pass MUST journal as a single atomic entry spanning both channels -
  one undo press fully reverts a bounce; no reachable state has one
  channel reverted and the other not.
- REQ-503 Undo state MUST survive save/load of the project.
- REQ-504 Undo and redo MUST be rejected while the transport is not Stopped.
- REQ-505 The destructive UX MUST NOT be weakened by the journal: no
  visible history browser, no multi-level "restore take" UI. Undo/redo
  buttons only.

### 4.6 Mixer

- REQ-601 Each track MUST have a volume fader (dB) and pan (equal-power
  law); the master MUST have a volume fader.
- REQ-602 Mixer moves MUST be non-destructive (playback-side only) and MUST
  be smoothed so parameter jumps produce no clicks. One narrow, explicit
  carve-out: while feeding an active bounce pass, tracks 1-4's
  fader/pan/mute and the buss's own fader/mute ARE baked into what gets
  printed (that is the point of printing a mix - REQ-401/406/409); the
  controls themselves stay non-destructively adjustable afterward.
- (REQ-603 deleted by change 001: bounce is no longer a mono sum, and
  tracks keep their real pan while feeding a bounce pass. The id is
  retired, not reused.)

### 4.7 Character chain

- REQ-701 The record path MUST apply, in order: saturation (tanh with drive
  and makeup gain), bandwidth limiting (low-pass near 10kHz, high-pass near
  60Hz), wow/flutter (0.5-5Hz LFO plus random drift on a fractional delay),
  hiss (seeded, filtered noise), then optional bitcrush/sample-rate
  reduction (default off), then TPDF dither to i16.
- REQ-702 All stochastic elements MUST derive from the cassette noise seed
  plus the pass id (plus a channel term for a stereo bounce pass: hiss
  and dither seed per channel via `seed_for(noise_seed, pass, channel)`;
  the single shared flutter modulator seeds at channel term 0); two renders of the same session script MUST be
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
  track files (including the bounce buss's two channel files, whose
  absence reads as "never bounced yet" so pre-buss cassettes open
  unchanged), and the undo journal. The manifest persists the buss's
  fader/mute alongside the per-track mixer state.
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
- REQ-904 Resident memory (tape storage plus every realtime-safety
  reserve) MUST stay at or below ~1.8GB steady-state / ~2.8GB transient
  peak worst case (30-minute cassette, the transient only while undoing
  a full-length stereo bounce); the default 15-minute cassette uses
  roughly half each figure. Verified against the deployment Pi's real
  headroom (8GB, ~5.8GB free at idle); if a smaller-RAM target is ever
  adopted, this requirement MUST be recomputed against it. (Amended by
  change 001 from ~700MB, which counted tape buffers alone.)
- REQ-905 Realtime operation on the Pi SHOULD target a 128-256 frame period
  at 48kHz; 64 frames is NOT a requirement.
- REQ-906 The full test suite MUST pass headlessly in CI on every commit.

## 6. Acceptance gates per milestone

- M1: scripted record/playback roundtrip; punch boundaries click-free;
  undo restores byte-exactly; unarmed tracks untouched.
- M2: REQ-403 generation-loss test passes (procedure rewritten under
  change 001 for the buss-based bounce); all DSP numeric assertions
  pass; bit-reproducible renders.
- M3: export matches playback; the single golden render passes (golden
  regenerated once under change 001 - old `{"op":"bounce"}` scripts and
  the pre-master mixer refactor both perturb it).
- M4: block-size invariance (REQ-203) under simulated realtime; manual
  smoke test on macOS.
- M5: UI drives the engine through the command queue only.
- M6: on-device Pi smoke test with measured callback headroom documented.
