# Task Queue

Statuses: `[ ]` todo, `[>]` in progress (max one), `[x]` done (checked in
the same commit as the work), `[!]` blocked (one-line reason + date).
Verification is `cargo test --workspace` plus the noted assertions.

## M0 - Scaffolding

- [x] M0.1 Workspace + four crate skeletons, toolchain pin, docker wrapper
- [x] M0.2 CI: fmt, clippy -D warnings, workspace tests
- [x] M0.3 Spec, workflow docs, task queue, rubric agents; delete old spec
- [x] M0.4 testkit: generators (sine/sweep/noise/impulse/silence/dc) +
      RMS/peak meters in dBFS (verify: full-scale sine RMS = -3.01 dBFS)
- [x] M0.5 testkit: FFT band energy, click detector, pitch-deviation probe,
      WAV helpers, assert macros (verify: detector catches injected
      discontinuity, passes clean sine; band energy concentrates at sine
      frequency)

## M1 - Headless tape engine (passthrough chain)

- [x] M1.1 Tape type: fixed-length i16 tracks, region read/write,
      dirty-chunk bitmap (verify: roundtrip, bounds, dirty tracking)
- [x] M1.2 Transport state machine + sample-accurate playhead, seek/rew/ff
      (verify: transition and position tests)
- [x] M1.3 Playback mixer: fader/pan/master with per-block smoothing,
      process_block API (verify: RMS math, equal-power pan at center, no
      clicks on fader jumps)
- [x] M1.4 Record pass: punch crossfades, displaced-audio capture, REQ-306
      (verify: recorded region equality, no clicks across punch points,
      unarmed tracks byte-identical)
- [x] M1.5 Undo/redo stack with disk spill and cap eviction (verify:
      byte-equal restore, redo symmetry, eviction)
- [x] M1.6 Project persistence: manifest + chunked track files, save/load
      (verify: roundtrip equality, only-dirty-chunks-written)
- [x] M1.7 Engine facade + session-script runner + WAV export (verify:
      scripted record-then-play renders expected signal, WAV
      header/length correct, block-size invariance)

## M2 - Lo-fi DSP

- [x] M2.1 Chain ordering + block-size invariance harness in
      porta_dsp::testing (verify: identity, stage order, split
      equivalence across block sizes 1/37/64/128/512/4096)
- [x] M2.2 Saturation (tanh drive + makeup) (verify: THD above/below drive
      threshold, no NaN under abusive input)
- [x] M2.3 Bandwidth limiting: 2x Butterworth LPF biquad @11kHz + HPF
      @60Hz (verify: attenuation and passband assertions). Note: one-pole
      cascades were tried first and rejected - they flatten near Nyquist
      and leave 20kHz only 9 dB down, too bright for cassette.
- [x] M2.4 Hiss: seeded, high-tilted noise (verify: noise-floor window,
      seed determinism, spectral tilt)
- [x] M2.5 Wow/flutter: wow sine + flutter random walk on a fractional
      delay, Catmull-Rom interpolation (verify: pitch-probe deviation in
      cents band, no clicks, latency reported, seeds decorrelate)
- [x] M2.6 Optional bitcrush/sample-rate reduction (verify: quantization
      grid, alias energy, held-sample runs, off by default)
- [x] M2.7 TapeCharacter wired into the record path (fresh chain per
      pass), stored in the manifest, script `character` preset added
      (verify: cassette take has more THD and a higher noise floor than
      a clean take of the same source)
- [x] M2.8 Generation-loss acceptance test REQ-403 (milestone gate)
      (verify: 3 generations show monotonic 8kHz decay and monotonic
      noise-floor rise, renders reproducible, passes decorrelate).
      Note: hiss moved before the bandwidth stage - printed after it,
      most hiss energy sat above the corner and the next generation
      just filtered it away, so the floor barely built up.

## M3 - Bounce, export, CLI

