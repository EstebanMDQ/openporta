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

- [ ] M1.1 Tape type: fixed-length i16 tracks, region read/write,
      dirty-chunk bitmap (verify: roundtrip, bounds, dirty tracking)
- [ ] M1.2 Transport state machine + sample-accurate playhead, seek/rew/ff
      (verify: transition and position tests)
- [ ] M1.3 Playback mixer: fader/pan/master with per-block smoothing,
      process_block API (verify: RMS math, equal-power pan at center, no
      clicks on fader jumps)
- [ ] M1.4 Record pass: punch crossfades, displaced-audio capture, REQ-306
      (verify: recorded region equality, no clicks across punch points,
      unarmed tracks byte-identical)
- [ ] M1.5 Undo/redo stack with disk spill and cap eviction (verify:
      byte-equal restore, redo symmetry, eviction)
- [ ] M1.6 Project persistence: manifest + chunked track files, save/load
      (verify: roundtrip equality, only-dirty-chunks-written)
- [ ] M1.7 Session-script runner + debug WAV dump (verify: scripted
      record-then-play renders expected signal, WAV header/length correct)

## M2 - Lo-fi DSP

- [ ] M2.1 Chain plumbing already present; add block-splitting so chains
      accept arbitrary lengths up to MAX_BLOCK (verify: identity + split
      equivalence)
- [ ] M2.2 Saturation (tanh drive + makeup) (verify: THD above/below drive
      threshold, no NaN under abusive input)
- [ ] M2.3 Bandwidth limiting: biquad LPF ~10kHz + HPF ~60Hz (verify:
      attenuation and passband assertions)
- [ ] M2.4 Hiss: seeded filtered noise (verify: noise-floor window, seed
      determinism)
- [ ] M2.5 Wow/flutter: LFO + drift on fractional delay, cubic
      interpolation (verify: pitch-probe deviation in cents band at
      configured rate, no clicks, latency reported)
- [ ] M2.6 Optional bitcrush/sample-rate reduction (verify: quantization
      step and alias energy when on, bit-transparent when off)
- [ ] M2.7 TapeCharacter::build_chain wired into record path with TPDF
      dither -> i16, seed in manifest (verify: recorded sine shows
      saturation + rolloff signatures, two renders bit-identical)
- [ ] M2.8 Generation-loss acceptance test REQ-403 (milestone gate)

## M3 - Bounce, export, CLI

- [ ] M3.1 Bounce op: post-fader mono sum of 1-3 recorded to track 4
      (verify: matches independently computed reference with fixed seed,
      undo restores track 4)
- [ ] M3.2 Mixdown renderer + WAV export, 16-bit default / 24-bit flag
      (verify: export RMS matches engine playback, header golden)
- [ ] M3.3 porta-app CLI: new/script/render/export subcommands (verify:
      integration tests drive the binary via scripts)
- [ ] M3.4 The one end-to-end golden render + UPDATE_GOLDEN bless flow

## M4 - Realtime adapter (macOS first)

- [ ] M4.1 Command/EngineEvent enums + rtrb SPSC queues + simulated
      audio-thread loop (verify: identical output offline vs simulated
      realtime across block sizes 64 vs 480, REQ-203)
- [ ] M4.2 cpal duplex adapter behind `realtime` feature, device selection,
      48kHz negotiation, stop-gated disk work (verify: #[ignore] smoke
      tests + manual checklist; CI unaffected)
- [ ] M4.3 Metering, xrun counter, latency report (verify: simulated meter
      math tests; manual checklist; notify owner for hands-on session)

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
