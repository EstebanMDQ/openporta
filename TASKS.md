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
      Follow-up 2026-08-21, requested directly: MP3 alongside WAV -
      WAV stays the master (lossless, tunable --bits), MP3 is a
      convenience format to share, fixed at 192kbps, no new flag -
      `render`/`export --out` pick the format from the extension, and
      the UI got a second "Export MP3" button that swaps the existing
      export path's extension rather than needing its own field.
      shine-rs (pure Rust MP3 encoder, LGPL-2.0) over the LAME-binding
      alternative specifically to avoid an autotools-in-CI repeat of
      the pipewire saga - it needs nothing beyond a Rust compiler,
      release.yml untouched. Every real Rust MP3 encoder is LGPL
      (LAME and Shine both are); no permissively-licensed alternative
      exists in the ecosystem today. Verified for real, not just unit
      tests: rendered both formats from an actual cassette, confirmed
      the .mp3 with `file`/macOS's afinfo (valid MPEG Layer III,
      correct duration/bitrate/channels - genuinely decodable), and
      redeployed to the Pi to confirm the same on real aarch64
      hardware (`file` there reports identical: MPEG ADTS, layer III,
      v1, 192kbps, 48kHz, JntStereo). UI button visually confirmed via
      a real screenshot on the Pi; the click itself wasn't remotely
      testable (no input injection there), but it calls the exact same
      render::write_mp3 already verified through the CLI.
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
- [x] M5.4 Cassette new/load from within a running UI. Turned out
      smaller than M5.3's note suggested: Engine already lived behind
      `Rc<RefCell<Engine>>`, shared with every handler and the timer,
      so a swap is just `*engine.borrow_mut() = new_engine` - nothing
      else needs rebuilding or rewiring. Added a path LineEdit + New/
      Load buttons above the transport. New uses fixed defaults (15
      min, cassette character, seed 0 - the CLI's --minutes/--seed/
      --character flags have no UI equivalent, out of scope here);
      Load is a plain `Engine::open`. Both refresh the export-path
      default and status line on success, report the error on failure.
      The `<dir>` CLI arg stays required at startup (no "no cassette
      loaded" empty state) - deliberately, to avoid threading
      `Option<Engine>` through every function for a v1 UI.
      Export path default fixed same day: now `<cassette-dir>/
      export.wav` via a new default_export_path helper (tested), not
      whatever the process's cwd happened to be.
      2 new tests: create_default_cassette against a real tempdir, and
      a direct test of the swap mechanism itself (replacing
      *engine.borrow_mut() is visible through every other Rc::clone of
      the same RefCell). Verified on the real window: played cassette
      A to 00:01, typed cassette B's path, clicked Load - counter and
      state correctly reset to B's own fresh 00:00/Stopped rather than
      carrying over A's Playing state, status read "loaded
      .../m54-b.porta". New produced a real cassette on disk and
      "created .../m54-new-cassette.porta". Clean quit, no panics.
      Full gate green.
- [x] M5.5 Wire the realtime adapter into the Slint UI, plus a
      settings panel for device/channel/period selection.
      `EngineEvent::Levels` now carries real per-track + master dBFS
      (`[f32; NUM_TRACKS]` + `(f32, f32)`, was a stale unused `{left,
      right}` shape) and is actually emitted from the output callback
      in realtime.rs, mirroring the Playhead push right next to it.
      `ui.rs` no longer owns a single `Engine` unconditionally: a new
      `Backend` enum is `Silent(Box<Engine>)` (today's timer-driven
      skeleton, unchanged behavior) or, with `--features realtime`,
      `Live(Box<LiveState>)` wrapping a real `RealtimeSession` on a
      cpal thread - the UI polls/sends through it exactly like
      `cmd_live` does from the terminal, and a `Snapshot` struct
      decouples `refresh()` from which variant produced the values.
      The hard constraint driving the design: REQ-902 blocking
      commands (Save/Undo/Export/New/Load) can't reach a running
      RealtimeSession, so they can't just call through to it. Solved
      with disconnect-run-reconnect - `with_engine()` takes the
      backend out from behind a shared `Rc<RefCell<Option<Backend>>>`,
      shuts the session down cleanly if it was live (via the existing
      M4.5 handoff), runs the blocking op on a bare `Engine`, and
      reconnects with the same device/period/offset settings
      afterward, falling back to `Silent` if reconnecting fails so the
      cassette isn't stranded unreachable.
      Also fixed along the way: `realtime::start()` used to drop the
      `Engine` on most early-return error paths (found while designing
      the UI's Connect flow, not a pre-existing bug report) - split
      into `negotiate()` (pure device/config negotiation, never touches
      the engine) and `start()` (only consumes it after negotiation
      succeeds), with a `StartError::Negotiation(Box<Engine>, ..)` /
      `StartError::StreamBuild(..)` split so a failed Connect attempt
      in the UI gets its cassette back instead of losing it; only the
      later `build_output_stream`/`stream.play()` failures are
      unrecoverable, since the engine is already moved into the cpal
      closure by then.
      New settings row in main.slint: input/output device text fields
      (blank = system default, matching the CLI), period and channel-
      offset fields, and a Connect/Disconnect toggle with a status
      line - all read only when Connect is pressed, matching --in/
      --out/--period/--in-offset on `live`, not applied live to an
      already-open stream.
      4 new ui.rs tests (non_empty blank-means-default,
      silent-backend snapshot reflects engine state, with_engine
      leaves the backend usable afterward, take_engine unwraps Silent
      directly) plus the pre-existing realtime handoff tests, still
      green. Full gate green for all four feature combinations
      (default, `ui`, `realtime`, `realtime,ui` - fmt, clippy -D
      warnings, and cargo test each). `Engine` and the new `LiveState`
      both had to be boxed inside their enums (large_enum_variant) -
      clippy catches this reliably, no manual size auditing needed.
      Live GUI click-through (Connect against a real device, watch
      meters move from a real EngineEvent::Levels, Save/Undo/Export
      through an actual disconnect-reconnect) still needs a hands-on
      pass with the screen unlocked - not done yet, tracked
      separately, not blocking the commit since the gate is the
      project's actual definition of done.
      Follow-up fix 2026-08-21: reported against the real Pi kiosk
      display (800x480 native, checked via wlr-randr on-device - even
      smaller than the 800x600 first reported) - the window used a
      fixed `width`/`height` (Slint pins that to a non-resizable
      window, maximize is then a no-op) sized well under the content's
      actual ~600px layout height, so most of the bottom of the UI was
      permanently off-screen with no way to reach it. Switched to
      `preferred-width`/`preferred-height` + `min-width`/`min-height`
      so the window manager can resize and maximize it, wrapped the
      content in a `ScrollView` so anything still too tall to fit
      stays reachable instead of silently clipped, and tuned
      preferred-height to 440px to roughly fit the actual kiosk panel
      under its window-manager chrome. Full gate green (all four
      feature combinations); visual confirmation on the real screen
      (Mac screen locked, Pi needs the new binary redeployed) is the
      next step, not done as of this commit.
      Second follow-up fix, same day: 440px still wasn't enough - real
      content was ~600px tall, so even a full-height window scrolled
      for most of the track strips. Compacted the layout for real
      (smaller MeterBar, tighter padding/spacing throughout, counter +
      transport-state sharing one row) to ~410px, so it fits the real
      screen without scrolling at all; ScrollView stays as a fallback,
      not the primary fix. Also added the seek-to-start/end and
      rewind/fast-forward-by-1s buttons that had been missing from the
      UI entirely (cmd_live has always had them via [ ] and 0 -
      Transport::seek already clamps to tape length, so "go to end" is
      just `Seek { sample: usize::MAX }`, no tape-length tracking
      needed in the UI), and a `--kiosk` flag (off by default) that
      sets Slint's own full-screen/no-frame Window properties rather
      than reconfiguring the Pi's desktop - keeps kiosk behavior
      versioned with the binary instead of a manual setting that
      wouldn't survive a reflash.
      Verified for real this time, not just gate-green: built via CI,
      deployed to the Pi, launched in its actual graphical session,
      and screenshotted with grim (`WAYLAND_DISPLAY`/`XDG_RUNTIME_DIR`
      exported over ssh - a plain ssh session has neither). Windowed:
      the whole UI - cassette row, audio settings, transport with the
      new seek/rewind/ff buttons, all 4 tracks, master, save/undo/
      export - fits inside the window with nothing cut off. `--kiosk`:
      genuine fullscreen, no titlebar, no wf-panel-pi taskbar visible,
      the app owns the whole 800x480 screen.
      Third follow-up, 2026-08-21, direct feedback plus a real bug
      chase: reported "not recording, signal looks like it's getting
      to the app." Root cause, found by re-running the same L6 test
      that worked earlier in the day: the L6 had been unplugged since,
      and every device-name lookup silently fell back to cpal's
      default_input/output pseudo-device (a generic 2-channel stand-in,
      not the real hardware) instead of erroring when an explicitly
      named device wasn't found - exactly the kind of bug that reads
      as "looks connected but isn't." Fixed in realtime.rs
      (pick_named_or_default: `Some(name)` with no match is now
      RealtimeError::DeviceNotFound, surfaced through the same error
      paths --in/--out already used); verified locally that a bogus
      name now errors clearly and blank still falls back to default.
      Same session, also direct feedback, a genuine UI overhaul:
      buttons too small for a touchscreen, low contrast, no light/dark
      adaptation, faders too small and not positioned like a real
      mixing console, and (separately raised) wanting an actual device
      list instead of free-text fields. Landed together since the
      device-list ask is also what closes the door on the silent-
      wrong-device bug above:
      - New Settings view (gear button on the main screen, Back
        returns) with real cpal device-name dropdowns
        (realtime::list_device_names, a names-only sibling of the CLI
        `devices` command) instead of free-text fields - refreshes on
        open so a device plugged in after launch shows up without a
        restart. Period/offset/Connect/Disconnect moved here too.
      - TactileButton (min-height/min-width 46px) on every button that
        does something, replacing std-widgets' mouse-sized default.
      - Track/master panel backgrounds and titles switched from
        hardcoded colors to Slint's `Palette` (alternate-background/
        foreground), which already tracks the OS light/dark setting -
        the custom elements just weren't using it before.
      - Each channel's fader is now a vertical Slider beside its meter
        (a real channel-strip layout) instead of a small horizontal
        slider stacked underneath, range changed from -60..6 to
        -36..12 so unity sits at exactly 3/4 up the travel
        ((0-(-36))/(12-(-36)) = 0.75) - cut below, up to +12dB of real
        gain above. Same range on the master fader.
      One known cosmetic gap, not fixed: writing the device text
      property from Rust (the Settings view's remembered-device
      prefill) doesn't move the ComboBox's internal highlighted
      selection, a documented Slint limitation (current-value writes
      don't update current-index - slint-ui/slint#11970). The label
      text itself is bound directly to current-value and displays
      correctly either way; only the popup's highlight can lag until
      a real click. Not chased further this pass.
      Verified for real on the Pi, both windowed and `--kiosk`
      (fullscreen): tactile buttons visibly bigger, light theme
      (matching the Pi's current OS setting) reads with good contrast,
      vertical faders sit beside their meters with the thumb
      positioned close to the intended 75% mark, and in kiosk mode the
      entire layout - transport, all 4 tracks, master, save/undo/
      export - fits the real 800x480 screen without scrolling.

## Release process

Requested 2026-08-21 in place of front-loading M6: a way to ship
prebuilt binaries before the Pi hands-on work, not instead of it.

- [x] R1 Multi-OS release pipeline. `.github/workflows/release.yml`,
      triggered on `v*` tags (or workflow_dispatch for a dry run that
      builds without publishing). Builds porta-app with
      `--features realtime,ui` for macOS (Apple Silicon native, Intel
      cross-linked from the same Apple Silicon runner - no native
      Intel runner label exists on GitHub-hosted runners anymore),
      Linux x86_64, Linux aarch64 (a genuine ARM runner,
      ubuntu-24.04-arm, not cross-compiled - doubles as a real
      per-commit build signal for M6.1's aarch64 requirement, short of
      the actual hardware and an on-device run), and Windows. Every
      build job depends on the same fmt/clippy/test gate ci.yml runs -
      nothing publishes without passing it. Packages each binary with
      README.md and LICENSE into a tar.gz (zip on Windows), uploads as
      a workflow artifact, and on an actual tag push attaches all of
      them to a GitHub Release via softprops/action-gh-release.
      Along the way: added the MIT LICENSE file the workspace's
      Cargo.toml had been claiming (`license = "MIT"`) without actually
      shipping since M0; updated README's status table, which still
      said "UI not started" and M4 "needs a hardware session" - both
      long since done.
      To cut a release: bump `workspace.package.version` in Cargo.toml,
      commit, `git tag vX.Y.Z && git push origin vX.Y.Z`.
      Verified 2026-08-21 with a real workflow_dispatch dry run, not
      just linting (actionlint on the YAML first, then the actual run -
      cross-platform CI breaks at runtime, not parse time): gate green,
      all 5 platform builds succeeded, publish correctly skipped (not a
      tag push). Downloaded and inspected every artifact - correct
      tar.gz/zip contents (binary + README + LICENSE), and the macOS
      arm64 one is a real, correctly-architected Mach-O binary that
      actually ran `--help` on this machine and listed both the
      `realtime` and `ui` command sections.
      Follow-up fix same day: the linux-aarch64 artifact from the first
      dry run failed on the real Pi with `GLIBC_2.39 not found` -
      ubuntu-24.04-arm's own userland (glibc 2.39) is newer than real
      Raspberry Pi OS/Patchbox (Debian 12 bookworm, glibc 2.36), and a
      glibc binary won't run against an older glibc than it was linked
      against. Rebuilt that one matrix entry inside `container:
      debian:bookworm` on the same native ARM runner instead of relying
      on the runner's own userland. Needed two follow-up fixes to get
      the container itself working: no git preinstalled (bootstrap step
      installs it before actions/checkout, which needs git to run at
      all) and no Rust toolchain (installed via rustup.rs manually,
      `$HOME/.cargo/bin` appended to `$GITHUB_PATH`); then a second
      failure, `dash` (the container's default `sh`) rejecting `set -o
      pipefail`, fixed with job-level `defaults: run: shell: bash`.
      Verified via two more real workflow_dispatch dry runs, not just
      actionlint - the second succeeded across all 5 platforms
      including the container-built linux-aarch64.

## M6 - Raspberry Pi 4 deployment

- [ ] M6.1 aarch64 build, cpal-ALSA, config for L6 device name, period,
      and input channel offset settings (verify: on-device smoke
      checklist). Channel offset landed as a `--in-offset` CLI flag in
      M4.6 (`live --in-offset 2` currently: L6 channels 3-6 -> tracks
      1-4, decided 2026-08-20) - fine for manual testing, but it
      needs to persist as real project/device config rather than being
      retyped every run, and this is the natural place to do that
      alongside device name and period.
      On-device progress 2026-08-21, real hardware (Patchbox OS,
      Debian 12 bookworm, glibc 2.36, PipeWire): deployed R1's fixed
      linux-aarch64 build to `~/openporta/bin/porta-app` over ssh and
      ran it there - `--help` and `devices` both actually execute now
      (the earlier glibc mismatch is confirmed gone on the real
      target, not just in CI). `devices` itself turned up a real bug:
      the Pi's two vc4-hdmi ALSA cards fail to open with no monitor
      attached (a completely normal headless setup), and
      `list_devices()` used `?` on `supported_output_configs()`, so
      the first dead HDMI output took the entire listing down with it
      - `porta-app devices` failed outright, headphones and any USB
      interface included. Fixed: each device gets its own `Result` now,
      an unusable one is reported inline as `[unavailable: ...]`
      instead of aborting the command. Verified both ways: locally
      against real macOS devices (still lists cleanly, several
      Pro Tools Bridge devices included) and redeployed to the Pi
      (exit 0, full listing, HDMI entries correctly marked
      unavailable, headphones and every ALSA plugin device listed).
      The Zoom L6 is not currently plugged into this Pi - dmesg shows
      it was connected earlier and cleanly recognized by
      `snd-usb-audio` (`ZOOM Corporation L6`) before being unplugged,
      which is a good sign for the USB-audio side, but the actual
      device-name/period/offset smoke checklist against a live L6
      still needs it physically reconnected - not something doable
      over ssh.
      On-device smoke test 2026-08-21, L6 now physically connected:
      real full duplex against the actual interface, on the actual
      target hardware, for the first time. `lsusb`/`arecord -l`/
      `pw-cli` all recognize it correctly (card 3, "L6 Multichannel"
      input, "L6 Analog Surround 4.0" output, PipeWire auto-promoted
      it to the system default sink/source the moment it was plugged
      in). `probe --in "L6"` opened a real 12-channel capture stream
      cleanly.
      One real snag: naming the device explicitly (`--in "L6" --out
      "L6"`) failed with "the requested stream configuration is not
      supported by the device". `devices` explains why - ALSA
      enumerates the same physical L6 many times over (hw:, plughw:,
      dmix, front, surround40, iec958, ...), and cpal's `Display` for
      all of them is just the card's description, so `porta-app
      devices` lists roughly a dozen entries that all print as `L6,
      USB Audio` with no way to tell them apart by name - some of
      those routes are raw `hw:` devices that reject a config the
      broader `plughw:`/PipeWire-native ones would happily convert.
      `pick()` matches by name and has no way to know which duplicate
      it grabbed. Blank device fields (`live --in-offset 2`, no
      `--in`/`--out`) sidestep this entirely by asking cpal for its
      *default* device instead of naming one - and since PipeWire had
      already made the L6 that default, this Just Worked: armed track
      1, recorded 2s and separately 8s, stopped, saved, all clean.
      `xruns: output 0, starved 0, dropped ~7-8k` both runs - the drop
      count didn't scale with duration (rules out a sustained clock-
      drift problem; reads as a fixed startup transient before the
      input ring reaches steady state, ~150-170ms worth of samples).
      Exported the recording and inspected the raw PCM (peak 18, rms
      5.4 of int16 full scale) - real, non-zero samples made it the
      entire way through capture -> per-track ring -> tape -> mixdown,
      just quiet (self-noise/room ambient, nothing was intentionally
      played into it for this pass).
      Net: the pipeline itself is solid end to end on real hardware.
      The name-collision problem is a real usability gap worth solving
      before M6.1's persisted device config leans on typed names the
      same way - either preferring PipeWire's own node identity over
      raw ALSA card names on Linux, or having `pick()`/`negotiate()`
      skip a duplicate that fails to open and try the next match
      rather than surfacing the first failure. Not fixed yet - flagging
      it for M6.1 proper rather than patching around it now.
      Root-cause fix, same day: enabled cpal's `pipewire` feature
      (target-gated to Linux inside cpal itself - confirmed a no-op on
      macOS/Windows via a full release-matrix dry run, all 5 platforms
      green). `default_host()` already prefers PipeWire over raw ALSA
      when the feature's compiled in, so this needed no code changes
      outside Cargo.toml - just `libpipewire-0.3-dev` added to
      release.yml's Linux deps (Patchbox itself runs PipeWire 1.2.7,
      well past cpal's >=0.3.53 floor), plus `libclang-dev` for
      libspa-sys's bindgen step, which the bare debian:bookworm
      container doesn't ship by default (ubuntu-latest already had it -
      only the container leg needed the extra package).
      Verified for real on the Pi: `devices` went from ~20 duplicate
      `L6, USB Audio` entries to 3 clean, distinct, sensibly-named ones
      (`L6 Analog Surround 4.0` output, `L6 Multichannel` input, plus
      the output's own monitor). `live --in "L6 Multichannel" --out
      "L6 Analog Surround 4.0" --in-offset 2` - the exact thing that
      failed before - now connects, arms, records, and saves cleanly,
      three separate runs, real non-zero samples confirmed via export
      each time. Input startup transient shrank too (starved 1024
      samples, ~21ms, vs ~7-8k dropped/~150ms on the old default-device
      workaround) and is now a fixed, repeatable number across runs,
      not just "doesn't grow with duration." One cosmetic leftover, not
      a bug: `shutdown()` reliably prints "audio input/output error:
      Device disconnected" once as PipeWire tears the stream down,
      immediately followed by a normal "saved." - harmless, just noisy;
      worth quieting later, not blocking.
- [ ] M6.2 Performance pass: 128-256 frame period, callback-time
      instrumentation (verify: measured headroom documented in repo)
- [ ] M6.3 systemd/kiosk launch, microSD save-timing check, Pi setup README
      Kiosk auto-launch done 2026-08-21, requested directly:
      `deploy/openporta-kiosk.desktop`, a standard XDG autostart entry
      (not a systemd unit, not an edit to the Pi's own
      /etc/xdg/labwc/autostart) - the desktop session already processes
      `~/.config/autostart/*.desktop` via lxsession-xdg-autostart, so
      this is purely additive and per-user: no system file touched,
      survives an OS update, no sudo, trivially reversible. Installed
      on the real Pi at ~/.config/autostart/openporta-kiosk.desktop.
      `--kiosk` removes all window chrome (no titlebar, no close
      button), so added an escape hatch: Escape now toggles kiosk-mode
      off (a FocusScope wraps the window content and grabs focus on
      init so the first keypress works without a click first);
      documented ssh + `pkill -f "porta-app ui"` as the always-works
      fallback regardless of what has keyboard focus. Both written up
      in docs/pi-setup.md.
      Verified for real: ran the exact Exec= command from the
      installed .desktop file by hand on the Pi (not just read the
      file) and screenshotted it - genuine fullscreen, no
      titlebar/taskbar, matching what boot will actually run.
      Reboot confirmed for real 2026-08-21, without needing to trigger
      it as a background task: came back to the Pi mid-session
      (`uptime` showed it booted 33 minutes earlier) and found
      `porta-app ui ... --kiosk` already running - autostart fired on
      its own after a real reboot, exactly as designed. The Escape
      keypress itself still hasn't been confirmed by an actual
      keystroke (no way to inject one into the Pi's session remotely
      without installing a new tool there, not done without asking
      first) - the property/handler wiring compiles clean and is
      correct by construction, but wants a real press to be sure.
      Full gate green across all four feature combinations.
      Taskbar launcher + desktop icon, 2026-08-21, requested directly:
      `deploy/openporta-launcher.desktop` (installed per-user at
      ~/.local/share/applications/, same additive/no-sudo approach as
      the autostart entry) pinned to the panel via one added
      `launcher_NNNNNN=` line in ~/.config/wf-panel-pi.ini, plus
      `deploy/openporta-desktop-icon.desktop` (a Type=Link entry in
      ~/Desktop/, matching how this system's other app icons are
      done). Launches windowed, not kiosk - a manual launcher wants to
      still reach the rest of the desktop. Also drew a real cassette
      icon (`deploy/openporta.svg` - two reels, tape path, label,
      screws) rather than reusing a generic stock icon; needed
      explicit width/height attributes alongside the viewBox, or it
      rendered at an unclamped huge size on this system's icon loader.
      Verified for real: both the taskbar's small rendering and the
      full-size desktop icon show the cassette artwork correctly,
      confirmed via grim's own geometry capture after sips's crop
      flags turned out to be unreliable for this on macOS.
      Auto-connect + in-app kiosk toggle, same day, requested directly:
      the UI now tries connecting to whatever device last connected
      successfully right at startup (reusing the exact same connect()
      path a manual button press already used, right after the
      Settings fields are prefilled), instead of always opening Silent
      and waiting for a click - the appliance is meant to be ready
      when it's turned on. Silent no-op if nothing's ever been
      remembered; falls back to Silent with a status message the same
      way a failed manual Connect would if the device's gone away
      since. Also added a kiosk-mode toggle button in Settings
      ("Enter/Exit kiosk mode") - previously the only way in was
      --kiosk at launch and the only way out was Escape; now it's a
      real two-way switch, no new property needed since kiosk-mode was
      already in-out for Escape's sake.
      Verified for real on the Pi with the L6 actually connected: no
      manual Connect click, and `pw-cli`/`pw-link` afterward showed
      two real `porta-app` PipeWire clients with 6 input ports and 4
      output ports - exactly what offset-2 + 4 tracks against the L6's
      4-channel output should open, not some fallback default. Didn't
      chase a visual screenshot of the Settings view this pass (the
      PipeWire client/port evidence is more conclusive than a screen
      grab would be anyway); the kiosk-toggle button itself is UI-only
      logic identical in shape to the already-verified Escape handler,
      not separately screenshotted.
