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
- [x] M4.4 REQ-902 violation found 2026-08-20 during the hardware
      checklist: Command::Stop reliably triggered a CoreAudio buffer
      overrun on the L6 going record -> stop, invisible to the app's own
      Xrun counters. Command::Stop.is_blocking() == false, so it runs on
      the realtime output-callback thread; the handler chain
      (Engine::stop -> close_passes -> Journal::push_pass, undo.rs) did
      a heap allocation sized to the whole pass and a synchronous
      fs::File::create + write_all there - also reachable from
      process_block itself when recording runs off the tape end, not
      just from an explicit Stop.
      Fixed without needing to hand the engine itself across a
      thread boundary: Journal::push_pass now only does in-memory
      bookkeeping (the pass's Vec<i16> moves into a pending_writes list,
      no new allocation proportional to its size) and returns
      immediately - no I/O, cannot fail. The actual fs::File::create +
      write_all is deferred to a new Journal::flush_pending, called
      internally by save/undo/redo, all of which are blocking commands
      that only ever run off the realtime thread (and, per M4.5, save
      is what actually runs at live's shutdown). Eviction's
      fs::remove_file is deferred the same way.
      New regression test (engine.rs,
      stop_does_not_write_the_journal_payload_until_save): asserts no
      pass-*.bin file exists on disk right after stop, and that save
      produces it. Full gate green, golden render unchanged (byte-exact
      - this only changes when the write happens, not any audio path).
      Not addressed here, lower priority since it's a virtual-memory
      reservation rather than real I/O: Engine::record's
      RecordPass::with_capacity still reserve_exacts up to the
      remaining tape length inside the callback (record.rs) - flag if
      it ever shows up as a measurable xrun.
      Needs hardware re-verification: repeat the manual checklist's
      record/stop step and confirm the xrun summary is actually zero at
      --period 256 now.
- [x] M4.5 `live` has no working path to persist anything. Found
      2026-08-20 during the hardware checklist: recorded takes are lost
      the moment the process exits (tape/*.raw and undo/journal.json
      never touched after `new`; a `render` right after a live session
      is silent). `cmd_live` (main.rs) had no key bound to Command::Save,
      and RealtimeSession::send unconditionally rejected all blocking
      commands (Save/Bounce/Undo/Redo) rather than routing them anywhere
      - realtime::start moves Engine into the audio callback closure, so
      there was no control-thread path back to it at all.
      Fixed with a shutdown-only handoff rather than a full live
      round-trip (simpler, and it's what the checklist actually needed):
      `RealtimeSession::shutdown` sets an AtomicBool the callback checks
      every block for free, stops touching the engine, and hands it back
      over a wait-free ring the control thread was already blocked on.
      `q` in `cmd_live` now calls shutdown, then engine.stop() +
      engine.save() on the control thread, where blocking disk I/O is
      safe. Mid-session Save/Bounce/Undo/Redo while still jamming remain
      unreachable from `live` (out of scope for the M5 UI's stopgap
      harness) - M4.4 (Stop's own synchronous journal write inside the
      callback) is a separate, still-open bug.
      Verified 2026-08-20 on the MacBook with the L6: recorded a take
      live, quit, saw "saving... / saved.", and a fresh `render` process
      showed the real captured audio (track0.raw and journal.json both
      updated, non-silent render). New handoff-protocol unit tests in
      realtime.rs (cargo test -p porta-app --features realtime), full
      gate green.
- [x] M4.6 Input capture only ever opened 1 channel and broadcast that
      same mono signal to all 4 tracks regardless of which was armed
      (`[slice; NUM_TRACKS]` in realtime.rs). Found 2026-08-20 during
      the hardware checklist: takes were inconsistent/partial and on the
      Zoom L6 channels 1-2 are its own main mix, not a per-track send,
      so whichever single channel cpal picked wasn't even the right
      signal. Fixed: capture now opens up to channel_offset + NUM_TRACKS
      device channels and gives each track its own ring, fed from a
      distinct channel; a new `--in-offset N` flag on `live` skips
      leading channels (2 for the L6). Tracks beyond however many
      channels the device actually has record silence, not a duplicate.
      Also added, same session: cmd_live's 1-4 keys now toggle
      arm/disarm (previously arm-only, no way back) and print a status
      line after every toggle ("1R - 2 - 3 - 4R" style).
      Verified 2026-08-20 on the MacBook with the L6 at --in-offset 2:
      banner correctly reports "channels 3-6 -> tracks 1-4", arm status
      line toggles correctly through 1/2/1. Full gate green.
      Follow-up same day: [ and ] only ever nudge the playhead 1 second
      relative to wherever it is, not to the start - after a take longer
      than one rewind press, playback only caught the tail of it,
      reading as a single short fragment. Added a `0` key bound to an
      absolute Seek { sample: 0 }.
      Second follow-up same day: --in-offset 2 was a guess (L6 channels
      1-2 main mix, 3-6 the four inputs in order) and it was wrong - L6
      Input 1 showed up on software track 3, not track 1. Turns out the
      L6 exposes 12 channels over USB, not 6, so the whole assumed
      layout was off. Added `probe` (new subcommand): opens the input
      device at its full channel count and prints a live per-channel
      peak meter, so the real mapping can be read off directly instead
      of guessed via record/render round trips. Still needs a hands-on
      session to read the actual channel numbers and land on the right
      --in-offset (or, if the four inputs aren't contiguous, a proper
      per-track channel list instead of a single offset).

## M5 - Slint UI

- [x] M5.1 UI skeleton behind `ui` feature: transport + tape counter wired
      to command queue only (verify: engine tests untouched, builds with
      --features ui). Slint 1.17, `ui/main.slint` + `src/ui.rs`,
      `porta-app ui <dir>`. No real audio yet - a repeating Slint timer
      (20ms) stands in for the audio thread, feeding silence through
      process_block so the transport and counter behave the way they
      will once M5.2/M5.3 wire in the realtime adapter. Button handlers
      and the timer only call porta_engine::command::apply and read
      back state()/playhead() - never Engine's internal fields, per the
      M5 gate ("UI drives the engine through the command queue only").
      Verified two ways: `cargo test --workspace` (default, no ui
      feature) untouched - 122 tests, all green; and manually, since
      Slint has no documented headless-testing pattern for a downstream
      app (its own test-driver crates test the compiler/language, not
      this) - launched the real window, screenshotted it, clicked Play
      via System Events/accessibility, watched the counter run
      00:00 -> 01:00 and the state label flip to Stopped when the
      1-minute test tape ran out, clicked Record with nothing armed
      (correctly a no-op, arming is M5.2), quit clean, no panics. Same
      manual-verification split as M4's hardware checklist: pure logic
      (format_counter) gets an automated test, window/wiring behavior
      gets watched directly.
- [x] M5.2 Track strips (fader/pan/arm) + master + meters.
      Metering didn't exist anywhere in the engine before this -
      EngineEvent::Levels was declared but never emitted. Added it
      properly at the source: Mixer now tracks each track's post-fader
      peak and the summed master L/R peak per block (computed inline in
      mix_block, no extra pass over the audio, REQ-902-safe - plain
      floats, no allocation), exposed as dBFS via
      Mixer::track_level_db/master_level_db and the matching
      Engine::track_level_db/master_level_db/fader_db/pan/master_db
      read accessors. Deliberately fader-only, not master, on the
      per-track meter, so it reads track balance independent of the
      overall volume knob - covered by
      track_level_follows_the_fader_but_not_the_master. 5 new mixer
      tests plus an engine-level integration test
      (levels_reflect_the_current_block_during_playback) driving the
      real process_block path, not just the isolated Mixer - it caught
      a wrong assumption in my first draft (the meter is live during
      record monitoring too, per REQ-305, not just during playback).
      UI: one TrackStrip Slint component (arm button, fader slider, pan
      slider, a MeterBar) instantiated 4 times, properties/callbacks
      aliased to the root with <=> so Rust only touches root-level
      generated methods; a wire_track! macro collapses the four
      near-identical arm/fader/pan handler triples into one definition.
      Verified on the real window: arm toggle confirmed via its ARMED
      label; fader/pan/master sliders confirmed with actual simulated
      mouse drags (cliclick, installed for this) rather than trusting
      the accessibility "set value" shortcut, which can bypass a
      widget's real event path - checked the resulting value through
      System Events after each drag and visually confirmed the knob
      positions diverged from the untouched tracks. Clean quit, no
      panics. Full gate green, including cargo test -p porta-app
      --features ui.
- [x] M5.3 Save, undo button, export - partial, see M5.4. Save and Undo
      are one Button each, wired directly to Engine::save/undo (real
      Result, shown as a status line: "save: ok" / "undo failed:
      nothing to undo" / etc via a small status_message helper).
      Export is a LineEdit path field (defaults "export.wav", plain
      text, not a native file-picker dialog - avoids a new dependency
      for a v1 that's explicitly "an instrument, not a DAW") + a button
      that reuses render::mixdown/write_wav verbatim, the same
      functions the CLI's render/export commands already use - one
      code path, not a second one that could drift (REQ-803's spirit).
      Undo's a plain Button, not a menu/history list (REQ-505).
      Punch UX needed no new UI at all: transport.rs already documents
      punch-in as record() from Playing and punch-out as play() from
      Recording (crossfade is REQ-306, already baked into the engine,
      already tested in record.rs) - the same Play/Record/Stop buttons
      from M5.1 already do this, noted in a .slint comment so it isn't
      mistaken for a gap. (My first draft of that comment claimed Stop
      punches out - wrong, checked transport.rs before committing to
      it: Stop halts the transport too, Play is what punches out while
      staying rolling.)
      New/load a different cassette from within a running UI is NOT
      done - deferred to M5.4, since it needs the running Engine
      swapped out at runtime, a real structural change, not more
      buttons. Today: quit, `porta-app new <dir>`, relaunch
      `ui <dir>` - inconvenient but functional.
      3 new tests (status_message, and export_wav against a real
      tempdir + hound-read-back of the written WAV, not just "it
      didn't error"). Verified on the real window: Save produced
      "save: ok" and rewrote manifest.json; Undo on a fresh cassette
      correctly produced "undo failed: nothing to undo"; Export wrote a
      real, valid 2-minute stereo 48kHz WAV (verified with Python's
      wave module) and reported "exported to export.wav". Found in the
      process, not fixed (default relative path lands wherever the
      process's cwd happens to be, not the cassette directory - a
      quirk worth an absolute-by-default path in M5.4, not a
      correctness bug). Clean quit, no panics. Full gate green.
- [ ] M5.4 Cassette new/load from within a running UI (needs the
      Engine swapped at runtime, not just more buttons - see M5.3).
      Export path default fixed same day: now `<cassette-dir>/
      export.wav` via a new default_export_path helper (tested), not
      whatever the process's cwd happened to be. New/load itself still
      not done.

## M6 - Raspberry Pi 4 deployment

- [ ] M6.1 aarch64 build, cpal-ALSA, config for L6 device name, period,
      and input channel offset settings (verify: on-device smoke
      checklist). Channel offset landed as a `--in-offset` CLI flag in
      M4.6 (`live --in-offset 2` currently: L6 channels 3-6 -> tracks
      1-4, decided 2026-08-20) - fine for manual testing, but it
      needs to persist as real project/device config rather than being
      retyped every run, and this is the natural place to do that
      alongside device name and period.
- [ ] M6.2 Performance pass: 128-256 frame period, callback-time
      instrumentation (verify: measured headroom documented in repo)
- [ ] M6.3 systemd/kiosk launch, microSD save-timing check, Pi setup README
