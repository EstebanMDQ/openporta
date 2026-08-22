# 001: A dedicated stereo bounce buss, printed in real time

## Motivation

Requested directly by the owner while using the app, then reshaped five
times after review found real design holes (see "History" at the end -
this is now v6). The underlying problems are unchanged:

1. **Stereo information is lost.** Today's bounce is a mono sum of
   tracks 1-3 onto track 4; anything panned comes out center.
2. **Bounce is one-shot.** A second bounce replaces track 4 with a fresh
   sum of 1-3, silently discarding the first submix.

**Approach, from the owner directly:** a fifth, dedicated, always-stereo
**bounce buss** - separate storage, not one of the 4 mono tracks - that
is always part of the mix but can only ever be *written* by bouncing.
Bouncing arms the buss, presses Record, and the transport rolls in real
time recording the current mix (tracks 1-4 at whatever fader/pan/mute
you're riding live, plus the buss's own existing content, since it's
already part of that mix) into the buss, replacing it as playback
proceeds.

### This is not a pure win - say so plainly

v1 and v2 of this proposal undersold the cost. A always-mixed 5th buss is
a real step away from "the constraint IS the product" (spec section 1):
on real hardware, every bounce costs you two tracks, permanently - that
scarcity is part of what a 4-track forces you to commit to. This design
trades that economics away in exchange for stereo imaging and repeatable
layering, at a real, non-trivial memory cost (REQ-904 below roughly
doubles, ~700MB to ~1428MB steady-state, ~2.5GB worst-case peak while
undoing a bounce). That trade is the owner's to make, and it's being
asked for directly here, not slipped in as a footnote. It is not "no
cost, all upside," and this document stops framing it that way.

## Change

### Storage

Add a fifth storage area to Tape: one stereo (2-channel) i16 buffer, the
cassette's fixed length, alongside the existing 4 mono track buffers.
New tape storage, not a reuse of an existing track (see REQ-904).

### Mix

