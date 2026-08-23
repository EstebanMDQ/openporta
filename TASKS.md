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
      SUPERSEDED by M7 (change 001, spec v1.1): this task's verify text
      describes the mono-sum-onto-track-4 bounce, which no longer
      exists. Bounce is now a real-time stereo pass onto the dedicated
      bounce bus - see M7.7/M7.10. Left checked and unedited above as
      the historical record of what shipped at the time.
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
      **Closed the flagged reserve_exact gap, 2026-08-22**, resurfaced by
      spec review of the stereo-bounce proposal (openspec/changes/001):
      Command::Record is also non-blocking, so RecordPass::with_capacity's
      reserve_exact (up to the whole remaining tape - ~172.8MB for one
      mono track at 30 minutes) ran on the realtime thread on every
      record engagement, not just the Stop path M4.4 already fixed.
      record.rs's module doc comment has the full design; the short
      version: displaced audio is now captured in fixed-size chunks
      (tape::CHUNK_SAMPLES, 5s, matching REQ-802's own save granularity)
      instead of one reserve_exact'd buffer. Journal hands each new pass
      a reserve of pre-allocated chunks (24/track, ~2 minutes of
      continuous recording) up front; a rollover mid-pass just pops the
      next one, no allocation. Deliberately not live-refilled during a
      session - Engine is exclusively realtime-thread-owned while
      connected (the same reason Save/Undo fully disconnect first), so
      there's no safe off-thread moment to hand more buffers over
      without a dedicated background thread and wait-free queues, which
      was considered and explicitly scoped out for now (asked directly;
      the smaller fix was preferred over that new subsystem). Instead
      the reserve replenishes at the existing off-thread touchpoint
      (Journal::flush_pending, run by Save/Undo/Redo) as passes are
      written to disk. A single pass longer than 2 minutes with nothing
      flushing in between falls back to an ordinary allocation for the
      overflow - rare in practice, counted via a new
      Engine::pass_buffer_fallbacks() rather than silently corrupting
      undo data or refusing to record. Also fixed in the same pass:
      RecordPass::finish()'s punch-out fade did an unnecessary
      `.to_vec()` copy of its own already-computed scratch buffer before
      writing it to tape - deleted, writes the slice directly.
      Journal's on-disk format is unchanged (chunks are written to the
      same one-file-per-pass layout, just via several small sequential
      writes instead of one big pre-concatenated buffer), so no
      migration concern for existing cassettes.
      New tests: a multi-chunk pass round-trips through undo byte-
      exactly (record.rs); a pass with enough spares to cover its length
      never falls back (record.rs). All pre-existing tests pass
      unchanged, including the golden render (byte-exact) and the full
      generation-loss suite - the refactor is behaviorally transparent.
      Full gate green across all four feature combinations.
      This was prerequisite work for the bounce proposal (which would
      have made the original violation worse - two channels instead of
      one) but stands on its own: it fixes a real, pre-existing bug in
      ordinary track recording, unrelated to whether bounce ever ships.
      **Follow-up fix, same day**: a fourth spec-review pass (still on
      the bounce proposal, but checking the shipped code directly rather
      than trusting the design doc) found the fix above didn't actually
      hold up in steady state. `Journal::push_pass` only ever returned
      the chunks a pass *used* back toward the reserve, never the ones
      it took but didn't write into - so a track's 24-chunk reserve
      shrank by its whole per-pass share on *every* record engagement
      regardless of length, and about 4 short takes with no intervening
      Save/Undo was enough to drain it to zero. Also caught: `take_spares`
      used `Vec::split_off`, which allocates a new container despite a
      doc comment claiming otherwise; `push_pass` computed each entry's
      filename via `format!`+`PathBuf::join`+`to_string` on the realtime
      thread; `RecordPass.chunks` was never pre-reserved. Fixed properly:
      the pool is now one dedicated reserve per track (`[Vec<Vec<i16>>;
      NUM_TRACKS]`), handed out and returned via `mem::take`/plain moves
      (genuinely zero-allocation, not just "small"); `push_pass` gives
      back whatever a pass didn't use *immediately*, not just what
      eventually flushes; `Entry.file` is gone entirely (the filename is
      always derived from `id` via the existing `path_for`, so there was
      nothing to compute or store) - serde-compatible with old journals,
      since removing a field is a no-op for deserialization, not a
      breaking one. New regression test
      (unused_spares_return_to_the_pool_so_short_takes_never_fall_back,
      engine.rs): ten short takes on one track, no Save/Undo between any
      of them, all assert zero fallbacks - would have failed against the
      first version of this fix within 2 takes. Full gate green, golden
      render still byte-exact.
      Not yet done, flagged by the same review as the real way to make
      this invariant load-bearing rather than inferred: a global-
      allocator-backed counting test around record()/process_block()/
      stop() that would catch ANY future realtime-thread allocation
      directly, not just regressions in this specific pool mechanism.
      Worth its own task if this keeps mattering.
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
      Per-track input selection, 2026-08-23 (openspec/changes/002,
      owner-requested, review-approved v3): the channel offset became a
      per-track map. `--in-map 3,4,5,6` replaces `--in-offset 2` (the
      old flag now errors explicitly rather than being silently
      ignored by the validation-free flag() parser - a caught silent-
      wrong-channel hazard); the UI's offset field became an "Input
      channels" comma-list field sharing the same parser; a parse
      error blocks the connect (never a fallback map - the startup
      auto-connect path made that a real hazard); `-` marks an
      unassigned track; the persistent status line reports the
      validated per-track list (`[3,-,5,6]`). Everything data-shaped
      (parse/format, routing plan, serde config + offset migration,
      status formatters, 14 tests) lives in the new UNGATED
      `input_map.rs` so it actually runs in the plain CI gate - the
      review caught that device_config's own tests never did
      (feature-gated module, default features empty); device_config.rs
      keeps only file I/O. Capture wiring is per-track on both ring
      sides (a positional-prefix Vec would misroute sparse maps).
      Offset-era audio.json entries migrate on load, keyed on the
      field being absent, not empty. Verified headlessly by the gate;
      the on-device L6 checks are a new manual-checklist item (map
      parity with the old offset, scrambled map, narrower-than-probe
      stream).
      Verified on the real Pi + L6 same day: `--in-map 3,4,5,6`
      connects with the correct per-track banner (`track1<-ch3 ...`)
      and zero xruns; the remembered offset-2 config migrated and
      produced identical wiring with no flag at all; a sparse
      scrambled map (`6,-,4,3`) reports
      `track1<-ch6 track2<-silent track3<-ch4 track4<-ch3`;
      `--in-offset` and `--in-map 0,1,2,3` both error and block, exit
      1. The on-hardware run also caught a REAL migration bug no
      review or test had: re-saving the config dropped the legacy
      offset of every entry NOT touched by that connect (the old field
      is never re-serialized, and untouched entries had no map written
      in its place) - fixed with an eager normalize() on load plus a
      regression test for the untouched-sibling round trip; the one
      damaged entry on the Pi was hand-restored to map form. Kiosk UI
      relaunched on the new binary: auto-connect succeeded through the
      new parse path using the migrated map (a parse failure would
      have left it disconnected by design), screenshot confirms
      connection and no layout regression. The Settings view's
      channel-list status line and field can't be click-verified (same
      no-input-injection limitation as every prior UI entry); their
      formatters and parse are covered by the 15 ungated tests. The
      narrower-than-probe `--in-map 1,2` check remains on the manual
      checklist for a session with real signal into jacks 1-2.
