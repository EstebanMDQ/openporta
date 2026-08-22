# 001: A dedicated stereo bounce buss, printed in real time

## Motivation

Requested directly by the owner while using the app, then reshaped three
times after review found real design holes (see "History" at the end -
this is now v4). The underlying problems are unchanged:

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
layering, at a real, non-trivial memory cost (REQ-904 below goes up by
roughly 58%, ~700MB to ~1103MB worst case). That trade is the owner's
to make, and it's being asked for directly here, not slipped in as a
footnote. It is not "no cost, all upside," and this document stops
framing it that way.

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
REQ-602 for tracks 1-4's contribution while feeding an active bounce
pass; the controls themselves stay non-destructively adjustable
afterward, same as after any record pass - moving a track's fader later
doesn't retroactively change what's already printed.

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

**Resolution, stated normatively (REQ-408):** while a bounce pass is
open, tracks 1-4's own contribution to the audible mix is replaced by
silence, and the buss's contribution is the pass's post-chain printed
signal - so what you hear is exactly the buss alone, which already
*is* tracks 1-4 plus the buss's own prior content, mixed. Not a double
count, and REQ-305's intent (hear what the tape receives) is honestly
satisfied - you hear precisely what's landing on the buss, nothing
added or hidden. Tracks 1-4 themselves are silent to listen to
individually during a bounce (their live monitor/arm state is
irrelevant here - REQ-405 already disallows arming them at the same
time anyway), which matches how you'd expect a "print the mix" pass to
sound: like the mix, not like the mix plus itself.

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
  contribution to the audible mix MUST be silent; the buss's
  contribution MUST be the pass's post-chain printed signal. Resolves
  the double-sum a naive REQ-305 reading produces for a self-inclusive
  pass - see "Monitoring" above.
- **REQ-301**: "recording MUST engage only on armed tracks" needs
  "...or the armed bounce buss" - a bounce records onto something that
  is not a track.
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
- **REQ-904**: revised - see "Impact on tasks."
- **Section 2 (Scope)**: "4 mono tracks, one stereo master output"
  becomes inaccurate once a 5th, always-mixed stereo buss exists, and
  the buss brushes against the explicit v1 out-of-scope line "variable
  track counts, track groups, scenes." It is not a track group - it has
  no arm-for-input capability, no pan, can't be one of the 4 - but that
  distinction needs to be stated in section 2 itself, not left for a
  reader to infer from the REQ list.

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
  never have the field.

### Session-script support (REQ-804)

Today's format has no way to express a v3/v4 bounce at all:
`Op::Record` requires a WAV input a bounce pass doesn't have, and there
is no op to arm the buss - meaning none of this proposal's new tests, or
the golden render, would have a headless driver without an addition
here. Two new ops, matching the shape of what's already there:

- `Op::BounceArm { on: bool }` - arms/disarms the buss (REQ-404/405).
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
- **Realtime-safe allocation - already landed, not just designed**:
  the REQ-902 gap v2/v3 flagged (`RecordPass::with_capacity`'s
  `reserve_exact`, sized to the whole remaining tape, running directly
  on the realtime thread since `Command::Record` isn't blocking) is
  fixed as of `record.rs`'s chunked-capture rewrite (see TASKS.md, M4.4's
  closed-out follow-up) - a real, pre-existing bug in ordinary track
  recording, shipped and tested independent of whether this proposal is
  ever accepted. Displaced audio is now captured in fixed-size chunks
  (`tape::CHUNK_SAMPLES`) drawn from a small pool `Journal` pre-reserves
  and replenishes at its existing off-thread touchpoint
  (`flush_pending`, run by Save/Undo/Redo) - not live-refilled during a
  session (a background-thread/wait-free-queue design was considered and
  explicitly deferred; asked directly). Extending this to the buss's two
  channels is a small, bounded addition to a mechanism that already
  exists: bump the pool's per-role budget (`undo::CHUNK_POOL_PER_TRACK`
  currently multiplies by `NUM_TRACKS`; the buss needs its own two
  shares alongside it, roughly +23MB pool capacity per share) rather
  than inventing anything new. `RecordPass::finish`'s separate small
  `.to_vec()` allocation (the punch-out fade tail) is also already fixed
  in the same change.
  - **Known, separate, pre-existing risk not addressed here**: the
    journal's `push_pass`/`evict` also run on the realtime thread today
    (reachable from `Stop`, which isn't blocking either) and grow plain
    `Vec`s (`undo: Vec<Entry>`, `pending_writes: Vec<(u64, Vec<Vec<i16>>)>`).
    That's a second, smaller realtime-allocation risk (small, pointer-
    sized container growth, not bulk sample data), already present for
    ordinary tracks, not made categorically worse by this proposal
    (still one push per pass, mono or the new multi-channel entry alike)
    - flagged for honesty, worth its own future task, not a blocker
    here.
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
  both causes.
- **New arm-like flag** for the bounce buss (REQ-404), plus the mutual-
  exclusion wiring with tracks 1-4's `armed` array (REQ-405).
- **`process_block`**: the buss becomes a fifth mix contributor (fader +
  mute, no pan) in both ordinary playback and while a bounce pass is
  running; its own read during a pass follows REQ-407's read-before-
  write ordering.
- **Journal**: `Entry`/`RecordPass` gain a multi-channel variant used
  only by the buss (ordinary tracks keep the existing single-channel
  shape) - see "Undo" above.
