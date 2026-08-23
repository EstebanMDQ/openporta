# 002: Per-track input channel selection

## Motivation

Requested directly by the owner: "we need to be able to select the
input for each channel" - each of the 4 tracks should record from a
user-chosen input channel of the audio device, not from a fixed
contiguous block.

Today the capture wiring is a single **offset**: track `t` reads device
channel `offset + t`, always contiguous (`frame[channel_offset + t]` in
`realtime.rs`'s input callback). That was a deliberate, documented
decision - `device_config.rs`'s module doc says so explicitly:
"Deliberately just an offset, not a full per-track channel assignment -
every device this has actually been run against wants a contiguous
block starting somewhere... no confirmed use case yet. If a real
interface ever needs non-contiguous channels, this is an additive
change to one `DeviceSettings` entry, not a rewrite." This proposal is
that confirmed use case arriving, and it takes the extension path that
comment predicted. The M6.1-era decision "L6 channels 3-6 -> tracks
1-4" (TASKS.md) stays *expressible* - `3,4,5,6` - it just stops being
the only expressible shape.

This is also not a new control the instrument never had: the reference
hardware's channel strips carry an input select switch (a Porta 424
channel chooses what feeds it). Per-track input selection restores a
control the real machine has, rather than adding one it lacked - no
drift against section 1's fixed small control set.

**Owner decisions already made (asked directly, 2026-08-23):** the map
*replaces* the offset rather than layering an override mechanism on top
of it (one mental model; the old offset becomes nothing more than the
default fill `offset+t`), and it is settable from **both** the UI
Settings view and the CLI, like every existing device setting.

## Change

### The mapping

- Each track `t` (0..4) has an **input channel assignment**: which
  device channel feeds it while recording, or **unassigned** (records
  silence). Stored per *device*, not per cassette, in the same
  `~/.config/openporta/audio.json` the offset lives in today, for the
  same reason (wiring is a property of the physical setup, not of any
  one project).
- **User-facing channel numbers are 1-based**, matching what
  `porta-app probe` already prints (`ch1: -12dB  ch2: ...`) - the probe
  command is the tool users identify channels with, so the numbers they
  type must be the numbers it shows. Internally 0-based, converted at
  the parse boundary, in exactly one place.
- **Duplicate assignments are allowed, deliberately**: two tracks may
  map the same device channel (record one source onto two tracks at
  once - real portastudios allow the same routing, and forbidding it
  would be extra validation for no benefit). One consequence stated so
  no test asserts it wrongly later: the two tracks' *tape content* will
  NOT be byte-identical - each record pass runs its own character chain
  with its own seed (REQ-304), so the same source lands with different
  wow/flutter/hiss on each track. Same input, two takes' worth of tape
  character - exactly what the real machine would do.
- **Out-of-range assignments degrade per-track, not globally**: a track
  whose mapped channel doesn't exist on the device records silence and
  is reported as inactive (see "Status reporting" below). The other
  tracks are unaffected.
- The map is **fixed for the lifetime of a stream**, captured by value
  into the input callback at connect time, same as the offset today.
  Changing it means reconnect. No mutation reaches the audio callback
  (REQ-902 untouched).

### Syntax, pinned (a first review found four edge cases undecided)

One comma-separated list, shared verbatim between the CLI flag and the
UI field, parsed by one shared function:

- `3,4,5,6` - tracks 1-4 from channels 3-6.
- `-` marks an interior unassigned track: `3,-,5,6` leaves track 2
  silent while tracks 1/3/4 record. (Trailing truncation alone can't
  express this, and per-track degradation makes interior holes
  meaningful - so the syntax needs a token for them.)
- Fewer than four entries: remaining tracks are unassigned. More than
  four: parse error.
- `0` is a parse error, not "unassigned" - the list is 1-based and a
  zero is almost certainly a mental off-by-one; failing loudly beats
  silently recording the wrong channel.
- Channels are bounded at the parse boundary: `1..=1024` accepted,
  anything larger is a parse error. (Without the bound, the connect
  path's cast through `u16` would silently truncate - e.g. `65537`
  becoming channel 1, a wrong-channel recording with no error.)
- An **empty list is a parse error** in both surfaces. This is what
  keeps migration honest (see below): "absent field" and "deliberately
  empty" must not collapse into the same value. "Record nothing" is
  already expressible by giving no `--in` device at all.