- [ ] M6.2 Performance pass: 128-256 frame period, callback-time
      instrumentation (verify: measured headroom documented in repo).
      Extended for the stereo bounce bus (REQ-905, openspec/changes/
      001): the headroom measurement MUST include a bounce pass running
      - two full per-channel character chains plus the shared
      StereoFlutter in the same callback as ordinary playback - so this
      part can only run after M7.7 lands. Stated fallback if it does
      not fit at 128-256: raise the frame period before arming the
      bus, as a deliberate per-bounce tradeoff (a period change tears
      down the stream, so it can never adjust mid-pass); document the
      chosen period alongside the measured numbers. [manual] on-device
      measurement; the numbers land in docs/, not a cargo test.
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
      Mute/monitor + real button colors + tape position bar + hold-to-
      scrub, 2026-08-22, all requested directly and landed together:
      - Engine: Command::Mute/Monitor, both non-blocking. Mute silences
        a track's contribution to the mix (output and meter) without
        touching its fader, folded into the same smoothed pan/fader
        target so it rides the existing 5ms click-avoidance ramp;
        persisted in the manifest like fader/pan (#[serde(default)] so
        an existing cassette's manifest.json still loads); bounce()
        fixed to exclude muted sources too - it summed tracks through
        its own gain calculation, bypassing the mixer entirely, so
        needed a separate fix. Monitor makes an armed track's live
        input audible while stopped or playing, not just recording -
        dry, not run through the character chain (that's reseeded
        fresh per record pass, reusing it for a stateless preview
        would mutate state a real pass doesn't expect); session-
        transient like arm, not persisted. 3 new tests. Golden render
        unaffected - both default off.
      - UI: TactileButton rebuilt as a plain Rectangle + TouchArea
        instead of inheriting std-widgets' Button, which never exposed
        enough to restyle (font-size wasn't even overridable, found
        earlier this session) - gives it a real `active`/`active-color`
        for a solid, meaningful highlight (red for armed/muted, blue
        for monitoring, green for Play while actually playing, red for
        Record while actually recording) instead of a generic theme
        accent. Track/master panels and every button also gained a
        visible border for definition - the general "more contrast"
        ask, not just the specific buttons named.
      - Tape position bar under the counter, driven by two new raw-
        sample float properties (playhead-samples/tape-len-samples -
        float so the fraction doesn't truncate under integer
        division); tape length set once at cassette-open time (launch/
        New/Load), not every tick.
      - Hold-to-scrub: holding rewind/fast-forward repeats the jump
        every 200ms via a native Slint `Timer` bound to the button's
        own pressed state - no new engine capability needed. If the
        transport happens to be playing while held, each 200ms window
        between jumps is genuine audible playback: a real, if choppy,
        audible scrub through the existing Rewind/FastForward
        commands. A quick tap still fires once via clicked, well under
        the 200ms window.
      Verified for real on the Pi: deployed the built binary, and
      since I can't click through a remote session, hand-edited a
      throwaway test cassette's manifest.json (muted[0]=true, playhead
      at 25% of tape length) to exercise the exact same persisted-mute
      and playhead-fraction paths a real session would - screenshotted
      the result: Track 1's mute button renders solid red ("MUTED"),
      the position bar shows correctly at 25%, and every panel/button
      border reads clearly against the light theme. Play/Record's
      active-coloring uses the identical TactileButton mechanism with
      a different (already-proven-correct-by-construction) boolean
      expression, not separately screenshotted. Full gate green across
      all four feature combinations.
      Export Video, 2026-08-23, requested directly ("make it create a
      video... ready to upload to youtube and other services"), scoped
      down to file generation only after two rounds of confirming with
      the requester that actual uploading (even via a script like
      youtubeuploader) stays out - spec.md section 2 rules out "Cloud
      anything" for v1: a still image plus the mix, muxed into an MP4
      via ffmpeg (`render::write_video`) - the standard "static image +
      audio" recipe most platforms, YouTube included, accept directly.
      Not a bundled encoder: shells out to `ffmpeg`, same reasoning MP3
      export already applies to shine_rs, with a clear error if it's
      missing rather than a silent failure. Reachable three ways
      sharing the one function: an "Export Video" field+button in the
      Tapes view (next to Export WAV/MP3, image path typed alongside
      the existing export path), `porta-app render/export --out
      *.mp4 --image <file>` on the CLI, and an `export_video` op in
      session scripts. ffmpeg installed on both the Mac (brew) and the
      Pi (apt) for this. 3 new Rust-side tests, one of them a real
      ffmpeg round trip (generates a fixture PNG via ffmpeg itself,
      then asserts the output starts with an MP4 ftyp box) - skips
      cleanly if ffmpeg isn't on PATH rather than failing the suite.
      Verified for real on the Pi: redeployed the binary, confirmed
      `porta-app render --out out.mp4 --image cover.png` produces a
      real MP4 (`file` reports "ISO Media, MP4 Base Media v1") using
      the Pi's own older ffmpeg 5.1.9 build, not just the Mac's;
      relaunched the kiosk UI on the new binary and screenshotted the
      mixer screen to confirm no regression. Did not click through to
      the Tapes view itself to exercise the new field/button - same
      no-input-injection-tool limitation noted for the Tapes view and
      Bounce button above; its logic is the same `with_engine`
      disconnect/run/reconnect shape as Export WAV/MP3, already proven
      live on this Pi, and the CLI path exercises the actual
      `render::write_video` function end to end on the real hardware.
      Full gate green across all four feature combinations.
      Tapes view + autosave + free-space indicator + no-scroll mixer
      screen, 2026-08-22, all four requested together
      ("saving progress... a view to manage tapes... free space...
      avoid the screen to scroll"):
      - New Tapes view (a "Tapes" button next to the cassette path
        opens it, same pattern as the Settings gear): lists sibling
        cassette directories next to the one currently open (anything
        with a manifest.json, via std::fs::read_dir - no new
        dependency), tap a name to load it; New/Load and both Export
        buttons moved here from the main mixer screen. Free-space text
        (fs4::available_space on the tapes directory, new optional
        dependency, ui-feature-gated only) shown at the top.
      - Autosave: the timer tick tracks whether a Record pass happened
        since the last save and saves automatically the instant the
        transport lands back on Stopped - REQ-802 explicitly allows
        this trigger. Never mid-recording, never redundantly. Pulled
        the decision out into a standalone autosave_decision(flag,
        state) function partway through (it started as inline timer-
        closure logic, which isn't headlessly testable - the project's
        own invariant) with 2 new tests covering fire-once-then-quiet
        and the no-recording case.
      - Moving New/Load/Export out wasn't enough by itself to fit the
        800x480 kiosk display - first deploy still showed a scrollbar
        with Save/Undo/status cut off below the fold. Second pass:
        shortened the vertical fader/meter travel 140px -> 90px,
        dropped the "pan" text label (the slider's own center position
        already says it), merged the Save/Undo row with the status
        line instead of stacking them, tightened padding/spacing.
      - 3 new Rust-side tests (sibling-cassette scanning against a real
        temp directory, free-space text format, root-path edge case).
      Verified for real on the Pi, two full CI-build-and-deploy passes
      (release.yml's workflow_dispatch leg, linux-aarch64 artifact):
      first pass's screenshot showed the predicted scrollbar/cutoff;
      second pass's screenshot (grim, real 800x480 kiosk output) shows
      the whole mixer screen - path/Tapes/Settings, counter, position
      bar, transport, all 4 tracks + master, Save/Undo/status - with no
      scrollbar and visible margin to spare. That session had also
      auto-connected to the L6 for real (status line: "connected: out
      L6 Analog Surround 4.0 / in L6 Multichannel"), a rerun of M6.1's
      auto-connect confirming it still works end to end. Did not verify
      the Tapes view's own click-through (load a sibling, hit New/
      Export) - no input-injection tool is installed on the Pi and
      installing one wasn't asked for; its Rust-side logic (the risky,
      novel part - filesystem scanning and free-space formatting) is
      covered by the 3 tests above against a real filesystem, and its
      .slint layout reuses the same ScrollView/TactileButton/list-row
      patterns already screenshot-verified for the Settings view.
      Worth a real click-through later if an input tool gets approved.
      Full gate green across all four feature combinations, both
      passes.
      Bounce wired into the UI, 2026-08-23, requested directly:
      `Command::Bounce`/`Engine::bounce()` have existed since M3 but
      were only ever reachable from a session script - added a "Bounce"
      TactileButton next to Save/Undo. Bounce is blocking and stop-
      gated like Save/Undo/Export, so it's wired through the same
      `with_engine` disconnect/run/reconnect path, not the live command
      queue - structurally identical to the already-verified Save/
      Undo/Export handlers, not a new pattern. No behavior change to
      bounce itself (still the mono sum of tracks 1-3 onto track 4,
      REQ-401), just a way to reach it. Deployed and screenshotted on
      the real Pi: button renders correctly, connected to the L6, no
      layout regression. Did not verify by actually clicking it - same
      no-input-injection-tool limitation as the Tapes view above;
      Engine::bounce() itself is already covered by
      crates/porta-engine/tests/bounce.rs, and the handler's own shape
      is identical to Save/Undo/Export's, already proven live on this
      Pi. Full gate green across all four feature combinations.

## M7 - Stereo bounce bus (openspec/changes/001, spec v1.1)

Implements the approved stereo-bounce proposal: REQ-401/402 rewritten,
REQ-404..409 new, REQ-603 deleted, REQ-502/602/702/801/904 amended
(spec commit b5134b3). Dependency order below is load-bearing - each
task assumes everything above it. The old Command::Bounce path stays
alive and tested until M7.10 so the full gate is green at every commit.
M6.2's headroom measurement gains a bounce clause that depends on M7.7
(noted there).

- [x] M7.1 porta-dsp: split Flutter into FlutterModulator (wow osc +
      flutter walk, emits a delay-in-samples value) + FlutterDelay
      (ring buffer + Catmull-Rom read); Flutter becomes a thin
      composition of one of each; add StereoFlutter (one modulator, two
      delays, process(&mut self, l, r)). Depth-clamp constants
      (.min(CENTRE - 4.0), .min(CENTRE / 4.0)) shared between Flutter
      and StereoFlutter construction, not duplicated. AudioProcessor
      stays mono/in-place. REQ-402, REQ-701/704 unchanged. (verify:
      every existing flutter and generation-loss test passes
      unmodified; StereoFlutter fed identical input on both channels
      produces identical outputs - shared modulation, directly;
      latency_samples unchanged)
      Done 2026-08-23. The clamp constants live only in
      FlutterModulator::new - both compositions construct through it,
      so they can't drift by construction. Two new tests: the
      identical-input one from the verify text, plus a stronger
      mono-equivalence check (a StereoFlutter channel is bit-identical
      to a mono Flutter with the same seed - guards both the refactor
      and the shared constants; the delay-value sequence is the same
      arithmetic in the same order). All existing flutter tests pass
      unmodified; golden render byte-identical (the split is bit-exact,
      no regen needed). Full gate green, all four feature combos.
- [x] M7.2 porta-dsp: split-chain builder on TapeCharacter - pre-
      flutter Chain [Saturation, Hiss, Bandwidth], shared
      StereoFlutter, post-flutter Chain ([Crush] if enabled, empty
      otherwise), with a HISS_STAGE constant kept beside the builder
      (the TapeCharacter::HISS_STAGE precedent) so builder and reseeder
      cannot drift. StereoFlutter::reseed as an inherent method that
      clears both rings + write indices and reseeds the modulator
      (exactly the state Flutter::reset clears, across three objects);
      the numbered per-pass sequence: reset both sub-chains,
      reseed_stage(HISS_STAGE) on the pre-flutter chain only,
      StereoFlutter::reseed. Modulator always seeds at channel term 0.
      REQ-402/702. (verify: a reused, reset+reseeded split setup
      renders identical output to freshly built ones with the same
      seeds - the reseed_chain_matches_a_freshly_built_one property,
      stereo; the empty post-flutter chain resets/processes safely)
      Done 2026-08-23. build_split_chain returns (pre, post) per
      channel; build_stereo_flutter supplies the shared middle;
      reseed_split_chain resets both halves then reseeds hiss at
      SPLIT_HISS_STAGE (its own constant beside the builder - equal to
      HISS_STAGE today by coincidence, deliberately not by reference);
      StereoFlutter::reseed clears both rings + writes + modulator,
      pinned to exactly Flutter::reset's state set. Two new tests: the
      stereo reused-equals-fresh property (dirties the setup with a
      different pass first, distinct per-channel seeds, distinct
      content per channel), and empty-post-half safety + crush landing
      there when enabled. Full gate green, golden byte-identical.
- [x] M7.3 porta-engine: Tape gains the stereo bounce bus - an
      explicit dedicated field (NOT an appended Tape.tracks element:
      0..NUM_TRACKS loops would silently skip it), fixed cassette
      length, 2 x i16, region read/write per channel, its own dirty-
      chunk bitmap. REQ-101/401. (verify: bus roundtrip, bounds, and
      dirty tracking; tracks 1-4 byte-identical across bus writes; no
      track-indexed API can address the bus)
      Done 2026-08-23. Refactored the per-channel storage mechanics
      (read/read_raw/write_raw/chunk/dirty) down into `Track` so the bus
      reuses them rather than duplicating; `Tape`'s track methods now
      delegate, behavior unchanged (golden byte-identical). The bus is a
      dedicated `bus: [Track; 2]` field addressed by a `BusChannel`
      enum, not a usize - so the "no track-indexed API can address the
      bus" property is compile-time, not a convention: track methods
      take a usize into `tracks`, bus methods take a BusChannel, neither
      can reach the other's storage. 5 new tests: per-channel roundtrip
      (channels don't share storage), truncate/zero-fill at the tape
      end, per-channel dirty tracking that leaves the other channel and
      all 4 tracks clean, both-directions audio isolation (REQ-306's
      symmetry clause), and short-tail chunk access. Full gate green.
- [x] M7.4 porta-engine: Mixer::mix_block split into sum_tracks (ticks
      each track's fader/pan ramps exactly once per sample; produces
      the monitor sum gated by a new excluded-from-sum-but-still-
      metered flag AND the ungated print sum from the same scaled
      values; meters from input * fader_amp exactly as today) and
      finish_mix (adds the bus playback pair at its own smoothed
      fader/mute - no pan - then a separate master Smoothed ramp, then
      the +/-1 clamp; only place out_l/out_r are written). REQ-406/408/
      409 groundwork; REQ-602/203 preserved. (verify: track-only output
      matches the pre-split mixer across block sizes 1/37/64/512;
      master-fader jump produces no click - extend
      fader_jump_does_not_click to master; an excluded track vanishes
      from the monitor sum, stays full-weight in the print sum, and its
      meter still reads; golden stays within its 3 LSB tolerance - if
      the fp reordering exceeds it, re-bless here with a note and
      notification, and M7.9's event then covers only the op change)
      Done 2026-08-23. target() no longer folds in the master; master
      and the bus each got their own Smoothed. sum_tracks ticks both
      per-track ramps unconditionally BEFORE any gating, then routes
      the same scaled value into the monitor sum (gated) and the print
      sum (never gated) - so exclusion can't freeze a ramp. finish_mix
      adds the bus at its own gain (no pan), ticks the master once per
      sample, clamps, and is the only writer of out_l/out_r; the bus
      gain ticks even when no bus buffers are passed, same
      no-frozen-ramp reason. mix_block is now a two-line wrapper, so
      engine.rs is untouched this task and every combo stays green.
      5 new tests (master click, the three-way exclusion split, ramp-
      not-frozen across un-exclude, block-size invariance at 1/37/64
      vs 512, bus fader/mute/no-pan); the 12 existing mixer tests pass
      unmodified. GOLDEN: the fp reordering does change the render but
      stays INSIDE the 3 LSB tolerance (verified by blessing to a temp
      copy and diffing, then restoring) - so no re-bless here, and
      M7.9's regeneration event still covers only the op change, as
      the task hoped. Full gate green, all four feature combos.
- [x] M7.5 porta-engine: journal stereo entry + bus reserve.
      Entry.right_track: Option<usize> #[serde(default)] (None = every
      existing single-channel entry; len stays per-channel);
      Entry::bytes() doubles when right_track.is_some(); one payload
      file per id, left channel's bytes then right's, back to back;
      Journal::undo/redo run the read/write sequence for both channels,
      succeeding or failing together. pending_writes entries gain an
      explicit Track(usize)-vs-Bus tag (no overloaded bare index), and
      the bus gets a double-buffered reserve - two full-tape-length
      buffers per channel, allocated once at open/create, handed out
      and reclaimed via mem::take, give-back routed by the tag.
      REQ-502/503/505, REQ-902. (verify: stereo-entry undo/redo
      restores both channels byte-equal with no reachable one-channel-
      reverted state; a pre-existing journal JSON still loads; eviction
      accounting counts a stereo entry at 2x; the reserve hands out
      buffer A then B with zero allocation and a third take falls back)
      Done 2026-08-23. Entry gained right_track (#[serde(default)]) plus
      a PassTarget::{Track,Bus} accessor - the virtual bus slots live
      ONLY in the serialized form and target() is what code matches on,
      so an index past NUM_TRACKS can never reach a track array (and
      they're deliberately NUM_TRACKS/+1, which would panic loudly
      rather than alias track 0 if anything ever tried). bytes() charges
      a stereo entry 2x. pending_writes became a tagged Pending enum
      (Track{chunks} vs Bus{left,right}) - the two really do have
      different storage shapes and different reserves, so give-back is
      routed by the tag, with a debug_assert that entry target and
      payload tag agree. undo/redo collapsed into one swap_with_tape
      ordered so everything fallible happens BEFORE both channel writes,
      which is what makes a stereo revert atomic against failure. The
      bus reserve is two full-tape pairs, allocated in Engine::assemble
      via with_bus_reserve (Journal::new can't know the tape length);
      buffers are given back WITHOUT clearing, unlike chunks - they're
      index-written and must keep their length, and clearing would force
      a ~170MB re-resize on the audio thread. 5 new tests: both-channel
      undo/redo with a track proven untouched, 2x byte accounting, a
      pre-change journal JSON still loading as single-channel, the
      reserve handing out A then B then reporting empty (and refilling
      on flush), and an evicted-while-pending bus entry returning its
      buffers to the bus reserve with every track's chunk reserve
      untouched. Full gate green, golden unaffected.
- [x] M7.6 porta-engine: bus arm/fader/mute state + non-blocking
      Command::BounceArm/BounceFader/BounceMute, REQ-405 mutual
      exclusion (arming the bus clears all 4 track arms and vice
      versa), and the bus summed into ordinary playback: tape readback
      into its playback slot when no pass is open, through finish_mix.
      REQ-404/405/409. (verify: mutual exclusion both directions; bus
      content audible at its fader during plain playback and mute
      silences it; fader/mute moves ride the 5ms ramp, no clicks)
- [x] M7.7 porta-engine: the realtime bounce pass in
      Engine::process_block. record() with the bus armed engages a
      stereo pass (REQ-301); print buffers, bus playback pair, and
      the per-block gain scratch buffer are Engine-owned, allocated at
      open/create; both split chains built unconditionally at open/
      create and reset+reseeded per pass via M7.2's sequence; print
      input = sum_tracks' print sum + the bus's prior content read
      before write (REQ-407), scaled by the bus gain ticked once per
      sample into the scratch buffer and reused post-chain (REQ-406
      pre-master tap); per-channel hiss/dither seeds via
      seed_for(noise_seed, pass, channel), L=0/R=1; post-chain W(t)
      dithered/quantized through the double-buffered reserve and copied
      to the bus playback slot while tracks 1-4 are excluded-but-
      metered (REQ-408); punch crossfades per REQ-302 unchanged.
      (verify: printed region equals tracks-at-live-fader/pan plus
      folded-forward prior bus content, byte-identical across two
      same-seed cassettes regardless of master position; tracks 1-4
      byte-identical across a bounce and the bus byte-identical across
      a track pass - REQ-306 both ways; one undo reverts both channels;
      two back-to-back bounces with nothing saved keep
      pass_buffer_fallbacks() == 0 and a third is allowed to fall back;
      renders bit-reproducible)
      Done 2026-08-23. New BouncePass in record.rs mirrors RecordPass
      (same REQ-302 crossfades, same displaced capture) with three
      differences: two channels in lockstep, one full-tape buffer per
      channel from the reserve instead of chunks, and REUSED IN PLACE -
      Engine owns one for its whole life and calls begin()/finish(), so
      engaging a bounce allocates nothing. Caught during implementation:
      finish() must NOT truncate the capture buffers to the pass length,
      or they go back to the reserve short and the next bounce can't use
      them - len travels alongside instead (push_bus_pass and
      Pending::Bus both carry it). process_block's bounce branch:
      sum_tracks with the print sum, tick_bus_gain once into the
      Engine-owned scratch, add the bus's prior content read before
      write (REQ-407), run pre/flutter/post per channel, write through
      the pass, copy W into the bus playback slot, then finish_mix with
      the ALREADY-ticked gain (new Option<&[f32]> param - ticking there
      too would double the ramp rate). Tracks excluded-but-metered for
      the pass duration. 6 acceptance tests, all green first run: stereo
      image printed and identical across two master positions on
      same-seed cassettes, REQ-306 both directions, fold-forward with
      tracks muted proving prior content is read, one undo/redo
      reverting both channels, two bounces with zero fallbacks and a
      third allowed one (plus a save refilling the reserve), and
      tracks-silent-but-metered with the bus muted. Full gate green,
      golden unaffected.
- [x] M7.8 porta-engine: bus persistence - tape/bounce_l.raw +
      bounce_r.raw written in the existing 5s dirty-chunk pattern;
      Project::open/load_tape treat missing bus files as never-
      bounced silence; Manifest gains bounce_fader_db: f32 +
      bounce_muted: bool, both #[serde(default)], carried by
      apply_to/capture_from. REQ-801/802, REQ-409. (verify: save/
      reopen roundtrips bus audio byte-exact and fader/mute values;
      a cassette saved before this feature opens with a silent bus at
      unity/unmuted; only dirty bus chunks are written)
      Done 2026-08-23. bounce_l/r.raw alongside the track files, same
      5s dirty-chunk write path; create() zero-fills them, load_tape
      treats a missing file as never-bounced silence, and save_tape
      CREATES a missing file on first write so an old cassette can be
      bounced and saved rather than erroring. Manifest gained
      bounce_fader_db/bounce_muted (#[serde(default)]) carried by
      apply_to/capture_from - worth persisting more than most mix
      state, since a muted bus changes what the NEXT bounce prints.
      4 new tests including a genuine pre-bus cassette (bus files
      deleted, old manifest JSON without the fields) opening clean at
      unity/unmuted and then saving successfully. Full gate green.
- [x] M7.9 porta-app script runner (+ fixtures): new ops Op::Mute
      {track, on}, Op::BounceArm {on}, Op::BounceFader {db},
      Op::BounceMute {on}, and Op::Bounce {seconds} (errors unless the
      bus is armed; runs the transport like Op::Play); update the
      three {"op":"bounce"} users - tests/golden.rs, tests/cli.rs,
      auditions/m3-session.json - to the new shape; regenerate the
      golden render via UPDATE_GOLDEN. This is the proposal's single
      regeneration event: record the re-bless note here in TASKS.md in
      the same commit and notify the owner. REQ-804. (verify: a script
      drives arm/fader/mute/bounce headlessly end to end; golden
      passes against the regenerated reference; cli suite green)
      Done 2026-08-23. Five new ops (mute, bounce_arm, bounce_fader,
      bounce_mute, bounce{seconds}); Op::Bounce errors unless the bus
      is armed and rolls the transport like Op::Play, capturing the
      monitor output - which during a bounce is the printed signal with
      tracks excluded (REQ-408), exactly what an export across a bounce
      should contain.
      *** GOLDEN RE-BLESSED - the proposal's single regeneration event.
      Understood before blessing, not blessed to make a red test green:
      the session's bounce semantics genuinely changed (mono sum onto
      track 4 -> stereo bus), so track3.raw is now EMPTY where it used
      to hold the sum, and the bus carries the print. Verified by
      running the golden session standalone and inspecting the raw tape:
      bounce_l/r.raw non-zero, track3.raw all zeros, tracks 0-2 intact.
      Also took the opportunity - same event, no extra cost - to widen
      the bounce window from 0.5s to 1.5s: at 0.5s it only covered the
      centered bass (the script records sequentially, so the panned
      chord/lead sit later on the tape) and the print was mono, L-vs-R
      differing by only 1.9 rms of per-channel dither/hiss. At 1.5s the
      print carries a real stereo image (908 rms L/R difference across
      the panned chord), so the golden actually exercises the stereo
      bus instead of a mono-only window. New reference: no clipping,
      comparable level (rmsL 4970 vs 4861), wider image (L-R 912 vs
      737). ***
- [x] M7.10 porta-engine + porta-app: delete the old bounce -
      Command::Bounce and its is_blocking() arm, Engine::bounce(), the
      disk_touching_commands_are_marked_blocking assertion about it;
      rewrite crates/porta-engine/tests/bounce.rs wholesale to the new
      semantics (every listed test:
      bounce_sums_the_source_tracks_onto_track_four,
      bounce_respects_faders_and_ignores_pans (REQ-603 is deleted -
      pans are now honored), bounce_excludes_muted_tracks,
      bounce_is_undoable, bounce_applies_the_character_again,
      bounce_is_refused_while_rolling, bounce_is_reproducible,
      bounce_leaves_the_source_tracks_alone); swap ui.rs's
      on_bounce_pressed to bus-arm + Record so --features ui still
      builds (minimal - the full UI surface is M7.14); append a
      superseded-by-M7 note to M3.1's entry, whose verify text
      describes the deleted semantics. (verify: rewritten bounce.rs
      suite green - stereo sum honors faders and pans, sources
      untouched, fold-forward, undo byte-exact, character compounds,
      refused while a track pass is open (REQ-405), reproducible; no
      reference to the deleted paths remains; gate green across all
      four feature combinations)
      Done 2026-08-23. Command::Bounce, its is_blocking arm,
      Engine::bounce() and the now-unused db_to_amp helper are gone;
      the blocking-command test asserts over the remaining three.
      bounce.rs rewritten wholesale - 10 tests, all new semantics, not
      ports: prints onto the bus, honours pans (the inverse of the
      deleted REQ-603 test), respects faders, excludes muted tracks,
      leaves ALL FOUR sources untouched (the old bounce consumed track
      4), atomic undo/redo, folds the previous generation forward,
      character compounds, REQ-405 exclusion, reproducible with the two
      channels genuinely differing, and dominant-frequency identity.
      ui.rs's Bounce button now sends BounceArm+Record through the
      command queue instead of a blocking call - minimal, so --features
      ui keeps building; the bus fader/mute strip is M7.15. M3.1 got a
      superseded-by-M7 note appended rather than being edited. Full
      gate green, no references to the deleted paths remain.
- [x] M7.11 porta-engine: generation_loss.rs REQ-403 rewrite, in
      place - prime (bounce once with tracks 1-4 unmuted), then mute
      all four and bounce three more times, measuring generations
      2/3/4 (bus re-printing only its own prior content, identical
      input conditions per generation). (verify: monotonic 8kHz-band
      decay and monotonic noise-floor rise across gens 2-4,
      reproducible, tolerating a few hundred samples of the accepted
      per-generation latency drift - no exact-alignment assertion)
      Done 2026-08-23. REQ-403 now measures REAL bounce generations
      instead of track-to-track passes standing in for them: prime the
      bus from a track, mute all four, then bounce three more times so
      each generation re-prints only the bus's own prior content -
      identical input conditions per generation, which is what makes
      the three measurements comparable. Monotonic 8kHz decay and
      monotonic noise-floor rise across gens 2-4, both with audible
      magnitude floors. Windows are generous rather than sample-aligned
      (each generation adds ~480 samples of flutter delay - accepted
      drift, deliberately not asserted against). Added a
      bounce-path reproducibility test alongside. Full gate green.
- [x] M7.12 porta-engine acceptance tests (+ a Pearson correlation
      helper in porta-testkit beside band_energy_db, not inline in a
      test): stereo image - hard-left source, bounce twice, right
      channel's band energy in the source range stays >= 10dB below
      the left's; hiss decorrelation - Pearson between the two
      channels' hiss-only regions < 0.1 over a multi-second window
      (REQ-702); master invariance - two fresh same-seed cassettes,
      identical op sequences differing only in Op::Master before the
      bounce, printed regions byte-identical (REQ-406); clamp
      engagement - 5 generations of 0dBFS material produce a sustained
      flat-top plateau of consecutive extreme samples (not a lone
      boundary sample), then pin a regression bound from the measured
      clip fraction, not a guessed one. (verify: the four named
      numeric assertions above)
- [x] M7.13 porta-engine REQ-408 monitoring tests, two separate tests:
      (a) dither-bound - prime the bus with a real first bounce, mute
      tracks 1-4, set a settled -6dB bus fader via BounceFader, run a
      measured pass lying entirely inside the primed region and ending
      short of the tape end; capture the live monitor output over the
      middle stretch clear of both XFADE_SAMPLES windows, replay the
      same tape region after the pass closes, and assert RMS of the
      per-sample difference < ~0.25 LSB (the dither bound at -6dB) -
      an RMS assertion, not per-sample tolerance; (b) metering -
      tracks 1-4 unmuted and carrying signal, bus muted, pass open:
      each track_level_db reads above the meter floor while the
      block's audible output is exactly silent. (verify: the two
      assertions as stated)
      Done 2026-08-23. tests/bounce_monitoring.rs, two tests with
      deliberately opposite setups (folding them together is what a
      review caught as unpassable: the dither test mutes the tracks,
      and the mixer meters a muted track as silent by design).
      The dither bound came out as a genuine confirmation of the
      spec's derivation: predicted 0.5 LSB RMS scaled by -6dB =
      0.2505 LSB, MEASURED 0.251 - three significant figures. Made the
      assertion two-sided rather than just loosening it: an upper bound
      at 1.25x catches regressions, and a lower bound at 0.5x catches
      the vacuous case where the two captures aren't actually
      independent. The metering test mutes the BUS instead, so the
      audible output is exactly zero while both tracks read well above
      the floor. Full gate green.
- [ ] M7.14 porta-engine: allocator-counting harness - a test-only
      counting global allocator asserting zero alloc/dealloc on the
      simulated realtime path across record(), process_block(), and
      stop(), for both an ordinary track pass and an open bounce pass.
      Makes REQ-902 load-bearing instead of inferred; independent of
      the bus but must cover it. (verify: counts are zero across
      those calls in both scenarios, and the harness fails when an
      allocation is deliberately injected)
- [ ] M7.15 porta-app UI: Bounce button becomes bus-arm + Record
      through the live command queue (no blocking call), with REQ-405's
      engine-side auto-clear reflected in the UI - either echo arm
      changes back via an EngineEvent that LiveState.armed mirrors, or
      document accepting a stale display until the next resync (a
      decision to make here, flagged in the proposal); add a bus
      fader/mute strip (fader + mute, no pan, REQ-409). Must still fit
      the 800x480 kiosk layout without scrolling. (verify: new pure
      logic under cargo test --features ui; gate green across all four
      feature combinations; [manual] Pi checklist: layout fits 800x480
      with no scrollbar, Bounce click-through arms and prints the
      bus, bus fader/mute audibly apply during playback and while
      bouncing)