- **REQ-403's test, rewritten procedure**: the old three-successive-
  bounces procedure doesn't transfer - because the buss folds its own
  prior content forward every pass, re-bouncing *unchanged, unmuted*
  source material re-injects full-bandwidth signal each generation, so
  measuring HF energy/noise floor across generations 1-3 would measure
  "source material present or not," not generation loss. All three
  *measured* generations need identical input conditions: bounce once
  with tracks 1-4 unmuted (primes the buss), then mute tracks 1-4 and
  bounce three more times, measuring generations 2, 3, and 4 (buss
  re-printing only its own prior content each time) for the existing
  monotonic HF-loss/noise-floor-rise assertion. Scriptable via
  `Op::Bounce`, `Op::Arm{track,on:false}`x4, `Op::Bounce`x3.
- **New test**: stereo image survives a bounce and a second bounce (a
  hard-panned source stays audibly on its side, no collapse toward
  center).
- **New test**: a bounce pass's two channels share flutter (correlated
  wobble, not two independent LFOs) but decorrelated hiss (REQ-702).
- **New test, corrected from v3**: riding the master fader during a
  bounce does not change what's printed (REQ-406). v3's version asked
  for byte-identical output from two bounces of "identical material" -
  wrong, because `Engine::record()`'s per-track `pass_counter` gives
  every pass a different seed (REQ-304), so two bounces are never
  byte-identical regardless of the master fader. Corrected version:
  build the cassette with a passthrough character
  (`TapeCharacter`/`Chain::passthrough`, REQ-704) so no stochastic
  element is in play, bounce once at one master-fader position, Undo,
  set a different master-fader position, bounce again - assert the two
  printed regions are byte-identical.
- **New test**: peak level after several successive bounces of hot
  (0dBFS) material stays at or below the same ceiling any record pass
  already respects - assert post-bounce tape peak, read back via
  `Tape::read`, stays within the i16 full-scale clamp
  (`Dither::quantize`'s existing `clamp(-32768.0, 32767.0)`) after e.g.
  5 generations; no new clamping mechanism needed; this just proves the
  existing one still holds under self-inclusive summing.
- **New test**: while a bounce pass is open, tracks 1-4 read back as
  silent in the mix and the buss's contribution matches the pass's own
  post-chain signal (REQ-408) - no double-sum.
- **New test**: one Undo press after a bounce restores both channels
  atomically - no reachable state with one channel reverted and the
  other not (REQ-502/505).
- **New test**: a pass onto the buss's two channels, run back-to-back
  with ordinary track passes, doesn't exhaust the chunk pool budget in
  ordinary use - covered by extending the existing pool-budget math
  (see "Realtime-safe allocation" above), not a new mechanism to test in
  isolation.
- **REQ-904 (resident memory ceiling), recomputed against what actually
  shipped**: v3's "~1040MB ceiling, ~1.4GB transient peak" assumed lazy
  per-pass allocation and was inconsistent with its own REQ-902 fix (a
  real finding - see History). With the chunked-capture design that
  actually landed, there's no separate large transient spike to account
  for at all: tape storage is 691.2MB (4 mono tracks) + 345.6MB (1
  stereo buss, once added) = **1036.8MB**, plus the chunk pool - a
  small, *permanent* addition, not a transient one -
  `CHUNK_POOL_PER_TRACK`(24) x `CHUNK_SAMPLES`(240,000 samples) x 2
  bytes x 6 channel-shares (4 tracks + 2 buss channels) = ~66MB. Total
  worst case: **~1103MB steady-state, with no additional peak beyond
  that** - a pass exceeding its pool share falls back to individually
  small (~480KB) chunk allocations, not a whole-tape-sized burst, so
  there's nothing left to call out as a separate "peak" line. Checked
  against the actual deployment Pi (`patch@192.168.68.55`, confirmed via
  `free -h`: 8GB total, ~5.8GB free at idle with the desktop session and
  audio stack already running) - ~1103MB against ~5.8GB free at idle
  leaves well over 4.5GB of margin. The ceiling is revised from ~700MB
  to ~1103MB (default 15-minute cassette: ~520MB tape + ~66MB pool =
  ~586MB) rather than shortening max cassette length or the buss,
  because on the real target hardware there is no actual memory pressure
  to trade against. If this project ever targets a smaller-RAM Pi 4
  variant, this number needs revisiting again - noted here so it isn't
  forgotten.
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

**v4 (this revision)**: the REQ-902 fix is no longer a design on paper -
it shipped as its own commit (chunked pass capture, `record.rs`, see
TASKS.md), verified with its own tests, full gate, and a byte-identical
golden render, independent of whether this proposal is ever accepted.
REQ-904 is recomputed consistently with what actually landed. Monitoring
during a bounce gets its own real rule (REQ-408) instead of a wrong
"unchanged" claim. The REQ-406/403 tests are corrected and a real
peak-level test is specified. Two new session-script ops
(`Op::BounceArm`, `Op::Bounce`) resolve REQ-804 without inventing
mid-pass live automation as a scriptable primitive. The affected-
requirements list is completed (section 2, REQ-301, REQ-702, REQ-801/802,
REQ-503), and REQ-502's sizing consequence is stated and accepted
rather than solved. Ready for a fourth spec-reviewer pass.