- **Zero effectively-assigned tracks** (all `-`, or every entry beyond
  the device's channels): no input stream opens at all - same as
  having no input device - and the status surfaces say so.

### CLI

- `--in-map C1,C2,C3,C4` replaces `--in-offset N` on `live`: e.g.
  `--in-map 3,4,5,6` for the L6.
- `--in-offset` becomes an **explicit error** ("--in-offset was
  replaced by --in-map C1,C2,C3,C4"), not a removed-and-ignored token.
  A first review caught why this matters: `flag()` never validates
  unknown arguments, so plain removal would leave `--in-offset 2`
  silently ignored and the session recording from whatever map was
  remembered - a silent wrong-channel take, precisely the failure class
  the device-config work exists to prevent. Three lines of code, not
  ceremony.
- The connect banner's contiguous wording (`channels N-M -> tracks
  1-K ... the rest record silence`) is rewritten as a per-track list
  (e.g. `inputs: track1<-ch3 track2<-ch4 track3<-ch5 track4<-ch6`, with
  `silent` for unassigned/out-of-range tracks).
- USAGE documents `--in-map` with the same L6 worked example the offset
  text has today. `docs/manual-checklist.md`'s copy-pasteable
  `--in-offset 2` procedure (line 29 and its explanation) is updated to
  `--in-map 3,4,5,6` **in the same change** - it's the operational
  procedure for exactly the manual test this proposal defers to.

### UI (Settings view)

- The "Input channel offset" LineEdit is replaced by a single "Input
  channels" LineEdit taking the same comma list the CLI takes
  (placeholder: `e.g. 3,4,5,6`), sharing the one parse function - not
  four separate per-track fields, which would cost Settings-view
  vertical space the 800x480 kiosk layout doesn't have (the no-scroll
  constraint is hard-won; see TASKS.md's M5.5 entries).
- Pre-filled from the remembered per-device setting the same way the
  offset field is today; a successful connect remembers what it used,
  silently, same as every other device setting (no separate save
  action).

### Status reporting (a first review found "reported as inactive" had
no surface that could express it)

- The UI's `connection_status` line gains a compact per-track channel
  list: `connected: out <name> / in <name> [3,-,5,6]`, with `-` for a
  track that is unassigned or whose channel the device doesn't provide.
  Today that line carries no channel information at all, so this is the
  one UI addition beyond the Settings field.
- The CLI banner change above is the same information in long form.
- `RealtimeSession`'s `input_channel_offset: usize` field becomes the
  resolved per-track map (`[Option<usize>; NUM_TRACKS]`), and
  `input_tracks` becomes a *count of assigned tracks*, not a prefix
  length. Known readers, listed as blast radius so none is missed:
  `ui.rs`'s `with_engine` reconnect tuple, both `remember()` call
  sites (`ui.rs`, `main.rs`), the CLI banner, and the doc comments in
  `realtime.rs` that describe the offset model.

### Persistence and migration

- `DeviceSettings.input_channel_offset: usize` is replaced by
  `input_channels: Option<Vec<Option<usize>>>` in the serialized form
  (0-based internally; `None` entries = unassigned). Serialized as the
  new field only; the old field stays deserializable.
- **Old `audio.json` files migrate on load**: a `DeviceSettings` whose
  `input_channels` field is **absent** fills the map from the offset as
  `[offset, offset+1, offset+2, offset+3]` - byte-for-byte the same
  wiring the offset produced. The rule is keyed on *absent*, not
  *empty* (a first review caught that `#[serde(default)]` to a bare
  `Vec` would make an old file and a deliberately-emptied new file
  indistinguishable) - which is also why the parse boundary rejects
  empty lists outright: no in-memory state ever needs the empty case.
  Old-to-new is lossless; no user re-types anything after upgrading.
- The `device_config.rs` module doc comment's "deliberately just an
  offset" paragraph is rewritten to record that the confirmed use case
  arrived and this proposal took the predicted additive path.

### Capture wiring (`realtime.rs`) - both sides of the ring

A first review found the first version of this section described only
the producer side; the contiguous-prefix assumption lives on both:

- **Input callback (producer)**: `frame[channel_offset + t]` becomes a
  per-track read of each ring's paired, connect-time-validated channel
  index. Tracks with no valid assignment get no ring at all, preserving
  today's "record silence rather than a duplicate of another track's
  input" behavior.
- **Ring consumers**: `track_capture_rx` is currently a positional
  `Vec` whose index *is* the track index - with a sparse map that
  misroutes audio (track 2's ring would land on track 1). It becomes
  track-indexed (`[Option<Consumer<f32>>; NUM_TRACKS]` or a paired
  vec), the drain loop reads per-track, and the tail zero-fill
  (`captured[track_capture_rx.len()..]`) becomes per-track fill of
  exactly the unassigned tracks.
- The stream's requested channel count becomes
  `max(assigned channels) + 1`, clamped to the device's maximum -
  replacing today's `channel_offset + NUM_TRACKS`. Stated as a neutral
  change, not a win: nothing validates the requested count against
  `supported_input_configs()` today either, and an odd count (e.g.
  `--in-map 1,3` asks for 3 channels) may be rejected by raw `hw:`
  routes that accepted the old even formula - "the requested stream
  configuration is not supported" is this project's top real-hardware
  failure (TASKS.md), so the manual verification below exercises a
  narrower-than-probe count on purpose.
- **A load-bearing assumption, stated rather than silently relied on**:
  probe opens the device at its *maximum* channel count while `live`
  opens `max(assigned)+1` - so "ch3 in probe = index 2 live" holds only
  if a narrower stream still delivers device channels `0..N` in order.
  This is already load-bearing today (offset mode opens narrower than
  probe too, confirmed working on the L6 at 6-of-12 channels), but this
  proposal elevates probe-parity to a requirement, so it's named, and
  the manual check exercises it directly.
- The routing decision (which device channel, if any, feeds each track,
  given a map and a device channel count; how many channels to request)
  is extracted into a **pure function**. Necessary but not sufficient -
  the consumer-side restructuring above is the other half; the pure
  function is what makes the *decision* testable headlessly.

### Where the shared code lives (a first review found the verification
plan as previously written would never run in CI)

`mod device_config` and `mod realtime` are both gated behind
`#[cfg(feature = "realtime")]`, porta-app's default features are empty,
and the CI gate runs plain `cargo test --workspace` - so tests placed
in those modules never run in CI (true of `device_config`'s existing
tests today, a pre-existing hole this proposal must not deepen). The
shared parse/format pair, the migration rule, and the routing pure
function all go in a new **ungated** module,
`crates/porta-app/src/input_map.rs` (no feature cfg, no cpal types -
plain data in, plain data out), with all their tests. `realtime.rs` and
`ui.rs` call into it; only the thin cpal wiring stays feature-gated.

### What doesn't change

- The engine and DSP crates: nothing. Input mapping is adapter-level;
  the engine's `process_block(&inputs, ...)` contract is untouched
  (REQ-901 is the reason this proposal is as small as it is).
- The probe command: unchanged, and becomes the documented first step
  of the workflow (probe, note the channels, type them into the map).
- Session scripts, offline rendering, golden render: untouched - none
  of them go near audio hardware.
- Per-device persistence model, the remembered-device-name fallback,
  the period setting: all unchanged.

## Requirements affected

Split per house style (proposal 001 uses a block; one id per claim).
REQ-907/908/909 verified free - spec.md tops out at REQ-906 and
proposal 001 claims REQ-404..409 only. Placement: REQ-907 is a
functional recording requirement and belongs with section 4's recording
requirements; REQ-908/909 concern the adapter/config surface and sit
with section 5's platform requirements.

- **REQ-907 (new, functional)**: While recording with a realtime input
  device, each track MUST record from its user-assigned input channel.
  A track with no assignment, or assigned a channel the device does not
  provide, MUST record silence without affecting other tracks. Two
  tracks MAY be assigned the same channel.
- **REQ-908 (new, adapter)**: Input channel assignments MUST be
  settable per track from both the UI and the CLI, using 1-based
  channel numbers matching the probe command's display, and each
  surface MUST report per-track assignment status (including inactive
  tracks) at connect time.
- **REQ-909 (new, adapter)**: Assignments MUST persist per input
  device; configurations saved by prior versions (channel-offset form)
  MUST load with identical routing and without user intervention.
- **REQ-901, REQ-902**: untouched, cited as constraints - the engine
  never sees the map, and the map never changes on a live callback.
- No existing spec.md requirement is reversed. The reversed decisions
  are code-level (`device_config.rs`'s documented offset-only choice)
  and a TASKS.md configuration note (M6.1's L6 offset), both updated by
  this proposal.

## Impact on tasks

- Folds into **M6.1** (still `[ ]`; its body is the offset's own
  history and this replaces the offset) rather than a new task - the
  task's verify text is rewritten around the map.
- `docs/manual-checklist.md`: the `--in-offset 2` procedure (line 29
  and its explanation) becomes `--in-map 3,4,5,6`; one new manual item
  added to the M6 section (see Verification).
- `TASKS.md`'s M6.1 note ("L6 channel offset decided (3-6 -> tracks
  1-4)") gains a line recording the map superseding the offset.
- New module `crates/porta-app/src/input_map.rs` (ungated);
  `DeviceSettings` field swap + migration; `realtime.rs` capture
  wiring both sides; `main.rs` flag + banner + USAGE; `main.slint` +
  `ui.rs` Settings field + status line.

## Verification (headless, REQ-906)

All in the ungated `input_map.rs` so they actually run in the CI gate:

- Parse tests: `"3,4,5,6"` -> `[Some(2),Some(3),Some(4),Some(5)]`;
  `"3,-,5,6"` interior hole; short lists; `"0"` errors; `"1025"`
  errors; empty errors; >4 entries errors; junk errors; round-trip
  through the shared parse/format pair used by both CLI and UI.
- Migration tests: old-format `audio.json` with
  `input_channel_offset: 2` loads as `[2,3,4,5]`; new-format file
  round-trips; absent-vs-present distinction preserved.
- Routing-function tests: full map on a big device; sparse map (track
  2 unassigned, track 3 assigned - the misrouting case); out-of-range
  entries (that track silent, others live); duplicate entries (both
  tracks fed); requested channel count = max(assigned)+1 clamped to
  device max; zero assigned -> no stream.
- [manual, added to docs/manual-checklist.md M6] On the Pi with the
  L6: `--in-map 3,4,5,6` reproduces exactly the current
  `--in-offset 2` behavior; a scrambled map (`6,5,4,3`) routes jacks
  to tracks in reverse; and `--in-map 1,2` (narrower than probe's 12)
  confirms the channel-order assumption on a count neither the old
  formula nor probe ever opened, with the probe command and a real
  signal as the reference.

## Alternatives considered and rejected

- **Keep the offset and add per-track overrides**: rejected by the
  owner directly - two interacting mechanisms to explain, test, and
  display in a Settings view with no room for them.
- **Four separate UI fields**: rejected for kiosk layout cost; the
  comma list shares its parser with the CLI and costs one line.
- **A deprecation alias for `--in-offset`**: still rejected, but the
  flag errors explicitly instead of vanishing - see CLI section for
  why silent removal is the worst of the three options.
- **Per-cassette mapping**: rejected - wiring is physical-setup state,
  the same reasoning that put the offset in `audio.json` and not the
  manifest.

## History

**v1**: initial proposal, following the owner's two scoping decisions
(map replaces offset; UI + CLI surface). A first review returned
REVISE: the verification plan's tests would never have run in CI
(`device_config`/`realtime` are feature-gated out of the plain
`cargo test --workspace` gate - a pre-existing hole for
`device_config`'s own tests, but this proposal promised the coverage);
the capture-wiring section described only the producer side while the
contiguous-prefix assumption also lives in the consumer-side
`track_capture_rx` positional Vec, its drain loop, and the tail
zero-fill (a sparse map would have misrouted audio); "reported as
inactive" had no surface that could express it (the UI status line
carries no channel information); and removing `--in-offset` outright
would have left it *silently ignored* by the validation-free `flag()`
parser - a silent wrong-channel recording - besides breaking
`docs/manual-checklist.md`'s operational procedure. Also: REQ-907
packed six claims into one id; the migration's "lossless" claim
conflated absent with empty; four syntax edge cases were undecided
(zero, upper bound, interior holes, zero-assigned); and no Impact on
tasks section existed.

**v2 (this revision)**: all of the above addressed - ungated
`input_map.rs` module for everything testable; both sides of the ring
restructured (track-indexed consumers); status reporting specified for
UI and CLI; `--in-offset` errors explicitly and the manual checklist
updates in the same change; REQ-907 split into 907/908/909 with
placement stated; migration keyed on absent-not-empty with empty lists
rejected at parse; all four edge cases pinned (`0` errors, `1..=1024`
bound, `-` for interior holes, zero-assigned opens no stream); Impact
on tasks added (folds into M6.1). Reviewer notes folded in: the
stream-config change stated as neutral with the odd-channel-count
hazard named; the probe-parity channel-order assumption stated and
exercised by a new narrower-than-probe manual check; duplicates
documented as not-byte-identical on tape (REQ-304 seeding); and the
reference-hardware identity argument (a Porta 424 channel has an input
select switch) added to Motivation. Ready for a second spec-reviewer
pass.