The bounce buss is always summed into the master output, at its own
fader level (muteable, not panned - it's already stereo) alongside
tracks 1-4, during ordinary playback as well as while bouncing.

### Bouncing (the "print" pass)

- A new arm-like state exists for the bounce buss, separate from the 4
  tracks' arm state (REQ-404).
- **Arming the buss and arming any of tracks 1-4 are mutually
  exclusive** (REQ-405, resolves the "not disallowed" gap v2 left open):
  arming the buss clears all 4 tracks' armed state, and arming any track
  clears the buss's armed state. No simultaneous case exists to reason
  about - a bounce pass never overlaps a live input pass on an ordinary
  track.
- With the buss armed, Record engages a real-time pass whose input is
  the current mix of tracks 1-4 (each at its own live fader/pan/mute)
  **plus the buss's own existing content at its own fader/mute**,
  computed **before** the master fader is applied (REQ-406) - see "Print
  tap point" below for why.
- The pass runs through the character chain like any record pass:
  wow/flutter shared between L and R (one modulation instance, not two
  independent ones); hiss may still be seeded independently per channel.
- Punch-in/out, the 5ms crossfade, and undo apply the same way they do
  to any record pass (see "Undo" below for the multi-channel case).
- Because the buss's own existing content is already part of what's
  being printed, a second bounce naturally folds the first one forward -
  no special self-referential summing code beyond the ordinary
  read-before-write ordering the engine already uses for undo's
  displaced-content capture (REQ-407 makes this normative, see below).

### Print tap point (REQ-406) - resolves v2's "double master fader" flag

Today, `Mixer::target()` bakes the master fader into each track's
per-sample gain before summing (`db_to_amp(fader_db) *
db_to_amp(master_db)`), so `mix_block`'s output is already post-master.
If bounce printed that value directly, riding the master fader during a
bounce would bake a master-gain multiplication onto tape - then a later
bounce would apply the *current* master gain again on top of the
already-baked one, compounding across generations. That's the "double
master fader" hazard v2's review flagged.

**Resolution, stated normatively:** the master fader MUST NOT be baked
into anything written to tape, for tracks 1-4 or the bounce buss. The
buss's print input is the sum of tracks 1-4's own fader/pan/mute-scaled
signal plus the buss's own fader/mute-scaled existing content, computed
**before** any master-fader multiplication. The master fader continues
to apply exactly once, at final output, identically whether or not a
bounce is in progress - unchanged from what REQ-602 already requires of
it. Mechanically this means `Mixer::mix_block` needs a pre-master
intermediate sum exposed alongside its existing post-master output (see
Impact on tasks); mathematically the two are related by one scalar
multiply, so ordinary playback's audible output is unchanged, though see
Impact on tasks for why this can still perturb the golden render at the
bit level.

Track-level fader/pan **do** get baked in, by design - that's the whole
point of "printing a mix," and what the owner asked for directly
("we should just create a render in realtime, so we can play with levels
and panning while it bounces"). This is a narrow, explicit carve-out to
REQ-602 for tracks 1-4's contribution **and the buss's own fader/mute
(REQ-409)** while feeding an active bounce pass; the controls themselves
stay non-destructively adjustable afterward, same as after any record
pass - moving a track's fader, or the buss's, later doesn't
retroactively change what's already printed.

**Where the tap sits relative to the hardware safety clamp (a fifth
review asked for this to be pinned explicitly)**: `mix_block` clamps
`out_l`/`out_r` to +/-1 *after* the master multiply, added earlier this
session after a real headphone-safety incident. The pre-master sum this
proposal taps is computed *before* that clamp - it has to be, since the
clamp only exists to protect what reaches speakers/headphones, a
concern that doesn't apply to an internal mix value. The bounce pass's
own tape write is bounded by a completely separate, already-existing
mechanism instead: `Dither::quantize`'s i16 clamp, the same one every
ordinary record pass already goes through. So there are two independent
ceilings, each doing its own job - the hardware clamp protects the
master output path (post-master, unaffected by any of this), and the
quantize clamp protects what lands on tape (applies to the pre-master
print tap, same as it always has) - neither is bypassed, and they don't
need to agree with each other.

### Self-reference is read-before-write, normatively (REQ-407)

For each sample position a bounce pass writes, the buss's own
contribution to that instant's mix MUST be its existing (pre-bounce)
value at that position, read before the pass's newly computed value is
written there. This is block-local read-then-write, sample-accurate -
not a separate prior full-buffer snapshot, and not a lookahead. It's the
same ordering `RecordPass` already uses to capture displaced content for
undo; a bounce pass uses it for its own input too, which is what makes
"a second bounce folds the first forward" true without any special-case
self-referential summing code.

### Shared flutter for a stereo pass - resolves v4's DSP gap

"Wow/flutter shared between L and R" has been in this proposal since
v1 and was never previously checked against the actual DSP code. A
fourth review did, and found it isn't achievable as stated:
`AudioProcessor::process(&mut self, block: &mut [f32])` is mono,
in-place, and `Flutter` (`porta-dsp/src/flutter.rs`) couples its
modulation state (the wow oscillator and flutter random walk) with its
delay line (the ring buffer audio actually passes through) in one
struct. Running two channels through one `Flutter` instance would
interleave its single ring buffer with two unrelated signals, not share
its modulation - not a subtle bug, a structurally different (broken)
result.

**Resolution**: split `Flutter` into two pieces it's already
conceptually made of - a `FlutterModulator` (the wow/walk state,
producing a delay-in-samples value per sample) and a `FlutterDelay` (a
ring buffer plus the existing Catmull-Rom read, no modulation state of
its own). `Flutter` itself becomes a thin composition of one of each -
same behavior, same tests, nothing changes for tracks 1-4. A new small
type, `StereoFlutter`, composes one `FlutterModulator` with *two*
`FlutterDelay`s (left and right) and exposes `process(&mut self, l:
&mut [f32], r: &mut [f32])`: each sample advances the modulator once and
reads both delay lines at that one delay value - genuinely shared
modulation, independent audio content per channel, exactly what REQ-402
asks for.

This does **not** touch the `AudioProcessor` trait - it stays mono and
in-place for every ordinary track (REQ-701/704 unchanged in the sense
that matters). A bounce pass isn't built as one `Chain` the way a track
is; it runs each channel through its own independent instances of every
other stage (saturation, hiss, bandwidth, optional crush - the same
`TapeCharacter` formulation, one full set per channel) with a single
`StereoFlutter::process` call in the middle where flutter belongs in
the stage order. A small, contained addition to porta-dsp, not a
widening of its general-purpose trait.

### Monitoring during a bounce pass (REQ-408) - resolves v3's REQ-305 double-sum

v3 claimed REQ-305 applied unchanged during a bounce. **That was wrong**,
and a third review caught it: a bounce pass's input already contains
tracks 1-4 (that's the whole point - it's printing their sum). If
monitoring left tracks 1-4 sounding through the mix *as well as* the
buss now carrying their sum, you'd hear them twice - roughly +6dB,
comb-filtered against themselves by the character chain's flutter delay
on the buss's copy. REQ-305 ("the user hears what the tape receives")
doesn't actually resolve this on its own for a self-inclusive pass; it
needs its own rule.

**Resolution, stated normatively (REQ-408), corrected from v5 - the
previous version had the math backwards**: v5 said the printed signal
should reach the speakers directly, bypassing the buss's own fader, to
avoid double-scaling. A fifth review traced it and found that's
*exactly* the rule that produces the jump it was trying to prevent:
write `P` = tracks-1-4-sum + `buss_gain` x buss's-prior-content (REQ-406).
Immediately after the bounce, playing that region back is `P x
buss_gain` (the buss sums into the master at its own fader, same as
always). If monitoring played `P` directly during the pass, a -6dB buss
fader would sound 6dB louder *while bouncing* than the instant you let
go of Record - a real, audible jump, the opposite of transparent.

The actual fix needs no bypass and no new mechanism at all: **the buss's
`playback` slot holds the pass's post-chain printed signal in place of
its prior tape content, and flows through `Mixer::mix_block` exactly
the way it always does** - through the buss's own smoothed fader/mute,
same code path as ordinary playback. This is precisely how monitoring
an armed *track* mid-recording already works (`engine.rs`'s
`self.playback[t] = self.processed[t]` during a pass, REQ-305) - REQ-408
extends the identical, already-proven mechanism to the buss instead of
inventing a parallel one. No discontinuity at punch-out, because the
buss fader is applied consistently before, during, and after the pass.

Tracks 1-4's own contribution to the mix still needs to go silent
during a bounce (unchanged from v5) - otherwise you'd hear them once
directly and again inside the buss's printed copy.

**Metering is not silenced (a second, separate clause of REQ-408):**
tracks 1-4's own individual meters MUST keep reflecting their live
signal during a bounce pass, independent of their audible contribution
being silenced above - otherwise the meters go dead exactly while the
user is riding those faders, defeating the feature's whole stated
purpose ("play with levels and panning while it bounces"). This *is* a
small new mechanism, not a free extension of an existing one (a
reviewer correctly caught v5 overclaiming this): `Mixer::mix_block`
computes a track's meter peak from the same input slice that feeds the
sum (`peak * fader_amp`, from `input`) - silencing a track's `playback`
slot for the sum silences its meter too, today. The fix needs a
per-track "excluded from the sum, but still metered" flag that
`mix_block` respects only during an open bounce pass - listed in
"Impact on tasks."

**Bouncing with the buss muted is destructive, on purpose, not a bug**:
per REQ-406, the print input includes the buss's *own* existing content
"at its own fader/mute" - so a bounce with the buss muted excludes the
buss's prior content from what gets printed, replacing rather than
folding it forward. Stated here explicitly (v4 left it implicit and a
reviewer flagged it as an accident waiting to happen) because it's the
mute control doing exactly what mute does, the same as muting a track
before recording over it - not a special case to design around.

### What doesn't change

- Tracks 1-4 stay exactly as they are: 4 mono, armable, recordable, with
  fader/pan/mute/monitor. REQ-601-602 apply to them unchanged outside
  the narrow bounce-pass carve-out above. REQ-603 no longer describes
  bounce (it never sums tracks through pan anymore in any form).
- Export/WAV mixdown: unaffected in shape - the buss just becomes one
  more thing already folded into the post-master output when present.

## Requirements affected (settled decisions being reversed or extended)

- **Definitions** (section 3): "Tape" becomes "4 fixed-length mono i16
  buffers plus one fixed-length stereo i16 buffer (the bounce buss), all
  at 48kHz." "Bounce" becomes "a real-time record pass onto the
  dedicated stereo bounce buss, whose input is the pre-master-fader sum
  of tracks 1-4 (at their live fader/pan/mute) plus the buss's own
  existing content (at its own fader/mute)." "Record pass" gains a
  clause: a pass onto the buss writes both channels atomically as one
  pass for undo purposes (see REQ-502 below) - still "one continuous
  record engagement," now on a buss instead of a track.
- **REQ-101**: the cassette gains a fifth, always-stereo storage area
  that is not one of the 4 tracks and has a different capability set
  (mix-only input, no arm for ordinary recording, mutually exclusive
  with tracks 1-4's arm state). The 4-mono-track guarantee for tracks
  1-4 themselves is unchanged.
- **REQ-401**: rewritten - see "Definitions" above.
- **REQ-402**: intent unchanged (character chain still applies, still
  compounds); wording updated for a stereo pass with shared flutter.
- **REQ-403**: acceptance-test procedure needs re-verification under the
  new bounce - see "Impact on tasks."
- **REQ-404 (new)**: the bounce buss MUST have its own arm-like flag,
  independent of tracks 1-4's `armed` array, with no ordinary-input
  recording capability.
- **REQ-405 (new)**: arming the bounce buss and arming any of tracks 1-4
  MUST be mutually exclusive; arming one MUST clear the other.
- **REQ-406 (new)**: the master fader MUST NOT be baked into any signal
  written to tape (tracks 1-4 or the bounce buss); a bounce pass's input
  MUST be computed before any master-fader multiplication.
- **REQ-407 (new)**: a bounce pass's own prior content at a given tape
  position MUST be read before the pass's new value is written to that
  position (block-local read-before-write; no lookahead).
- **REQ-408 (new)**: while a bounce pass is open, tracks 1-4's own
  contribution to the audible output MUST be silent; the audible output
  MUST be the pass's post-chain printed signal, output directly (not
  re-scaled by the buss's own fader/mute a second time). Track-level
  metering MUST NOT be silenced by this - it keeps reflecting each
  track's live signal. Resolves the double-sum a naive REQ-305 reading
  produces for a self-inclusive pass, and the dead-meters gap a fourth
  review caught in v4's version - see "Monitoring" above.
- **REQ-409 (new)**: the bounce buss MUST have its own volume fader and
  mute (REQ-406/408 both depend on "the buss's own fader/mute" already
  existing), independent of tracks 1-4's (REQ-601) - no pan, since it's
  already stereo. No requirement currently establishes this; REQ-601 is
  track-scoped. Both MUST be smoothed the same 5ms way every other
  mixer control already is (`mixer.rs`'s existing ramp) - it matters
  more here than for an ordinary track, since these values get printed
  to tape, not just heard. REQ-406's carve-out to REQ-602 (tracks 1-4's
  fader/pan baked in during a bounce) extends to the buss's own
  fader/mute too - it's baked into the print the same way, for the same
  reason (a reviewer pointed out the carve-out as originally worded only
  named tracks 1-4).
- **REQ-301**: "recording MUST engage only on armed tracks" needs
  "...or the armed bounce buss" - a bounce records onto something that
  is not a track.
- **REQ-306**: "unarmed tracks MUST be byte-identical before/after any
  record pass" gains a buss-shaped analogue for free, worth one clause
  rather than leaving it implicit - the buss MUST be byte-identical
  across an ordinary track pass, and tracks 1-4 MUST be byte-identical
  across a bounce. REQ-405's mutual exclusivity already makes both
  trivially true by construction; stating it is cheap and closes the
  symmetry.
- **REQ-502**: the undo journal's entry format MUST extend to cover a
  multi-channel (stereo) pass as a single atomic entry - see "Undo." Its
  byte cap (`DEFAULT_MAX_BYTES`, 512MB) is unchanged by this proposal,
  which has a real consequence - see "Impact on tasks."
- **REQ-503**: journal reload (`Journal::load`) already silently
  discards the whole undo stack on a parse failure
  (`if let Ok(state) = serde_json::from_str(...)`) - see "Persistence"
  below for why the multi-channel entry's shape is chosen not to trip
  this for existing cassettes.
- **REQ-602**: gains the narrow bounce-pass carve-out described above;
  otherwise unchanged.
- **REQ-603**: no longer describes bounce at all; tracks 1-4's own
  REQ-601/602 behavior is untouched.
- **REQ-702**: "hiss... independently per channel" needs a decision, not
  a MAY - see "Persistence and reproducibility" below.
- **REQ-801/802**: the buss needs its own on-disk storage and dirty-
  chunk tracking, and `Project::open`/`load_tape` need a path for
  cassettes saved before this feature existed - see "Persistence" below.
- **REQ-804**: session scripts (REQ-804) can't currently express a
  bounce at all under this design - `Op::Record` requires a WAV input a
  bounce pass doesn't have, and there's no op to arm the buss. See
  "Session-script support" below - this is required for every new test
  this proposal lists, not an optional nicety.
- **REQ-701/704 (porta-dsp)**: unchanged in the sense that matters most -
  `AudioProcessor` stays mono and in-place, tracks 1-4's chains are
  untouched. But porta-dsp gains a new type (`StereoFlutter`, see
  "Shared flutter for a stereo pass" below) used only by a bounce pass,
  which is not built as an ordinary `Chain`. Worth listing because it's
  new surface in a crate REQ-901 keeps hardware-agnostic - it stays that
  way; this is pure DSP, no new dependency.
- **Section 2 (Scope), replacement text drafted, not just flagged**:
  "4 mono tracks, one stereo master output" becomes "4 mono tracks, one
  stereo master output, plus one fixed, mix-only stereo bounce buss (not
  a 5th track: no arm for live input, no pan, exists only to receive a
  printed mix, cannot be added to or removed - REQ-101/404)."
  "destructive bounce" becomes "destructive real-time bounce onto the
  buss." This addresses the "track group" ambiguity a reviewer flagged
  directly, in the same document that moves scope, rather than leaving
  it for a reader to infer from the REQ list.
- **`Command::Bounce` removal**: this design deletes the old blocking
  batch command entirely (`command.rs`'s `Command::Bounce` variant and
  its `is_blocking()` match arm, `Engine::bounce()`, and the
  `disk_touching_commands_are_marked_blocking` test's assertion about
  it) in favor of arm-the-buss + ordinary Record - stated in "What
  doesn't change" implicitly before; explicit here because a reviewer
  pointed out only the golden/cli *test* impact was listed, not the
  removal itself.
- **REQ-804 / existing session scripts**: `{"op":"bounce"}` (no fields)
  parses today; `Op::Bounce { seconds: f32 }` makes that a parse error.
  Session scripts are test/audition fixtures within this repo, not
  persisted user data the way a cassette or its undo journal is (REQ-503
  cares about the latter, not the former) - so this is a small,
  mechanical chore (update the repo's own script fixtures that use the
  old op) rather than a compatibility requirement needing default-value
  plumbing. Worth listing so it isn't missed during implementation, not
  because it's a REQ-804 violation. Concretely: `tests/golden.rs:99`,
  `tests/cli.rs:208`, and `auditions/m3-session.json:14` all use
  `{"op":"bounce"}` today and all need updating to the new op shape.
- **Section 6 (acceptance gates)**: M2's gate text ("REQ-403 generation-
  loss test passes") and M3's ("the single golden render passes") both
  still apply in spirit, but the underlying test/procedure each refers
  to changes under this proposal (REQ-403's rewritten procedure, the
  golden render's regeneration) - both gates need re-pointing at what
  actually exists once this lands, not just re-passing by coincidence.
- **REQ-904**: revised - see "Impact on tasks."

### Undo

A bounce pass writes two channels of one buss, not one track. To keep
REQ-505's "no incoherent intermediate state" guarantee, **the journal's
`Entry` gains support for a multi-channel pass as a single atomic
record**: one entry spanning both channels' displaced payload, one undo
press fully reverts a bounce. This is the "single entry" option v2's
Undo section offered, chosen over "two entries always paired" because it
removes the pairing hazard entirely rather than managing it: eviction
(`Journal::evict`, oldest-first, one entry at a time today) can't split
what was never two entries to begin with. Ordinary track passes keep
using the existing single-channel entry shape unchanged - this is an
additive variant, not a rewrite of the whole journal format.

`Journal::undo`/`redo` themselves need a real second-channel code path,
not just a format change - today each does exactly one `read_raw`/
`write_raw` pair against `entry.track` (and, for `undo`, one
`read_payload`/`write_payload` pair). A stereo entry needs that same
sequence run twice, once per channel (`track` and `right_track`), and
both must succeed or fail together to honor REQ-505 - listed here
because a reviewer found the format change alone doesn't imply the
restore logic follows.

### Persistence and reproducibility

- **REQ-702 (hiss seeding, decided, not left as MAY)**: the noise seed
  derivation gains a channel term - `seed_for(noise_seed, pass_id,
  channel)`. Ordinary tracks always pass a fixed channel value (e.g. 0),
  so their seeds are bit-identical to today - no behavior change, no
  golden-render perturbation from this specific piece. A bounce pass
  passes 0 for its left channel and 1 for right, giving correlated
  wow/flutter (one modulation instance shared between channels, per
  "Bouncing" above) but decorrelated hiss between L and R, as intended.
- **REQ-801/802 (buss storage)**: the buss's audio lives in its own two
  raw i16 files (`tape/bounce_l.raw`, `tape/bounce_r.raw`), written in
  the same 5-second dirty-chunk pattern as tracks 1-4
  (`project.rs`/`tape::CHUNK_SAMPLES`) - not a new storage pattern, two
  more files of the existing kind. `Project::open`/`load_tape` treat
  missing buss files as "never bounced yet" (all-zero, matching how a
  fresh cassette's tracks already start) rather than an error, so every
  cassette saved before this feature exists opens unchanged.
- **REQ-503 (journal format stays backward compatible)**: `Entry` gains
  one additive field, `right_track: Option<usize>` (`#[serde(default)]`,
  matching the precedent already used for `Manifest::muted`) - `None`
  for every existing single-channel entry (unchanged meaning), `Some(r)`
  only for a bounce entry, whose `track` field holds the buss's left
  "virtual track index" and `right_track` its right. `Journal::load`'s
  existing silent-discard-on-parse-failure behavior is unaffected either
  way - this change can't be what triggers it, since old journals simply
  never have the field. **`len`'s meaning, pinned explicitly (a fifth
  review found this undefined)**: `len` stays per-channel sample count,
  exactly like every existing single-channel entry - a stereo entry's
  *total* resident payload is `len * 2 (channels) * 2 (bytes/sample)`.
  `Entry::bytes()` (which `evict()` sums against `max_bytes`) needs a
  `right_track.is_some()` branch that doubles accordingly, or eviction
  silently undercounts every bounce entry by half.

### Session-script support (REQ-804)

Today's format has no way to express a v3/v4 bounce at all:
`Op::Record` requires a WAV input a bounce pass doesn't have, and there
is no op to arm the buss - meaning none of this proposal's new tests, or
the golden render, would have a headless driver without an addition
here. Four new ops, matching the shape of what's already there:

- `Op::Mute { track: usize, on: bool }` - the engine already has
  `Command::Mute`; the script format never needed it before because no
  test cared about a muted track's exact contribution. REQ-403's
  rewritten procedure (below) does.
- `Op::BounceArm { on: bool }` - arms/disarms the buss (REQ-404/405).
- `Op::BounceFader { db: f32 }` / `Op::BounceMute { on: bool }` -
  REQ-409's buss fader/mute, existing today only as engine-internal
  state with no track index to attach to (`Op::Fader`/`Op::Mute` are
  both range-checked against `NUM_TRACKS`, and the buss isn't one of
  them). Without these, REQ-408's own test (distinguishing "the buss
  fader applies once, consistently" from "applied twice" needs a
  non-unity buss fader to even observe a difference) and "bouncing with
  the buss muted is destructive" are both unwritable - a fifth review
  caught that the two mute/arm ops alone don't cover this.
- `Op::Bounce { seconds: f32 }` - requires the buss already armed
  (errors otherwise, same as `Op::Record` on an unarmed track today);
  engages the pass and runs the transport for `seconds`, mirroring
  `Op::Play`'s existing shape exactly. Riding a track's fader *during*
  a scripted bounce isn't itself a new scriptable primitive - ops still
  execute strictly sequentially, and that's already true for ordinary
  tracks today (there's no way to script "ride a fader while playing"
  either). What's new and does need to be scriptable is bouncing
  *between* two different fixed settings, which needs no new op shape:
  `Op::Fader`, `Op::Bounce{...}`, `Op::Fader`, `Op::Bounce{...}` in
  sequence covers the REQ-406 test below (two bounces, two master
  positions) and the REQ-403 procedure (bounce, mute, bounce, bounce)
  without inventing mid-pass automation.

## Impact on tasks

- **Storage**: Tape gains a fifth (stereo) buffer, same fixed-length
  preallocation model as the existing 4 tracks (see REQ-904 below for
  the memory consequence) - no new storage *pattern*, just one more
  buffer of the same kind. On-disk layout: see "Persistence" above.
- **Realtime-safe allocation - already landed, not just designed, and
  already survived one round of "does it actually hold up" scrutiny**:
  the REQ-902 gap v2/v3 flagged (`RecordPass::with_capacity`'s
  `reserve_exact`, sized to the whole remaining tape, running directly
  on the realtime thread since `Command::Record` isn't blocking) is
  fixed as of `record.rs`'s chunked-capture rewrite (see TASKS.md, M4.4's
  closed-out follow-up) - a real, pre-existing bug in ordinary track
  recording, shipped and tested independent of whether this proposal is
  ever accepted. A first version of that fix shipped with its own bug (a
  shared pool whose `take_spares` handed out N chunks per pass but only
  ever got back the ones actually used, draining to nothing within a
  handful of takes) - caught by a fourth review checking the code
  directly, fixed in a follow-up commit the same day. The design that
  actually stands now: each track owns a dedicated reserve of
  pre-allocated chunks (`Journal.chunk_pool: [Vec<Vec<i16>>; NUM_TRACKS]`,
  `CHUNK_POOL_PER_TRACK` each), handed to a new pass and returned by a
  closed one entirely via `mem::take`/plain moves - no partial-take
  container to build, no allocation at either end. Extending this to the
  buss's two channels means growing that array by two more dedicated
  slots (own constant, not folded into `NUM_TRACKS`) - the same
  mechanism, not a new one, +2 x `CHUNK_POOL_PER_TRACK` x
  `tape::CHUNK_SAMPLES` x 2 bytes = ~23MB. `RecordPass::finish`'s
  `.to_vec()` allocation and `push_pass`'s `format!`/`PathBuf` filename
  computation (a second and third realtime-thread allocation the same
  review caught) are also already fixed in the same commits -
  `Entry.file` doesn't exist anymore; the filename is always derived
  from `id`.
  - **The buss does NOT extend this mechanism - a fifth review found
    that plan doesn't work, and it's right**: `CHUNK_POOL_PER_TRACK` (24
    chunks, ~2 minutes) is sized for an ordinary take. A bounce is not
    an ordinary take - by definition it's close to the full remaining
    tape, every time. A 3-minute bounce alone needs 36 chunks per
    channel with nothing flushing in between; a 15-minute one needs 180.
    Extending the *same* small reserve to the buss means the "rare
    fallback" path v5 described is actually the *common* case for this
    specific operation - which defeats the point. **Resolution: the buss
    gets its own, different mechanism - one dedicated reserve per
    channel, sized to the cassette's full length**, not a small
    per-take share. This is allocated once, off the realtime thread, at
    cassette open/create - the same moment ordinary `Tape` storage
    itself is allocated - and handed to a bounce pass wholesale via the
    same `mem::take` pattern already proven for tracks (a `[Vec<i16>;
    2]`-shaped reserve, or the two-channel equivalent, not a
    `Vec<Vec<i16>>` pool of small chunks at all - a bounce pass doesn't
    need to roll between chunks; it gets one buffer, sized for anything
    up to the whole tape, and that's the end of the allocation question
    for that pass). It's given back the same way tracks give back
    theirs: immediately on close (whatever the pass didn't use) plus
    whatever's left after the next flush. This is a real, larger memory
    commitment - see REQ-904 below, recomputed to include it honestly -
    not a cost this proposal gets to understate again.
  - **Known, separate, pre-existing risk not addressed here**: the
    journal's `push_pass`/`evict` also run on the realtime thread today
    (reachable from `Stop`, which isn't blocking either) and grow plain
    `Vec`s (`undo: Vec<Entry>`, `pending_writes: Vec<(u64, usize,
    Vec<Vec<i16>>)>`). That's a second, smaller realtime-allocation risk
    (small, pointer-sized container growth, not bulk sample data),
    already present for ordinary tracks, not made categorically worse by
    this proposal (still one push per pass, mono or the new
    multi-channel entry alike) - flagged for honesty, worth its own
    future task, not a blocker here.
  - **Not yet done, flagged by the fourth review as the way to make this
    invariant load-bearing rather than inferred from passing tests**: a
    global-allocator-backed counting harness around `record()`/
    `process_block()`/`stop()` that would catch *any* future realtime-
    thread allocation directly, including a regression to a completely
    different (non-chunked) implementation that the current tests
    wouldn't structurally notice. Worth its own task independent of
    whether this proposal proceeds.
- **`Mixer::mix_block`**: needs a pre-master-fader intermediate sum
  exposed (today `target()` bakes `master_db` into every track's
  per-sample gain before summing). The cleanest shape: compute the
  existing per-track sum without the master factor, expose it, then
  apply master gain as one scalar pass over `out_l`/`out_r` for the
  audible/export path - mathematically identical for ordinary playback,
  but floating-point multiply isn't strictly associative, so this
  reordering can perturb the existing golden render at the bit level.
  That's on top of the already-known golden-regen need from removing
  `{"op":"bounce"}` (`tests/golden.rs`, `tests/cli.rs:208`) - one
  regeneration event, one TASKS.md note, one notification, covering
  both causes. `Mixer` also needs a per-track "excluded from the sum,
  still metered" flag for REQ-408's metering clause (`mix_block`
  currently derives a track's meter peak from the same slice that feeds
  the sum, `peak * fader_amp` off `input` - silencing a track's
  contribution today silences its meter with it; this is a small, new,
  explicit mechanism, not a free ride on an existing separation).
- **Chain-splitting in porta-dsp**: `TapeCharacter::build_chain` returns
  one monolithic `Chain` with no way to build the stages either side of
  flutter separately. A stereo bounce pass needs that split (independent
  per-channel saturation/hiss/bandwidth/crush around one shared
  `StereoFlutter` step) - a small new builder method, not just the three
  flutter types already described. `Flutter::new`'s depth-clamp
  constants (`.min(CENTRE - 4.0)`, `.min(CENTRE / 4.0)`) depend on the
  delay geometry and must stay shared between `Flutter` and
  `StereoFlutter`'s construction, not redefined twice and allowed to
  drift. (Also: the real stage order in `build_chain` today is
  Saturation, Hiss, Bandwidth, Flutter, Crush - flutter is last, not "in
  the middle" as an earlier version of this document said; wherever
  `build_chain` actually puts it is where `StereoFlutter` goes too.)
- **Latency accumulation across fold-forward bounces - open, not
  resolved here**: `Flutter`'s delay line has a fixed ~480-sample (10ms)
  centre tap, reported via `latency_samples()` but not currently
  compensated anywhere in the engine. Each bounce shifts everything
  already printed a further ~10ms relative to tracks 1-4, compounding
  with every generation - a direct consequence of "a second bounce folds
  the first one forward." Two real options exist (accept the drift, real
  tape doesn't perfectly time-align either; or compensate using the
  already-present but unused `latency_samples()` when reading the buss's
  prior content) and this document doesn't pick one - flagged honestly
  as unresolved rather than papered over, since it also affects the
  stereo-image and REQ-403 tests' validity across several generations.
- **REQ-905 / M6 CPU headroom - open, not resolved here**: today's
  bounce is an offline batch operation with no realtime deadline. This
  design makes it a realtime operation running two full character
  chains (independent per-channel saturation/hiss/bandwidth/crush plus
  one shared `StereoFlutter`) inside the same audio callback as tracks'
  own chains, on a Pi 4 at a 128-256 frame period. Nothing in this
  proposal measures or bounds that cost - it needs real on-device
  profiling at implementation time, the same way M6.2's performance pass
  already covers tracks. Not a paper decision.
- **New arm-like flag** for the bounce buss (REQ-404), plus the mutual-
  exclusion wiring with tracks 1-4's `armed` array (REQ-405).
- **`process_block`**: the buss becomes a fifth mix contributor (fader +
  mute, no pan) in both ordinary playback and while a bounce pass is
  running; its own read during a pass follows REQ-407's read-before-
  write ordering.
- **Journal**: `Entry`/`RecordPass` gain a multi-channel variant used
  only by the buss (ordinary tracks keep the existing single-channel
  shape) - see "Undo" above.
- **REQ-403's test, rewritten procedure (corrected again - v4's script
  used Arm, which does nothing to a track's mix contribution; muting
  is what's needed, and `Op::Mute` didn't exist yet)**: the old three-
  successive-bounces procedure doesn't transfer - because the buss folds
  its own prior content forward every pass, re-bouncing *unchanged,
  unmuted* source material re-injects full-bandwidth signal each
  generation, so measuring HF energy/noise floor across generations 1-3
  would measure "source material present or not," not generation loss.
  All three *measured* generations need identical input conditions:
  bounce once with tracks 1-4 unmuted (primes the buss), then mute
  tracks 1-4 for real and bounce three more times, measuring generations
  2, 3, and 4 (buss re-printing only its own prior content each time)
  for the existing monotonic HF-loss/noise-floor-rise assertion.
  Scriptable via `Op::BounceArm{on:true}`, `Op::Bounce{...}`,
  `Op::Mute{track,on:true}`x4, `Op::Bounce{...}`x3.
- **New test**: stereo image survives a bounce and a second bounce (a
  hard-panned source stays audibly on its side, no collapse toward
  center).
- **New test**: `StereoFlutter`'s two channels' delay excursions
  correlate (driven by one `FlutterModulator`), verifiable directly by
  feeding it identical input on both channels and asserting the outputs
  match - simpler and more precise than inferring it from a full bounce
  render. A second test confirms hiss stays decorrelated between the
  two channels of an actual bounce pass (REQ-702).
- **New test**: `Flutter`'s own behavior (tracks 1-4's chain) is
  unchanged by its internal split into `FlutterModulator` +
  `FlutterDelay` - same existing tests (its own module's, generation-
  loss suite) pass without modification; this is refactor-safety, not a
  new requirement.
- **New test, corrected again from v4**: riding the master fader during
  a bounce does not change what's printed (REQ-406). v3 asked for
  byte-identical output from two bounces of "identical material" -
  wrong, because pass seeds differ. v4's fix (a passthrough character)
  was *also* wrong, caught by a fourth review: dither is seeded per pass
  and lives in `RecordPass` itself, applied unconditionally in
  `write_block` regardless of what the character chain does - a
  passthrough chain doesn't touch it. `Engine::undo()` doesn't roll
  `pass_counter` back either, so bounce-Undo-bounce still gets two
  different seeds. Corrected version, since `seed_for(noise_seed, pass)`
  depends only on the cassette seed and the pass *index* (not on what
  was recorded before it): build two fresh cassettes with the same
  seed, run identical op sequences on each - including whatever track
  recording precedes the bounce, so the bounce lands at the same pass
  index both times - differing only in the `Op::Master` value set
  before the bounce. Assert the two printed regions are byte-identical.
- **New test, corrected a second time (v5's version was still vacuous)**:
  peak level after several successive bounces of hot (0dBFS) material.
  v3's "stays within full scale via `Tape::read`" could never fail
  (dividing an `i16` by 32768 always lands in range). v5's fix - "no
  sample overflows i16 range... via the raw i16 read" - is the same
  non-claim in different words: an `i16` cannot be outside `i16` range
  by construction, and both `Dither::quantize`'s explicit `.clamp(...)`
  and a bare `as i16` cast saturate in Rust, so there was never anything
  for that assertion to catch. Decided and made genuinely falsifiable
  instead: clipping under sustained hot self-inclusive summing is
  accepted, expected behavior (real tape saturates the same way under a
  gain-staging mistake) - the test's job is to confirm the clamp
  *engages* under real pressure, not that overflow is impossible (it
  already is, trivially). Assert, after 5 generations of 0dBFS material,
  that the fraction of samples sitting exactly at the clamp boundary
  (32767 or -32768) is nonzero - a healthy, non-clipping signal would
  essentially never land exactly there, so a nonzero count is real
  evidence the clamp did something. No upper bound is asserted in this
  document - guessing one (v5's "under 50%") without having actually run
  the pass is asserting a number nobody has measured; pin a regression
  bound from the real figure once this is implemented, not before.
- **New test**: while a bounce pass is open, the audible output matches
  the printed signal scaled by the buss's *current* fader/mute
  consistently - set a non-unity buss fader (`Op::BounceFader`) and
  assert the same scaling applies during the pass and immediately after
  it closes, no jump at punch-out (REQ-408's core claim, corrected from
  v5's inverted rule). A second assertion: each track's own meter still
  reflects its live signal during the pass rather than reading silent
  (REQ-408's metering clause).
- **New test**: one Undo press after a bounce restores both channels
  atomically - no reachable state with one channel reverted and the
  other not (REQ-502/505).
- **New test**: a pass onto the buss's two channels, run back-to-back
  with ordinary track passes, doesn't exhaust the chunk pool budget in
  ordinary use - covered by extending the existing pool-budget math
  (see "Realtime-safe allocation" above), not a new mechanism to test in
  isolation.
- **REQ-904 (resident memory ceiling), itemized carefully this time -
  this number has been wrong twice already, both times from leaving out
  a real mechanism, so this version lists every contributor explicitly
  and rounds conservatively rather than claiming false precision**:

  *Steady-state* (cassette open, nothing mid-flush):
  - Tape storage: 4 tracks x 172.8MB + 1 stereo buss x 172.8MB/channel
    = 691.2MB + 345.6MB = **1036.8MB**
  - Track chunk pool (already shipped, independent of this proposal):
    4 x `CHUNK_POOL_PER_TRACK`(24) x `CHUNK_SAMPLES`(240,000) x 2 bytes
    = **~46MB**
  - Buss dedicated reserve (this proposal's new mechanism, see
    "Realtime-safe allocation" above - full-tape-sized, not chunked):
    2 channels x 172.8MB = **345.6MB**
  - Steady-state total: **~1428MB**

  *Additional worst-case transient*, during an Undo of a full-length
  stereo bounce entry (a fifth review's finding: `Journal::undo`'s
  `current` read, `read_payload`'s disk-read buffer, and
  `write_payload`'s byte-encoding buffer are all separate, temporary
  allocations, each sized to the whole entry, all live briefly at once):
  roughly 3x a full stereo entry's size, 3 x 345.6MB = **~1037MB**,
  on top of the steady-state figure above, only while that specific
  operation runs.

  **Worst-case peak: ~1428MB + ~1037MB ≈ ~2.5GB.** That's deliberately
  a generous, additive estimate, not a tight bound - the only claim that
  needs to hold is "fits comfortably in the real device's headroom,"
  which it does by a wide margin regardless of exactly how conservative
  this arithmetic is: checked against the actual deployment Pi
  (`patch@192.168.68.55`, confirmed via `free -h`: 8GB total, ~5.8GB
  free at idle with the desktop session and audio stack already
  running) - ~2.5GB peak against ~5.8GB free leaves over 3GB of margin
  even in the worst case. The ceiling is revised from ~700MB to ~2.5GB
  peak (~1428MB steady-state; default 15-minute cassette roughly half
  each figure) rather than shortening max cassette length or the buss,
  because on the real target hardware there is no actual memory
  pressure to trade against. If this project ever targets a
  smaller-RAM Pi 4 variant, this whole section needs recomputing against
  that device's real headroom, not assumed to still hold - noted here
  so it isn't forgotten.

  Said plainly, since a reviewer asked for the basis to be explicit:
  REQ-904's basis is changing from "tape buffers alone" to "tape buffers
  plus every realtime-safety reserve this proposal and its prerequisite
  depend on." The already-shipped 46MB track pool alone puts *today's*
  actual resident figure at ~737MB, already past the currently-
  documented ~700MB, independent of whether the buss itself is ever
  accepted - `spec.md`'s number needs updating either way.
- **REQ-502 sizing consequence, stated and accepted, not solved**: a
  full-length stereo bounce entry is ~345.6MB against the journal's
  default 512MB cap - one bounce alone consumes roughly two-thirds of
  the budget, and `evict()`'s "oldest-first, keep at least one entry"
  logic means a second full-length bounce entry can't coexist with the
  first; ordinary track undo history gets evicted first to make room.
  This proposal does not raise `DEFAULT_MAX_BYTES` to compensate -
  doing so would weaken the cap's actual purpose (bounding resident
  pending-payload memory) for every cassette, not just ones that bounce.
  Accepted as a real, known trade: undoing more than one or two bounces
  back is already a niche need REQ-505's own philosophy (no history
  browser, destructive by design) doesn't especially prioritize: the
  most recent bounce and a modest amount of ordinary track history
  staying undoable is the realistic guarantee, not "everything, always."
- `TASKS.md` M3.1 (bounce, currently `[x]`) and its verify text need
  updating - a re-open of a done milestone task.
- `openspec/spec.md` itself needs every REQ above rewritten once this
  proposal is accepted, plus the "Tape"/"Bounce"/"Record pass"
  definitions.

## Alternatives considered and rejected

- **Reuse tracks 3+4 as the stereo destination** (v1's design): rejected
  - only nets 2 free tracks per bounce instead of 3, and re-summing
    through the destination tracks' own pan on a second bounce collapses
    the stereo image (see History).
- **Mono self-inclusive bounce onto track 4 only** (the cheapest fix,
  raised by v1's own reviewer): solves repeatability alone, at far lower
  spec blast radius and zero extra memory. Rejected because it drops
  stereo information entirely, and the owner was explicit that losing
  stereo information is one of the two problems being solved here, not
  an acceptable trade.
- **Offline/batch bounce** (compute the sum programmatically, outside
  the realtime callback entirely): would sidestep the whole REQ-902
  allocation question, and be simpler to implement. Rejected because the
  owner asked for a real-time *performance* specifically - "play with
  levels and panning while it bounces" - which a batch operation
  structurally cannot offer.
- **Keep `Command::Bounce` as a separate blocking batch command**,
  layered on top of the new buss instead of reusing arm+Play/Record:
  rejected because it reintroduces the pre-existing gap (bounce wasn't
  reachable from the live UI or interactive session) and, like the
  offline option, can't be ridden live.
- **A variable-length (grow-as-you-bounce) buss** instead of a fixed
  full-tape-length buffer: rejected for consistency with how tracks 1-4
  already work (fully preallocated for the whole tape regardless of how
  much is actually recorded) and because growing it during a live pass
  hits the same REQ-902 problem this proposal already has to solve for
  the fixed-size case, with less benefit.

## History

**v1** reused tracks 3 and 4 as a self-inclusive stereo destination.
Review found it reversed more of the spec than it stated (REQ-602,
REQ-304, the "Record pass" definition, REQ-502's accounting), had a real
bug (the destination tracks' own default-center pan would collapse the
stereo image on the second bounce), decorrelated wow/flutter between the
two destination channels would smear the image even without that bug,
and overclaimed "unlimited layering" when every layer re-degrades
everything already printed.

**v2** was the owner's own redesign: a dedicated, always-stereo bounce
buss, printed in real time, not reusing ordinary tracks. This avoided
essentially all of v1's problems, but a second review found it still had
real, blocking gaps: no answer for where bounce-pass buffers get
allocated without violating REQ-902 in the realtime callback; REQ-904's
memory ceiling breach stated as "worth revisiting" instead of resolved
with a number; an unstated "double master fader" hazard at the print tap
point; the self-reference/read-order rule left implicit rather than
normative; REQ-602's carve-out and REQ-305's interaction left
unaddressed; the "not disallowed" simultaneous-arm phrasing was an
absent requirement, not a decision; the golden-render/cli-test impact
was known but not listed; and the "pure win" framing of an always-
audible 5th buss was called out as dishonest given what it actually
trades away.

**v3** addressed v2's gaps: a pre-allocation strategy for REQ-902; REQ-904
resolved with a real number, checked against the actual deployment Pi's
free memory over SSH; the print tap point pinned to pre-master-fader
(REQ-406); the self-reference rule made normative (REQ-407); REQ-602's
carve-out; simultaneous-arm resolved as mutually-exclusive (REQ-405);
and an explicit "this is not a pure win" framing. A third review found
real problems in the resolutions themselves, not just gaps: the REQ-902
"pre-reserve once" strategy didn't account for `Journal::push_pass`
moving the buffer's ownership away permanently (so a second pass on the
same track would allocate again exactly as before); a second, missed
realtime allocation in `RecordPass::finish`'s punch-out fade; REQ-904's
revised number was internally inconsistent with the REQ-902 fix (it
assumed lazy per-pass allocation while the fix pre-reserved everything
upfront, which would actually cost ~2.4GB, not ~1.4GB peak); the "REQ-305
applies unchanged" claim was wrong and would have shipped an audible
+6dB double-sum while bouncing; three of the five proposed tests
couldn't pass as written (seeds differ per pass, so a byte-identical
comparison across two bounces was never possible without a passthrough
chain; the REQ-403 procedure confounded two different input conditions;
a promised peak-level test didn't actually exist in the list); REQ-804
was violated with no way to even express a bounce in a session script;
and the affected-requirements list was missing section 2, REQ-301,
REQ-702, REQ-801/802, REQ-503, and REQ-502's real sizing consequence.

**v4**: the REQ-902 fix shipped as its own commit (chunked pass capture,
`record.rs`), REQ-904 recomputed against it, REQ-408 added for
monitoring during a bounce, the REQ-406/403 tests corrected, two new
session-script ops proposed, and the affected-requirements list mostly
completed. A fourth review checked the shipped code directly rather
than trusting the design doc, and found real problems again: the chunk
pool's `push_pass` only ever returned chunks a pass *used*, never the
ones it reserved and didn't, so a track's reserve drained to nothing
within ~4 ordinary takes - the fix didn't hold up in steady state,
exactly the kind of thing "shipped, not just designed" was supposed to
rule out. Also caught: `take_spares` allocated despite its own doc
comment; `push_pass` built each entry's filename with `format!`+
`PathBuf` on the realtime thread; REQ-408's monitoring rule left the
buss-fader double-application and track-metering questions open;
the REQ-406 test's "use a passthrough chain" fix didn't work (dither is
seeded per pass regardless of the chain); the peak-level test's
assertion could never fail; REQ-403's script used the wrong op
(`Arm`, not `Mute`) and `Op::Mute` didn't exist; and - the most
consequential finding - REQ-402's "shared flutter between L/R," present
since v1, isn't achievable with porta-dsp's mono, in-place
`AudioProcessor` trait as it exists today, and nothing had checked that
against the actual code until this pass.

**v5**: the chunk-pool leak was fixed for real (a dedicated per-track
reserve, `mem::take`/plain moves only, verified with a regression test
that would have failed the v4 code within 2 takes) and shipped as its
own commit. `take_spares`'s allocation and `push_pass`'s filename
computation were both eliminated. REQ-408 gained buss-fader-output and
metering clauses; a new REQ-409 gave the buss its own fader/mute.
Shared flutter got the `FlutterModulator`/`FlutterDelay`/`StereoFlutter`
design. A fifth review verified the shipped code held up this time
(it did), but found the *new* material had its own real problems:
REQ-408's rule was mathematically backwards - tracing the actual math
showed "print directly, no buss fader" produces exactly the punch-out
discontinuity it claimed to prevent, not the reverse; the extended
chunk-pool plan for the buss doesn't work at all, because
`CHUNK_POOL_PER_TRACK` (2 minutes) is sized for an ordinary take and a
bounce is by definition close to the full tape - the "rare fallback"
becomes the normal case for this specific operation; REQ-904 was wrong
a third time (missed the resident cost of unflushed payloads and the
transient cost of undoing one); the peak-level test's "fixed" assertion
was still unfalsifiable (an `i16` cannot be outside `i16` range,
obviously in hindsight); REQ-409 didn't extend REQ-602's carve-out to
the buss's own fader/mute or state its smoothing; the stereo journal
entry's byte-accounting (`len` per-channel or total?) was undefined;
and several real implementation surfaces were still missing from Impact
on tasks (a `Chain`-splitting builder in porta-dsp, latency
accumulation across generations, REQ-905/M6 CPU headroom for two
character chains in the realtime callback, a script op to actually set
the buss's fader for REQ-408's own test to be writable).

**v6 (this revision)**: REQ-408 rewritten with the corrected direction -
the buss's `playback` slot carries the printed signal through the exact
same monitoring mechanism tracks already use (REQ-305), not a bypass;
no discontinuity, no new special case. The buss gets its own dedicated,
full-tape-sized reserve, not an extension of the small per-track chunk
pool - a genuinely different mechanism for a genuinely different access
pattern, acknowledged as a real, larger memory commitment. REQ-904 is
recomputed a third time with every contributor itemized explicitly
(tape storage, the shipped track pool, the new buss reserve, and the
undo-transient cost) and rounded conservatively rather than claiming
false precision - ~1428MB steady-state, ~2.5GB worst-case peak, still
comfortable against the real Pi's ~5.8GB free. The peak-level test
finally asserts something a healthy signal couldn't produce by
accident. REQ-409 gets its REQ-602 carve-out and smoothing statement;
two more script ops (`Op::BounceFader`/`Op::BounceMute`) make REQ-408's
own test and the muted-buss behavior actually writable. `Entry.len`'s
per-channel meaning is pinned, `Journal::undo`/`redo`'s real
second-channel path is named, and the print-tap-point section states
explicitly where it sits relative to the hardware safety clamp added
earlier this session. Impact on tasks gains the `Chain`-splitting
builder, the metering flag as a real new mechanism rather than a free
extension, REQ-306's buss analogue, and two items left honestly
unresolved rather than papered over: latency accumulation across
fold-forward bounces, and REQ-905/M6's realtime CPU cost, both flagged
as needing a real decision or real measurement this document doesn't
have. Ready for a sixth spec-reviewer pass.
