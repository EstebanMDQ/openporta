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

**Owner decisions already made (asked directly, 2026-08-23):** the map
*replaces* the offset rather than layering an override mechanism on top
of it (one mental model; the old offset becomes nothing more than the
default fill `offset+t`), and it is settable from **both** the UI
Settings view and the CLI, like every existing device setting.

## Change

### The mapping

- Each track `t` (0..4) has an **input channel assignment**: which
  device channel feeds it while recording. Stored per *device*, not per
  cassette, in the same `~/.config/openporta/audio.json` the offset
  lives in today, for the same reason (wiring is a property of the
  physical setup, not of any one project).
- **User-facing channel numbers are 1-based**, matching what
  `porta-app probe` already prints (`ch1: -12dB  ch2: ...`) - the
  probe command is the tool users identify channels with, so the
  numbers they type must be the numbers it shows. Internally 0-based,
  converted at the parse boundary, in exactly one place.
- **Duplicate assignments are allowed, deliberately**: two tracks may
  map the same device channel (record one source onto two tracks at
  once - real portastudios allow the same routing, and forbidding it
  would be extra validation for no benefit). Stated so it isn't treated
  as an accident later.
- **Out-of-range assignments degrade per-track, not globally**: a track
  whose mapped channel doesn't exist on the device records silence and
  is reported as inactive - the same behavior tracks beyond the device's
  channel count get today, just decided track-by-track instead of as a
  contiguous-prefix truncation. The other tracks are unaffected.
- The map is **fixed for the lifetime of a stream**, captured by value
  into the input callback at connect time, same as the offset today.
  Changing it means reconnect. No mutation reaches the audio callback
  (REQ-902 untouched).

### CLI

- `--in-map C1,C2,C3,C4` replaces `--in-offset N` on `live` (and the
  UI binary's equivalent path): four comma-separated 1-based channel
  numbers, one per track, e.g. `--in-map 3,4,5,6` for the L6.
- Fewer than four entries: remaining tracks are unassigned (silence).
  More than four: error.
- `--in-offset` is **removed, not kept as an alias**. It appears in
  USAGE and nothing else scripts against it (session scripts drive the
  engine, which never sees device wiring - REQ-901); a deprecation
  period for a flag on an interactive command on a one-user appliance
  is ceremony. The USAGE text documents `--in-map` with the same
  L6-worked-example the offset text has today.

### UI (Settings view)

- The "Input channel offset" LineEdit is replaced by a single "Input
  channels" LineEdit taking the same comma list the CLI takes
  (placeholder: `e.g. 3,4,5,6`), sharing one parse function with the
  CLI flag - not four separate per-track fields, which would cost
  Settings-view vertical space the 800x480 kiosk layout doesn't have
  (the no-scroll constraint is hard-won; see TASKS.md's M5.5 entries).
- Pre-filled from the remembered per-device setting the same way the
  offset field is today; a successful connect remembers what it used,
  silently, same as every other device setting (no separate save
  action).

### Persistence and migration

- `DeviceSettings.input_channel_offset: usize` is replaced by
  `input_channels: Vec<usize>` (0-based internally, length <=
  NUM_TRACKS; missing entries = unassigned).
- **Old `audio.json` files migrate on load, losslessly**: a
  `DeviceSettings` deserialized with an `input_channel_offset` but no
  `input_channels` fills the map as `[offset, offset+1, offset+2,
  offset+3]` - byte-for-byte the same wiring the offset produced. Both
  fields stay deserializable (`#[serde(default)]`); only the new one is
  serialized. No user re-types anything after upgrading.
- The `device_config.rs` module doc comment's "deliberately just an
  offset" paragraph is rewritten to record that the confirmed use case
  arrived and this proposal took the predicted additive path.

### Capture wiring (`realtime.rs`)

- The input callback's routing changes from `frame[channel_offset + t]`
  to `frame[map[t]]`, guarded per-track: each ring's sender is paired
  with its (validated-at-connect-time) channel index, and tracks with
  no valid assignment get no ring at all - same as today's
  beyond-device-channels case, so the "record silence rather than a
  duplicate of another track's input" behavior is preserved.
- The stream's requested channel count becomes
  `max(assigned channels) + 1`, clamped to the device's maximum -
  replacing today's `channel_offset + NUM_TRACKS`. A map like `1,2`
  on a 2-channel interface asks for 2 channels, not 6.
- The routing decision (which device channel, if any, feeds each
  track, given a map and a device channel count) is extracted into a
  **pure function** with headless tests - today it's inline arithmetic
  in the connect path, which is exactly why the contiguous-prefix
  assumption never had a test to violate.

### What doesn't change

- The engine and DSP crates: nothing. Input mapping is adapter-level;
  the engine's `process_block(&inputs, ...)` contract is untouched
  (REQ-901 is the reason this proposal is as small as it is).
- The probe command: unchanged, and becomes more useful (it's now the
  first step of the documented workflow: probe, note the channels,
  type them into the map).
- Session scripts, offline rendering, golden render: untouched - none
  of them go near audio hardware.
- Per-device persistence model, the remembered-device-name fallback,
  the period setting: all unchanged.

## Requirements affected

- **REQ-907 (new)**: While recording with a realtime input device, each
  track MUST record from its user-assigned input channel. Assignments
  MUST be settable per track (UI and CLI), persisted per input device,
  and MUST use 1-based channel numbers in every user-facing surface,
  matching the probe command's display. A track assigned a channel the
  device does not provide MUST record silence and be reported as
  inactive, without affecting other tracks. Two tracks MAY be assigned
  the same channel.
- **REQ-901, REQ-902**: untouched, cited as constraints - the engine
  never sees the map, and the map never changes on a live callback.
- No existing spec.md requirement is reversed. The reversed decisions
  are code-level (`device_config.rs`'s documented offset-only choice)
  and a TASKS.md configuration note (M6.1's L6 offset), both updated by
  this proposal.

## Verification (headless, REQ-906)

- Parse tests: `"3,4,5,6"` -> internal `[2,3,4,5]`; short lists;
  empty; junk; >4 entries errors; round-trip through the shared
  parse/format pair used by both CLI and UI.
- Migration tests: an old-format `audio.json` with
  `input_channel_offset: 2` loads as map `[2,3,4,5]`; a new-format
  file round-trips; a file with both fields prefers the map.
- Routing-function tests: full map on a big device; partial map;
  out-of-range entries (that track silent, others live); duplicate
  entries (both tracks fed); map on a 2-channel device requests 2
  channels.
- [manual] On the Pi with the L6: `--in-map 3,4,5,6` reproduces
  exactly the current `--in-offset 2` behavior; a deliberately scrambled
  map (`6,5,4,3`) routes jacks to tracks in reverse, confirmed with the
  probe command and a real signal.

## Alternatives considered and rejected

- **Keep the offset and add per-track overrides**: rejected by the
  owner directly - two interacting mechanisms to explain, test, and
  display in a Settings view with no room for them.
- **Four separate UI fields**: rejected for kiosk layout cost; the
  comma list shares its parser with the CLI and costs one line.
- **A deprecation alias for `--in-offset`**: rejected as ceremony; see
  CLI section.
- **Per-cassette mapping**: rejected - wiring is physical-setup state,
  the same reasoning that put the offset in `audio.json` and not the
  manifest.

## History

**v1 (this revision)**: initial proposal, following the owner's two
scoping decisions (map replaces offset; UI + CLI surface). Not yet
reviewed.
