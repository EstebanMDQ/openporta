# 001: A dedicated stereo bounce buss, printed in real time

## Motivation

Requested directly by the owner while using the app, then reshaped twice
after review found real design holes (see "History" at the end - this is
now v3). The underlying problems are unchanged:

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
roughly 50%). That trade is the owner's to make, and it's being asked
for directly here, not slipped in as a footnote. It is not "no cost, all
upside," and this document stops framing it that way.

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

### Monitoring (REQ-305, explicitly unchanged)

A bounce pass is a record pass. REQ-305 already applies unchanged: what
you hear while bouncing is the post-chain signal being written to the
buss, the same as monitoring any other record pass - not some cleaner
pre-chain preview. Stated here explicitly because v2 left it implicit
and the reviewer flagged that as a gap, not because anything new is
being introduced.

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
- **REQ-502**: the undo journal's entry format MUST extend to cover a
  multi-channel (stereo) pass as a single atomic entry - see "Undo."
- **REQ-602**: gains the narrow bounce-pass carve-out described above;
  otherwise unchanged.
- **REQ-603**: no longer describes bounce at all; tracks 1-4's own
  REQ-601/602 behavior is untouched.
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

## Impact on tasks

- **Storage**: Tape gains a fifth (stereo) buffer, same fixed-length
  preallocation model as the existing 4 tracks (see REQ-904 below for
  the memory consequence) - no new storage *pattern*, just one more
  buffer of the same kind.
- **Realtime-safe allocation (resolves v2's REQ-902 flag, and a real
  pre-existing bug found while investigating it)**: `Engine::record()`
  today calls `RecordPass::with_capacity`, which calls
  `Vec::reserve_exact` for up to the tape's full remaining length -
  and `Command::Record` is not a blocking command, so this allocation
  already happens directly on the realtime audio thread for *ordinary*
  track recording, today, independent of bounce (confirmed by reading
  `porta-app/src/realtime.rs`'s callback, which calls `apply()` for any
  non-blocking command inline). This is a genuine REQ-902 violation that
  predates this proposal; bouncing would only make it worse (two
  channels instead of one). The fix, needed either way: the engine
  pre-reserves each track's (and the buss's two channels') `RecordPass`
  capacity once, at cassette open/create time (off the realtime thread
  by construction - that's a normal, blocking-context call), sized to
  the tape's full length; `record()` takes a pre-reserved buffer and
  clears it (no reallocation) instead of calling `reserve_exact` per
  call. This is an engine-internal correctness fix that keeps every
  requirement intact (REQ-902 is already settled; this makes the code
  honor it) - it does not itself need a separate proposal, but is called
  out here because the bounce buss depends on it existing first, and
  because it's worth landing and testing as its own step before the buss
  lands on top of it.
  - **Known, separate, pre-existing risk not addressed here**: the
    journal's `push_pass`/`evict` also run on the realtime thread today
    (reachable from `Stop`, which isn't blocking either) and grow plain
    `Vec`s (`undo: Vec<Entry>`, `pending_writes: Vec<(u64, Vec<i16>)>`).
    That's a second, smaller realtime-allocation risk, already present
    for ordinary tracks, not made categorically worse by this proposal
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
- **REQ-403's test**: needs a procedure that isolates what it measures.
  Because the buss folds its own prior content forward every pass,
  repeatedly bouncing unchanged, unmuted source material re-injects
  full-bandwidth signal each generation - the test needs to bounce, then
  mute tracks 1-4 for the next two generations, so what's re-printed is
  only the buss's own prior content aging against itself.
- **New test**: stereo image survives a bounce and a second bounce (a
  hard-panned source stays audibly on its side, no collapse toward
  center).
- **New test**: a bounce pass's two channels share flutter (correlated
  wobble), not two independent LFOs.
- **New test**: riding the master fader during a bounce does not change
  what's printed (REQ-406) - record two bounces of identical material at
  two different master-fader positions, assert byte-identical tape
  content.
- **New test**: one Undo press after a bounce restores both channels
  atomically - no reachable state with one channel reverted and the
  other not (REQ-502/505).
- **New test**: allocation-free `record()` on an already-open cassette
  (the REQ-902 fix above) - e.g. assert capacity was already sufficient
  going in, or an allocation-counting test if one is easy to add cheaply
  here; whichever is simplest to make genuinely load-bearing.
- **Gain staging**: self-inclusive summing over many bounces can
  accumulate level. The realtime output clamp added earlier this session
  (`mixer.rs`) already bounds what reaches hardware; tape writes go
  through the existing dither/quantize clamp to i16 the same way any
  record pass does - no new clamping mechanism needed. Worth a peak-
  level-after-several-bounces test, covered above.
- **REQ-904 (resident memory ceiling)**: revised from ~700MB to **~1040
  MB worst case** (30-minute cassette: 4 mono + 1 stereo buss = 6
  channel-equivalents x 172.8MB = 1036.8MB), with a documented **~1.4GB
  transient peak** during an active bounce pass (source + destination
  buffers briefly coexisting). This is a real increase, not "worth
  revisiting" hand-waving - resolved here, not left open: checked
  against the actual deployment Pi (`patch@192.168.68.55`, confirmed via
  `free -h`: 8GB total, ~5.8GB free at idle with the desktop session and
  audio stack already running). ~1.4GB peak against ~5.8GB free at idle
  leaves well over 4GB of margin - the ceiling is revised upward to
  ~1040MB (default 15-minute cassette: ~520MB) rather than shortening
  max cassette length or the buss, because on the real target hardware
  there is no actual memory pressure to trade against. If this project
  ever targets a smaller-RAM Pi 4 variant, this number needs revisiting
  again - noted here so it isn't forgotten.
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

**v3 (this revision)** addresses all of the above: a concrete
pre-allocation strategy for REQ-902 (which also fixes a genuine
pre-existing bug in ordinary track recording, found while investigating
this); REQ-904 resolved with a real number, checked against the actual
deployment Pi's real free memory over SSH rather than guessed; the print
tap point pinned to pre-master-fader with a stated reason; the self-
reference rule made normative (REQ-407); REQ-602's carve-out and
REQ-305's interaction stated explicitly; simultaneous-arm resolved as
mutually-exclusive (REQ-405), not left open; the golden/cli-test impact
folded into the existing regen note; and an explicit "this is not a pure
win" framing up front. Ready for a third spec-reviewer pass.