- [x] M3.1 Bounce op: post-fader mono sum of 1-3 recorded to track 4
      (verify: all three tones present in the bounce, sources untouched,
      faders respected and pans ignored, undo byte-exact, character
      applied again, reproducible, refused while rolling)
- [x] M3.2 Mixdown renderer + WAV export, 16-bit default / 24-bit flag
      (verify: script export and render command are byte-identical,
      headers correct at both depths)
- [x] M3.3 porta-app CLI: new/script/render/export subcommands + script
      `bounce` op (verify: cli.rs drives the real binary, bad arguments
      rejected)
- [x] M3.4 The one end-to-end golden render + UPDATE_GOLDEN bless flow
      (verify: full session - 3 overdubs, bounce, punch, undo/redo,
      mixer moves - matches tests/golden/session.wav sample-exactly).
      Golden created 2026-08-20 (initial). It also passes bit-identically
      across opt-levels, which is a useful determinism check.
      Golden tolerance set to 3 LSB 2026-08-20: it passed locally but
      failed on CI with 21294/72000 samples off by 1-2 LSB (-92 dBFS,
      inaudible). Cause is libm, not the engine - tanh/sin/exp differ in
      the last bits between glibc versions, so a cross-platform
      sample-exact golden is not achievable. Same-machine
      bit-reproducibility is unaffected and still tested separately.
      Golden re-blessed 2026-08-20 for M4.1: mixer smoothing changed from
      a one-block ramp to a fixed 5ms ramp (see M4.1). 122 of 72000
      samples changed, worst 5 LSB, all inside the master-fader ramp at
      the start of the render - exactly the region the change affects.
      Test profile now builds at opt-level 2: the suite was spending most
      of its time in unoptimised DSP.

## M4 - Realtime adapter (macOS first)

- [x] M4.1 Command/EngineEvent enums + simulated audio-thread loop
      (verify: identical render across block sizes 37/64/480/1024,
      blocking commands rejected while rolling, transport commands
      clamp correctly). Two real findings, both fixed:
      1. The adapter MUST split callback buffers at command boundaries,
         or a command's effect lands at a different sample depending on
         the device period. Required for M4.2.
      2. Mixer smoothing ramped over one block, so a fader move sounded
         different at 64 frames than at 512. Now a fixed 5ms ramp.
      rtrb deferred to M4.2 where the real audio thread needs it, so
      default builds and CI stay dependency-free.
- [x] M4.2 cpal adapter behind `realtime` feature: device listing and
      substring selection, 48kHz negotiation, input/output joined by a
      wait-free ring, buffers split at command boundaries, blocking
      commands bounced to the control thread, `devices` and `live` CLI
      commands. Verified by clippy -D warnings with the feature on;
      CI unaffected (feature off by default).
- [x] M4.3 Xrun counters (output / input-starved / input-dropped) and a
      device+period report printed by `live`.
- [!] M4-hardware BLOCKED 2026-08-20: needs a hands-on session on the
      MacBook with the Zoom L6. Nothing further can be verified here -
      this host has no audio hardware. Checklist in
      docs/manual-checklist.md. Fill in the lowest reliable period and
      any findings, then M6 can reuse the same procedure on the Pi.

## M5 - Slint UI

- [ ] M5.1 UI skeleton behind `ui` feature: transport + tape counter wired
      to command queue only (verify: engine tests untouched, builds with
      --features ui)
- [ ] M5.2 Track strips (fader/pan/arm) + master + meters
- [ ] M5.3 Cassette new/load/save, undo button, export dialog, punch UX

## M6 - Raspberry Pi 4 deployment

- [ ] M6.1 aarch64 build, cpal-ALSA, config for L6 device name and period
      settings (verify: on-device smoke checklist)
- [ ] M6.2 Performance pass: 128-256 frame period, callback-time
      instrumentation (verify: measured headroom documented in repo)
- [ ] M6.3 systemd/kiosk launch, microSD save-timing check, Pi setup README
