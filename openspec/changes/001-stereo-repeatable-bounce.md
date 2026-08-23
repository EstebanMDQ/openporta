# 001: A dedicated stereo bounce bus, printed in real time

## Motivation

Requested directly by the owner while using the app, then reshaped
twelve times after review found real design holes (see "History" at the
end - this is now v13, the first revision after an approving review).
The underlying problems are unchanged:

1. **Stereo information is lost.** Today's bounce is a mono sum of
   tracks 1-3 onto track 4; anything panned comes out center.
2. **Bounce is one-shot.** A second bounce replaces track 4 with a fresh
   sum of 1-3, silently discarding the first submix.

**Approach, from the owner directly:** a fifth, dedicated, always-stereo
**bounce bus** - separate storage, not one of the 4 mono tracks - that
is always part of the mix but can only ever be *written* by bouncing.
Bouncing arms the bus, presses Record, and the transport rolls in real
time recording the current mix (tracks 1-4 at whatever fader/pan/mute
you're riding live, plus the bus's own existing content, since it's
already part of that mix) into the bus, replacing it as playback
proceeds.

### This is not a pure win - say so plainly

v1 and v2 of this proposal undersold the cost. A always-mixed 5th bus is
a real step away from "the constraint IS the product" (spec section 1):
on real hardware, every bounce costs you two tracks, permanently - that
scarcity is part of what a 4-track forces you to commit to. This design
trades that economics away in exchange for stereo imaging and repeatable
layering, at a real, non-trivial memory cost (REQ-904 below more than
doubles, ~700MB to ~1774MB steady-state, ~2.8GB worst-case peak while
undoing a bounce). That trade is the owner's to make, and it's being
asked for directly here, not slipped in as a footnote. It is not "no
cost, all upside," and this document stops framing it that way.

## Change

### Storage

Add a fifth storage area to Tape: one stereo (2-channel) i16 buffer, the
cassette's fixed length, alongside the existing 4 mono track buffers.
New tape storage, not a reuse of an existing track (see REQ-904).

### Mix

The bounce bus is always summed into the master output, at its own
fader level (muteable, not panned - it's already stereo) alongside
tracks 1-4, during ordinary playback as well as while bouncing.

### Bouncing (the "print" pass)

- A new arm-like state exists for the bounce bus, separate from the 4
  tracks' arm state (REQ-404).
- **Arming the bus and arming any of tracks 1-4 are mutually
  exclusive** (REQ-405, resolves the "not disallowed" gap v2 left open):
  arming the bus clears all 4 tracks' armed state, and arming any track
  clears the bus's armed state. No simultaneous case exists to reason
  about - a bounce pass never overlaps a live input pass on an ordinary
  track.
- With the bus armed, Record engages a real-time pass whose input is
  the current mix of tracks 1-4 (each at its own live fader/pan/mute)
  **plus the bus's own existing content at its own fader/mute**,
  computed **before** the master fader is applied (REQ-406) - see "Print
  tap point" below for why.
- The pass runs through the character chain like any record pass:
  wow/flutter shared between L and R (one modulation instance, not two
  independent ones); hiss may still be seeded independently per channel.
- Punch-in/out, the 5ms crossfade, and undo apply the same way they do
  to any record pass (see "Undo" below for the multi-channel case).
- Because the bus's own existing content is already part of what's
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
into anything written to tape, for tracks 1-4 or the bounce bus. The
bus's print input is the sum of tracks 1-4's own fader/pan/mute-scaled
signal plus the bus's own fader/mute-scaled existing content, computed
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
REQ-602 for tracks 1-4's contribution **and the bus's own fader/mute
(REQ-409)** while feeding an active bounce pass; the controls themselves
stay non-destructively adjustable afterward, same as after any record
pass - moving a track's fader, or the bus's, later doesn't
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

For each sample position a bounce pass writes, the bus's own
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

**`StereoFlutter`'s one modulator seed, pinned (a seventh review found
this undecided)**: `build_chain` seeds `Flutter` from the pass seed
directly and `Hiss` from `pass_seed ^ 0x5f5f_5f5f` - both single-channel
derivations with nothing to disambiguate for a stereo pass with two
per-channel seeds in play (REQ-702's `seed_for(noise_seed, pass,
channel)`). Since there's exactly one modulator, not one per channel,
it MUST use a single, fixed choice rather than leaving "which channel's
seed" as an implementation coin-flip REQ-702's bit-reproducibility
would otherwise depend on silently: **the modulator always seeds at
channel term 0** (the same convention REQ-702 already uses for "left"
elsewhere in this document), independent of which physical channel
that ends up correlating with - it's shared by construction, so both
channels see it identically regardless of which seed produced it.

### Monitoring during a bounce pass (REQ-408) - resolves v3's REQ-305 double-sum

v3 claimed REQ-305 applied unchanged during a bounce. **That was wrong**,
and a third review caught it: a bounce pass's input already contains
tracks 1-4 (that's the whole point - it's printing their sum). If
monitoring left tracks 1-4 sounding through the mix *as well as* the
bus now carrying their sum, you'd hear them twice - roughly +6dB,
comb-filtered against themselves by the character chain's flutter delay
on the bus's copy. REQ-305 ("the user hears what the tape receives")
doesn't actually resolve this on its own for a self-inclusive pass; it
needs its own rule.

**Resolution, stated normatively (REQ-408), corrected from v5 - the
previous version had the math backwards**: v5 said the printed signal
should reach the speakers directly, bypassing the bus's own fader, to
avoid double-scaling. A fifth review traced it and found that's
*exactly* the rule that produces the jump it was trying to prevent:
write `P` = tracks-1-4-sum + `bus_gain` x bus's-prior-content (REQ-406).
Immediately after the bounce, playing that region back is `P x
bus_gain` (the bus sums into the master at its own fader, same as
always). If monitoring played `P` directly during the pass, a -6dB bus
fader would sound 6dB louder *while bouncing* than the instant you let
go of Record - a real, audible jump, the opposite of transparent.

The actual fix needs no bypass and almost no new mechanism: **the bus's
`playback` slot holds the pass's post-chain printed signal in place of
its prior tape content, and flows through `Mixer::mix_block` exactly
the way it always does** - through the bus's own smoothed fader/mute,
same code path as ordinary playback. This is precisely how monitoring
an armed *track* mid-recording already works (`engine.rs`'s
`self.playback[t] = self.processed[t]` during a pass, REQ-305) - REQ-408
extends the identical, already-proven mechanism to the bus instead of
inventing a parallel one. **The bus's own contribution** stays
continuous across punch-out, because its gain is applied consistently
before, during, and after the pass - that's the specific, narrow claim
this fix makes, not "nothing about the output changes at punch-out."

Tracks 1-4's own contribution to the mix still needs to go silent
during a bounce (unchanged from v5) - otherwise you'd hear them once
directly and again inside the bus's printed copy. **This means the
*overall* output does jump at punch-out** - by exactly tracks 1-4's own
contribution, which reappears the instant they un-silence. That's not a
bug to hide (a sixth review pointed out the previous wording implied no
jump at all, anywhere, which isn't true): it's the same thing real
hardware does when you stop feeding a bus and start monitoring the
result instead - you mute the sources after a bounce, same idea. State
it, don't paper over it: **the bus's own audible contribution is
continuous through punch-out; tracks 1-4 re-appearing is not.**

**The claim, worked out explicitly (a seventh review read an earlier,
looser version of this paragraph as claiming something that doesn't
hold, and re-deriving it precisely is the fix, not a change in
mechanism; an eighth review then found the re-derivation itself glossed
over dither, corrected below)**: let `P(t)` be the print input at tape
position `t` (REQ-406), `W(t) = Chain(P)(t)` the post-chain,
pre-quantize signal, and `g` the bus's own gain (held constant across
the comparison - riding it is REQ-406's concern, not this one). *During*
the pass, at the moment position `t` is being written, the monitor
output is `g x W(t)` - the bus's `playback` slot is `W(t)` itself (the
same pre-dither value `RecordPass::write_block` is about to dither and
quantize, reused rather than recomputed), scaled by `g` through the
ordinary `mix_block` path. *After* the pass closes, playing back that
same, now-written position `t` reads what's actually on tape -
`quantize(dither(W(t)))`, not `W(t)` - and scales *that* by `g`:
`g x quantize(dither(W(t)))`. **These are not bit-identical**; they
differ by up to +/-1.5 LSB combined (TPDF dither spans +/-1 LSB - it's
the *difference* of two independent uniform draws, `Dither::quantize`'s
`(r - self.prev)`, which is a triangular distribution on [-1, +1] LSB,
not +/-0.5 as an earlier version of this paragraph said; `.round()`
adds up to +/-0.5 LSB more on top) - RMS error is ~0.5 LSB (variance
adds: TPDF's 1/6 LSB^2 plus rounding's 1/12 LSB^2 = 1/4 LSB^2, so RMS is
its square root), roughly **-96dBFS**, not the -90dBFS a peak single-LSB
value would be (a different quantity, and a mistake this document itself
made once already, corrected below and everywhere else this number
appears). Inaudible either way, but a real difference this document was
wrong to call "identical" for the seventh-review version above. This is not a
new gap the bus introduces: it's the exact same approximation ordinary
track monitoring already makes today (a ninth review corrected the
ordering claim here - `engine.rs:343`'s `pass.write_block(...)` dithers
and quantizes into tape from an immutable borrow of `self.processed[t]`,
and `engine.rs:347`'s `self.playback[t] = self.processed[t]` copies that
same *pre-dither* buffer afterward, not before - either way, the copy
never sees the dithered value, so REQ-305's own "the user hears what
the tape receives" has always meant "receives, before dither" for
tracks 1-4). REQ-408 inherits that same precision level rather than
inventing a stricter or looser one - stated here explicitly so the
"bit-identical" claim isn't left overstated. The
comparison that matters is "monitored live vs. replayed after, within
dither's noise floor," at the *same* tape position, not "during the
pass vs. what that position would have sounded like un-bounced" (a
different, uninteresting comparison this claim was never about).

**The real caveat is REQ-302's tape-side crossfade, at both boundaries,
not `g`'s own ramp (a tenth review found the previous version of this
paragraph named the wrong mechanism, and missed the bigger one
entirely)**: `RecordPass::write_block` (`record.rs`) blends the first
`XFADE_SAMPLES` of what's actually *written to tape* between the
displaced (old) content and the new dithered value, `pass_idx <
XFADE_SAMPLES` in its own code - the bus's monitor slot, `W(t)`, holds
the *un-faded* new value throughout, since REQ-408 never says to fade
it. So for those first `XFADE_SAMPLES`, live-monitored and
replayed-after diverge by up to the full old-vs-new difference, not by
dither noise - a real, much larger gap than the one this paragraph
previously named. Symmetrically, `RecordPass::finish` *retroactively*
rewrites the last `min(XFADE_SAMPLES, pass_len)` already written,
blending them back toward the displaced content, **after** those same
positions were already monitored live at their un-faded values during
the pass - replaying them afterward reads the retroactively-blended
tape, not what was heard live. Not unconditional, though (an eleventh
review caught this stated too broadly): `finish` skips the out-fade
entirely when the pass ran to the very end of the tape (`end >=
tape.len_samples()` returns early - there's nothing beyond the end to
blend back into), so a bounce that runs out the tape has a punch-in
boundary but no punch-out one. Both boundaries, where they exist, are
the same, already-accepted mechanism REQ-302 already requires for every
punch in this engine (nothing new is being introduced here) - and worth
one clarifying clause: REQ-302's crossfade lives on *tape content*
only, at both ends; the monitor slot is never faded, for the bus here
exactly as for an ordinary armed track's `playback[t] = processed[t]`
today. REQ-408's own claim has to name these mechanisms explicitly
rather than gesture at a smaller, unrelated smoothing effect:
**"monitored live matches replayed-after, within dither's noise floor"
holds only for the stretch of a bounce pass clear of both its punch-in
and punch-out crossfade windows.** The ramp on `g` needs no separate
caveat once REQ-302's is stated - though not because it settles "well
within" the opening window: `SMOOTH_SAMPLES` (`SAMPLE_RATE / 200`) and
`XFADE_SAMPLES` (`SAMPLE_RATE * 5 / 1000`) are both exactly 240 samples,
5ms, coincident by construction with zero margin (an eleventh review
caught the earlier wording overstating this). If either constant ever
moves independently, this sentence is the one that becomes false first -
flagged so the coupling is visible rather than accidental.

**Tick-once, not twice, per sample (a sixth review caught this)**: the
bus's smoothed gain is needed at two points for one sample - once
folded into the pre-chain print input (REQ-406), once again for the
post-chain monitor output above, and the chain runs *between* those two
uses within the same sample's processing. `Smoothed`'s ramp advances
one step per `tick()` call; calling it twice per sample would double
the ramp's effective rate and make the result depend on how many times
it happened to be read, which also breaks REQ-203's block-size
invariance. The implementation MUST tick the bus's smoothed gain once
per sample position and reuse that one value for both uses, not
`tick()` it fresh at each use site.

**Metering is not silenced (a second, separate clause of REQ-408):**
tracks 1-4's own individual meters MUST keep reflecting their own
playback contribution during a bounce pass (a tenth review found "live
signal" here contradicts REQ-405 - no track can be armed during a
bounce, so there is no live input; what continues is playback at each
track's current fader/mute), independent of their audible contribution
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

**Bouncing with the bus muted is destructive, on purpose, not a bug**:
per REQ-406, the print input includes the bus's *own* existing content
"at its own fader/mute" - so a bounce with the bus muted excludes the
bus's prior content from what gets printed, replacing rather than
folding it forward. Stated here explicitly (v4 left it implicit and a
reviewer flagged it as an accident waiting to happen) because it's the
mute control doing exactly what mute does, the same as muting a track
before recording over it - not a special case to design around. A
consequence worth naming rather than leaving to be discovered (a tenth
review pointed it out): bouncing with the bus muted while tracks 1-4
are excluded from the monitor sum (REQ-408, unconditional during any
open bounce, mute or not) leaves **nothing** audible for the duration -
tracks are silent by REQ-408, the bus's own gain is zero by the mute
the user just set. This directly negates the feature's stated purpose
("play with levels and panning while it bounces") for exactly this one
combination, but it follows from mute doing what mute does, same as the
tape-side consequence above - not a design gap, just worth knowing
about before it's found by surprise.

### What doesn't change

- Tracks 1-4 stay exactly as they are: 4 mono, armable, recordable, with
  fader/pan/mute/monitor. REQ-601-602 apply to them unchanged outside
  the narrow bounce-pass carve-out above. REQ-603 no longer describes
  bounce (it never sums tracks through pan anymore in any form).
- Export/WAV mixdown: unaffected in shape - the bus just becomes one
  more thing already folded into the post-master output when present.

## Requirements affected (settled decisions being reversed or extended)

- **Definitions** (section 3): "Tape" becomes "4 fixed-length mono i16
  buffers plus one fixed-length stereo i16 buffer (the bounce bus), all
  at 48kHz." "Bounce" becomes "a real-time record pass onto the
  dedicated stereo bounce bus, whose input is the pre-master-fader sum
  of tracks 1-4 (at their live fader/pan/mute) plus the bus's own
  existing content (at its own fader/mute)." "Record pass" gains a
  clause: a pass onto the bus writes both channels atomically as one
  pass for undo purposes (see REQ-502 below) - still "one continuous
  record engagement," now on a bus instead of a track.
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
- **REQ-404 (new)**: the bounce bus MUST have its own arm-like flag,
  independent of tracks 1-4's `armed` array, with no ordinary-input
  recording capability.
- **REQ-405 (new)**: arming the bounce bus and arming any of tracks 1-4
  MUST be mutually exclusive; arming one MUST clear the other. A direct
  consequence, stated so it isn't rediscovered as a surprise later
  (input-monitor preview requires `armed[t]`, `engine.rs`): no track's
  live input can be monitored at all while a bounce pass is open, since
  none can be armed. Intended, not an oversight - a bounce pass is
  about the bus's own printed signal, not a live source.
- **REQ-406 (new)**: the master fader MUST NOT be baked into any signal
  written to tape (tracks 1-4 or the bounce bus); a bounce pass's input
  MUST be computed before any master-fader multiplication.
- **REQ-407 (new)**: a bounce pass's own prior content at a given tape
  position MUST be read before the pass's new value is written to that
  position (block-local read-before-write; no lookahead).
- **REQ-408 (new), corrected - a sixth review found this bullet still
  had v5's inverted rule even after the narrative section was fixed**:
  while a bounce pass is open, tracks 1-4's own contribution to the
  audible output MUST be silent; the bus's contribution MUST be the
  pass's post-chain printed signal flowing through the bus's own
  smoothed fader/mute, the same `Mixer::mix_block` path ordinary
  playback always uses (**not** output directly/bypassing that path -
  v5's rule, which produces a punch-out discontinuity, per "Monitoring"
  below). The bus's smoothed gain value MUST be computed once per
  sample and reused for both its contribution to the print input
  (REQ-406) and its contribution to the monitor output at that same
  sample position - never advanced twice for one sample (see
  "Monitoring" below for why that matters). Track-level metering MUST
  NOT be silenced by this - it keeps reflecting each track's own
  playback contribution, post-fader and pre-pan (a tenth review pointed
  out `Mixer`'s meter, `peak * fader_amp`, has never included pan -
  correcting "fader/pan/mute" from an earlier version, which overclaimed
  what the existing mechanism actually measures; a ninth review pointed
  out "live signal" contradicts REQ-405 two bullets up - no track can be
  armed during a bounce, so there is no live input to reflect, only
  playback). Resolves the
  double-sum a naive REQ-305 reading produces for
  a self-inclusive pass, and the dead-meters gap a fifth review caught
  in v5's version - see "Monitoring" below.
- **REQ-409 (new)**: the bounce bus MUST have its own volume fader and
  mute (REQ-406/408 both depend on "the bus's own fader/mute" already
  existing), independent of tracks 1-4's (REQ-601) - no pan, since it's
  already stereo. No requirement currently establishes this; REQ-601 is
  track-scoped. Both MUST be smoothed the same 5ms way every other
  mixer control already is (`mixer.rs`'s existing ramp) - it matters
  more here than for an ordinary track, since these values get printed
  to tape, not just heard. REQ-406's carve-out to REQ-602 (tracks 1-4's
  fader/pan baked in during a bounce) extends to the bus's own
  fader/mute too - it's baked into the print the same way, for the same
  reason (a reviewer pointed out the carve-out as originally worded only
  named tracks 1-4).
- **REQ-301**: "recording MUST engage only on armed tracks" needs
  "...or the armed bounce bus" - a bounce records onto something that
  is not a track.
- **REQ-306**: "unarmed tracks MUST be byte-identical before/after any
  record pass" gains a bus-shaped analogue for free, worth one clause
  rather than leaving it implicit - the bus MUST be byte-identical
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
- **REQ-603**: deleted outright, not reworded - it described exactly one
  thing ("during bounce, pans MUST be ignored, matching the reference
  hardware's bus behavior"), and that thing no longer exists in this
  design (bounce isn't a fader/pan-driven sum of tracks through the
  ordinary mixer anymore; tracks 1-4 keep their own real pan always,
  including while feeding a bounce pass, per REQ-406). Tracks 1-4's own
  REQ-601/602 behavior is otherwise untouched.
- **REQ-702**: "hiss... independently per channel" needs a decision, not
  a MAY - see "Persistence and reproducibility" below.
- **REQ-801/802**: the bus needs its own on-disk storage and dirty-
  chunk tracking, and `Project::open`/`load_tape` need a path for
  cassettes saved before this feature existed - see "Persistence" below.
- **REQ-804**: session scripts (REQ-804) can't currently express a
  bounce at all under this design - `Op::Record` requires a WAV input a
  bounce pass doesn't have, and there's no op to arm the bus. See
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
  stereo master output, plus one fixed, mix-only stereo bounce bus (not
  a 5th track: no arm for live input, no pan, exists only to receive a
  printed mix, cannot be added to or removed - REQ-101/404)."
  "destructive bounce" becomes "destructive real-time bounce onto the
  bus." This addresses the "track group" ambiguity a reviewer flagged
  directly, in the same document that moves scope, rather than leaving
  it for a reader to infer from the REQ list.
- **`Command::Bounce` removal**: this design deletes the old blocking
  batch command entirely (`command.rs`'s `Command::Bounce` variant and
  its `is_blocking()` match arm, `Engine::bounce()`, and the
  `disk_touching_commands_are_marked_blocking` test's assertion about
  it) in favor of arm-the-bus + ordinary Record - stated in "What
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

A bounce pass writes two channels of one bus, not one track. To keep
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
  **Dither gets the same channel term, decided here rather than left
  unstated (a sixth review pointed out `RecordPass`'s dither is seeded
  independently of the character chain and this document has been
  bitten by dither-seeding assumptions before)**: a stereo pass uses two
  `Dither` instances, `seed_for(noise_seed, pass_id, channel)` for each,
  same derivation as hiss. Dither error's RMS is ~0.5 LSB (~-96dBFS -
  see "Monitoring" above for the derivation) either way, so this is
  inaudible - decided for determinism's sake (REQ-702's
  bit-reproducibility guarantee), not because correlated dither would be
  a real problem.
- **REQ-801/802 (bus storage)**: the bus's audio lives in its own two
  raw i16 files (`tape/bounce_l.raw`, `tape/bounce_r.raw`), written in
  the same 5-second dirty-chunk pattern as tracks 1-4
  (`project.rs`/`tape::CHUNK_SAMPLES`) - not a new storage pattern, two
  more files of the existing kind. `Project::open`/`load_tape` treat
  missing bus files as "never bounced yet" (all-zero, matching how a
  fresh cassette's tracks already start) rather than an error, so every
  cassette saved before this feature exists opens unchanged.
  **REQ-409's fader and mute persist in the manifest (a twelfth review
  found this missing entirely - without it, save-and-reopen silently
  resets the bus to unity/unmuted, and by this document's own analysis
  the bus's mute is destructively load-bearing on the very next bounce,
  with no test to catch a field that was never added)**: `Manifest`
  gains `bounce_fader_db: f32` and `bounce_muted: bool`, both
  `#[serde(default)]` (unity / unmuted for every pre-existing cassette -
  the same additive-field precedent `Manifest::muted` already set), and
  `apply_to`/`capture_from` carry them in and out exactly as they do the
  per-track `fader_db`/`muted`. A mix decision, persisted like fader/pan
  - `project.rs`'s own words for why `muted` persists apply verbatim.
- **REQ-503 (journal format stays backward compatible)**: `Entry` gains
  one additive field, `right_track: Option<usize>` (`#[serde(default)]`,
  matching the precedent already used for `Manifest::muted`) - `None`
  for every existing single-channel entry (unchanged meaning), `Some(r)`
  only for a bounce entry, whose `track` field holds the bus's left
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

  **On-disk layout, pinned (a sixth review found this undefined too,
  wording corrected again - the bus reserve is one buffer per channel,
  not chunks, a seventh review pointed out "chunks" was the wrong noun
  here)**: one file per entry `id`, same as today (`path_for(id)`) -
  not two files, and not interleaved samples. The left channel's bytes
  are written first, then the right channel's, back to back; `len`
  (already pinned as per-channel) tells `read_payload`'s caller exactly
  where the split falls when reading it back for a stereo entry. This
  keeps `path_for`/the file-per-id model unchanged; only the caller
  needs to know an entry is stereo and read/split accordingly.

  **The give-back routing needs an explicit tag, not an inferred one (a
  seventh review pointed out `pending_writes`'s current shape,
  `Vec<(u64, usize, Vec<Vec<i16>>)>`, has nowhere to record which
  mechanism a payload belongs to)**: `Journal`'s per-track give-back
  array (`chunk_pool: [Vec<Vec<i16>>; NUM_TRACKS]`) and the bus's
  double-buffered reserve are two genuinely separate mechanisms, not
  one array both share, indexed by a "virtual track number." **Two
  different failure modes, for two different structures - an eighth
  review conflated them, a ninth split them back apart**:
  `chunk_pool` genuinely is the fixed-size `[Vec<Vec<i16>>; NUM_TRACKS]`
  its type says - a virtual index of `NUM_TRACKS` there panics loudly
  and immediately, the first time the bus's give-back is exercised, so
  routing it there by accident fails safely. `Tape.tracks`, separately,
  is a `Vec<Track>` (constructed with exactly `NUM_TRACKS` elements, but
  a growable `Vec`, not a fixed array) - appending a 5th/6th slot to it
  for the bus's own storage wouldn't panic at all, and every existing
  loop that walks tracks by `0..NUM_TRACKS` (the constant, not
  `tracks.len()`) - most of the codebase - would simply never iterate
  that far, so the bus's slot would sit there, correctly written,
  silently un-read by anything, with nothing failing loudly to catch it.
  Both are real reasons the bus needs its own explicit identity rather
  than an appended/overloaded index, just different reasons at different
  sites: `pending_writes`'s entries need
  an explicit tag alongside the id (e.g. `Track(usize)` vs `Bus`, not a
  bare `usize` overloaded to sometimes mean "index past the real
  tracks") so `push_pass`/`evict`/`flush_pending` can route each
  payload's give-back
  to the right mechanism without guessing from the number alone. Same
  tag resolves how `Tape` itself addresses the bus's storage: it needs
  its own field (or a small enum alongside `tracks`), not an appended
  element indexed by anything `0..NUM_TRACKS` code would silently skip.

### Session-script support (REQ-804)

Today's format has no way to express a v3/v4 bounce at all:
`Op::Record` requires a WAV input a bounce pass doesn't have, and there
is no op to arm the bus - meaning none of this proposal's new tests, or
the golden render, would have a headless driver without an addition
here. Four new ops, matching the shape of what's already there:

- `Op::Mute { track: usize, on: bool }` - the engine already has
  `Command::Mute`; the script format never needed it before because no
  test cared about a muted track's exact contribution. REQ-403's
  rewritten procedure (below) does.
- `Op::BounceArm { on: bool }` - arms/disarms the bus (REQ-404/405).
- `Op::BounceFader { db: f32 }` / `Op::BounceMute { on: bool }` -
  REQ-409's bus fader/mute, existing today only as engine-internal
  state with no track index to attach to (`Op::Fader`/`Op::Mute` are
  both range-checked against `NUM_TRACKS`, and the bus isn't one of
  them). Without these, REQ-408's own test (distinguishing "the bus
  fader applies once, consistently" from "applied twice" needs a
  non-unity bus fader to even observe a difference) and "bouncing with
  the bus muted is destructive" are both unwritable - a fifth review
  caught that the two mute/arm ops alone don't cover this.
- `Op::Bounce { seconds: f32 }` - requires the bus already armed
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
  container to build, no allocation at either end. `RecordPass::finish`'s
  `.to_vec()` allocation and `push_pass`'s `format!`/`PathBuf` filename
  computation (a second and third realtime-thread allocation the same
  review caught) are also already fixed in the same commits -
  `Entry.file` doesn't exist anymore; the filename is always derived
  from `id`. This mechanism (many small chunks, per-track) is a good fit
  for tracks; it is **not** extended to the bus - see below.
  - **The bus does NOT extend this mechanism - a fifth review found
    that plan doesn't work, and it's right**: `CHUNK_POOL_PER_TRACK` (24
    chunks, ~2 minutes) is sized for an ordinary take. A bounce is not
    an ordinary take - by definition it's close to the full remaining
    tape, every time. A 3-minute bounce alone needs 36 chunks per
    channel with nothing flushing in between; a 15-minute one needs 180.
    Extending the *same* small reserve to the bus means the "rare
    fallback" path v5 described is actually the *common* case for this
    specific operation - which defeats the point. v6's first attempt at
    a fix (one dedicated full-tape-length buffer per channel) traded
    this for a different, structurally identical bug, caught by a sixth
    review: a bounce pass either uses the *whole* reserve or doesn't -
    there's no partial-use remainder to give back immediately the way a
    track pass gives back its unused chunks, so the entire buffer moves
    into `pending_writes` on close and doesn't return until the next
    flush (`Save`/`Undo`/`Redo` - `Stop` deliberately does not flush,
    see `stop_does_not_write_the_journal_payload_until_save`). Bounce
    twice in a row with nothing saved in between - not an edge case,
    it's problem #2 in this proposal's own Motivation - and the second
    bounce allocates a full-tape buffer on the realtime thread.

    **Resolution: double-buffer the reserve - two full-tape-length
    buffers per channel, not one.** Standard practice for exactly this
    shape of problem (a producer needs a fresh buffer while the
    previous one is still draining downstream). Bounce 1 takes buffer
    A; bounce 2, started before anything has flushed, takes buffer B -
    available immediately, independent of A's state, no allocation.
    By the time a third bounce might want a buffer with nothing saved
    in between across *all three*, that's a genuinely rare pattern this
    proposal accepts falling back for, same as any track pass exceeding
    its own reserve - rare, documented, counted, not silently wrong.
    Both buffers are allocated once, off the realtime thread, at
    cassette open/create - the same moment ordinary `Tape` storage
    itself is allocated - and handed out/reclaimed via the same
    `mem::take` pattern already proven for tracks, just as a pair
    instead of a pool. This roughly doubles the bus reserve's own
    memory cost (see REQ-904 below, recomputed to include it honestly)
    - a real, larger commitment, stated plainly rather than understated
    a second time.
  - **A fourth realtime-thread bug, also already fixed**: a seventh
    review traced `Journal::evict()` and `push_pass`'s own redo-branch
    invalidation and found both used `Vec::retain` to drop a still-
    pending (never flushed) entry's chunk buffers outright - a bulk
    *deallocation* on the realtime thread (reachable from `Stop`, which
    isn't blocking either), and worse, a silent, permanent leak from the
    reserve every time it happened, since the chunks never came back.
    Fixed the same day: both call sites now go through one shared
    `release_entry_payload` - a still-pending payload's chunks return to
    the track's reserve (the same plain move `flush_pending` already
    uses once chunks are actually written), an already-flushed one still
    queues for file deletion. New regression test: evicting a still-
    pending entry recovers its chunk rather than losing it.
    **That first fix had its own residual gap, found by an eighth
    review and also already fixed**: `reclaim_chunks` (the function
    `release_entry_payload` and `flush_pending` both funnel through)
    still dropped a chunk in place whenever it didn't fit back into the
    track's reserve - fine when the caller is `flush_pending` (already
    off the realtime thread), a real deallocation-on-thread when the
    caller is `release_entry_payload` via `evict`/`push_pass`'s redo
    invalidation. Reachable: an entry whose pass ran past its reserve
    and fell back to ordinary allocation for the overflow (see
    `CHUNK_POOL_PER_TRACK`'s own doc comment - this is documented,
    accepted behavior for the *allocation*, not for what happens to
    those chunks later), evicted or redo-invalidated while still
    pending. Fixed by adding `Journal.pending_frees` - whatever
    `reclaim_chunks` can't return to a reserve is parked there instead
    of dropped, and `flush_pending` (the one call site guaranteed to be
    off the realtime thread) is where the actual deallocation happens,
    at the end of its own loop. New regression test: reclaiming more
    chunks than a reserve holds parks the excess rather than dropping it
    immediately, and `flush_pending` is what clears it.
    The remaining, smaller container-growth risk in these same functions
    (`Vec<Entry>`/`Vec<(u64, usize, Vec<Vec<i16>>)>`/`pending_frees`
    growing their own outer capacity) is unaddressed and accepted, same
    reasoning as before: small, pointer-sized, not bulk sample data, not
    made categorically worse by this proposal.
  - **A fifth realtime-thread allocation, pre-existing and independent of
    this proposal entirely - found by a ninth review auditing this
    section for completeness, and also already fixed**: `Engine::record()`
    runs on the realtime audio thread (its own comment says so) and was
    doing `self.chains[t] = self.project.manifest.character.build_chain(seed)`
    every time recording engaged - 4-5 `Box::new` heap allocations per
    track. This has been shipping since before this proposal existed;
    the bus would only have made it worse (two chains per bounce pass
    instead of one per track, per "Chain-splitting in porta-dsp" below).
    Fixed the same way the other two were: `AudioProcessor` gained a
    `reseed(&mut self, seed: u32)` method (default no-op-beyond-`reset`;
    only `Hiss` and `Flutter` override it, the only stages with their own
    seeded state), `Chain` gained `reseed_stage(index, seed)`, and
    `TapeCharacter::reseed_chain` mirrors `build_chain`'s exact per-stage
    seed derivation without allocating. `Engine` now builds each track's
    chain for real once, off the realtime thread, at cassette open/create,
    and `record()` reseeds it in place. New regression test:
    `reseed_chain_matches_a_freshly_built_one` asserts a reused, reseeded
    chain's output is identical to a fresh `build_chain` with the same
    seed. This closes the gap the allocator-counting harness below would
    otherwise have caught the moment it was written - worth noting since
    that harness's whole point is to stop this exact kind of thing from
    hiding.
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
  per-sample gain before summing). **Not** a bare scalar multiply over
  `out_l`/`out_r` for the audible/export path, as an earlier version of
  this document said - a sixth review pointed out that would drop
  master-fader moves out of the smoothed ramp `target()` currently
  folds them into, so a master move would click, a real REQ-602
  regression nothing in the existing test suite would catch
  (`fader_jump_does_not_click` only exercises track faders). The master
  factor needs its own `Smoothed` instance, ticked once per sample and
  applied as the final multiply - same click-free guarantee as today,
  just computed as a separate step instead of folded into each track's
  own ramp. Identical to today's output in steady state once the
  smoothing is preserved - a seventh review pointed out "mathematically
  identical" overclaimed this: today, one ramp carries the *product* of
  a track's fader and the master; splitting them into two independent
  ramps only computes the same product when at most one of the two is
  actually moving at a time; riding both together, the product of two
  live ramps isn't identical to one ramp on their product mid-move.
  Floating-point multiply isn't strictly associative either way, so
  this reordering can still perturb the existing golden render at the
  bit level even in the steady-state case. That's on top of the
  already-known golden-regen need from removing
  `{"op":"bounce"}` (`tests/golden.rs`, `tests/cli.rs:208`) - one
  regeneration event, one TASKS.md note, one notification, covering
  both causes.
  - **This is two distinct pre-master sums, not one, and they run either
    side of the character chain - an eighth review found the single-sum
    version self-contradictory (REQ-406 needs tracks 1-4's full
    contribution, REQ-408 needs it excluded, during the same pass); a
    ninth found the eighth's fix put both sums inside one `mix_block`
    call despite that, which silently requires entering `mix_block`
    twice per block and double-ticks every track's ramp - the exact bug
    this document already diagnosed for the bus's own gain, one level
    up.** The actual fix needs one more fact this document had lost
    track of: **the character chain does not run inside `mix_block` at
    all, for tracks 1-4 or the bus.** It runs in `Engine::process_block`'s
    existing per-track loop, *before* `mix_block` is ever called for
    that block (`self.chains[t].process(...)` at `engine.rs:341`, then
    `mix_block` once at `engine.rs:366`, on the now-finished
    `self.playback` array). `mix_block` today is purely a summing/gain
    stage over already-chain-processed material - it was never the
    place doing the thing the eighth-review fix assumed it was doing.
    Once that's clear, the fix is a genuine two-phase split of that
    summing/gain stage, run once each per block, no re-entry and no
    double tick:
    - **Phase 1, `Mixer::sum_tracks`** (new, replaces the track-summing
      half of today's `mix_block`): ticks each of tracks 1-4's
      fader/pan `Smoothed` ramps exactly once per sample - same as
      today, just factored out - and, in that same per-sample pass,
      accumulates *two* running sums from the identical scaled value:
      one gated by the exclude flag (the *monitor* sum - zero for an
      excluded track, full weight otherwise) and one never gated (the
      *print* sum - always full weight). Also updates each track's
      meter peak from the same `input * fader_amp` value already
      computed, exactly as today - unaffected by either sum's masking.
      Outside a bounce, the exclude flag is all-false and the print sum
      is simply never consulted; the extra accumulator costs one more
      multiply-add per sample, not a second pass over the data.
      **Storage, named explicitly (a tenth review found this bullet
      specified the computation but not where its output lives; an
      eleventh found the bus's `playback` slot - referenced throughout
      this design - was itself never in the allocation inventory
      either, the same omission class one step over)**: the bus needs
      two MAX_BLOCK-sized L/R buffer pairs, both owned by `Engine`,
      both allocated once, off the realtime thread, at cassette
      open/create, exactly mirroring the `processed`/`playback` pair
      every ordinary track already has (which are `Vec`s sized to
      MAX_BLOCK at construction, not fixed arrays - an eleventh review
      caught this document asserting a `[f32; MAX_BLOCK]` type the
      codebase doesn't use; "a MAX_BLOCK-sized buffer allocated once at
      open/create" is the actual claim). The first pair is the **print
      buffers** - the bus's equivalent of a track's `processed`:
      `sum_tracks` writes the print sum into them, the between-phases
      step adds the bus's prior-content term and runs the character
      chain over them in place (`AudioProcessor::process` is in-place),
      leaving `W(t)` in the same buffers. The second pair is the
      **bus's `playback` slot** - the bus's equivalent of a track's
      `playback`: during a pass it receives a copy of `W(t)` from the
      print buffers (the same `playback = processed` copy tracks
      already do); when no bounce is open it instead holds the ordinary
      tape readback of the bus's stored content. `finish_mix` only
      ever reads the playback pair, never the print pair - which is
      what keeps phase 2 identical in shape whether or not a bounce is
      open.
    - **Between phase 1 and phase 2, for the bus, when a bounce pass is
      open (not `mix_block`'s concern at all)**: `Engine::process_block`
      adds the
      bus's own prior content at this position - a tape-read, REQ-407's
      read-before-write, the same mechanism `RecordPass` already uses to
      capture displaced content for undo, **not** anything from
      `Mixer` - scaled by the bus's own smoothed gain (ticked here,
      once, into the per-block scratch buffer described further below),
      to phase 1's print sum, giving the full `P(t)`. That feeds the
      bus's own character chain (`StereoFlutter` and the rest, run the
      same way tracks 1-4's chains already run, just for the bus
      instead of a track), producing `W(t)`, which is what gets written
      to tape and copied into the bus's own `playback` slot (REQ-408) -
      all of this happens in `process_block`, after the ordinary
      per-track loop and before phase 2, exactly parallel to how an
      ordinary track's own chain-then-write already happens inside that
      per-track loop today. When no bounce is open, this step is skipped
      entirely and the bus's `playback` slot instead holds an ordinary
      tape-readback of its own stored content, the same way an idle
      track's does.
    - **Phase 2, `Mixer::finish_mix`** (new, the other half of today's
      `mix_block`): takes phase 1's already-computed monitor sum and
      adds the bus's own contribution - its `playback` slot (whatever
      phase between 1 and 2 left there) scaled by its own smoothed gain,
      reusing the exact same per-block scratch-buffer value phase-
      between-1-and-2 already ticked if a bounce is open this block, or
      ticking it fresh (once) if not - then applies the master `Smoothed`
      ramp (ticked here, once, same as today's single-pass version) and
      clamps. This is the only place `out_l`/`out_r` get written.
    - Net effect: every ramp this proposal touches - each track's
      fader/pan, the bus's own gain, the master - is ticked exactly
      once per sample per block, regardless of whether a bounce is open,
      because each one has exactly one owner (phase 1 for tracks, the
      between-phases step *or* phase 2 for the bus depending on which
      one runs this block, phase 2 for master) and that owner never
      changes mid-block. Outside a bounce, phases 1 and 2 run back to
      back with nothing new between them - the same single logical pass
      as today's `mix_block`, just expressed as two functions instead of
      one, which a caller with no bus (or this proposal's tests run
      against a plain track scenario) can't tell apart from today's
      behavior.
    - Stated plainly since it's easy to lose in the mechanism: **the
      "excluded from the sum, still metered" flag applies to the
      monitor sum only.** The print sum's whole purpose is to *not*
      exclude anything - a track marked excluded is still summed there
      at full weight. Metering is unaffected by either sum's masking -
      it comes from `input * fader_amp`, computed once in phase 1
      regardless of which sum(s) that value feeds.
    - **Is the exclude flag's own toggle a click (a ninth review's
      medium finding, resolved here rather than left open)?** It is not
      ramped, by design, and that is consistent with - not in tension
      with - the jump this document already accepts and defends two
      sections up ("the *overall* output does jump at punch-out...
      that's the same thing real hardware does when you stop feeding a
      bus and start monitoring the result instead"). A smoothed
      transition would paper over exactly the jump this document argues
      is correct and honest to keep audible. It also isn't the kind of
      thing REQ-602 (smoothed *mixer moves*) or REQ-302 (the 5ms tape-
      side punch crossfade) already governs - neither is about a track's
      presence in the monitor sum changing because a bounce pass opened
      or closed; this is a third, new category of transition this
      proposal introduces, coincident with but distinct from the bus's
      own punch boundary. Whether it registers as a "click" to the
      testkit's click detector in the technical sense is an empirical
      question this document isn't going to guess at - it's exactly what
      the REQ-408 test two sections up already exercises at that
      boundary; if the detector does flag it, that is new information
      about the detector's threshold, not evidence the design is wrong.
- **The existing bounce test suite, named explicitly (a seventh review
  found it missing from this list entirely)**:
  `crates/porta-engine/tests/bounce.rs` is a whole file of tests built
  on today's REQ-401/REQ-603 semantics -
  `bounce_sums_the_source_tracks_onto_track_four`,
  `bounce_respects_faders_and_ignores_pans` (encodes REQ-603 directly),
  `bounce_excludes_muted_tracks`, `bounce_is_undoable`,
  `bounce_applies_the_character_again`, `bounce_is_refused_while_rolling`,
  `bounce_is_reproducible`, `bounce_leaves_the_source_tracks_alone` (a
  tenth review found this one missing from the list) - every one of them
  needs rewriting or replacing for the new semantics, not just the
  golden render.
  `crates/porta-engine/tests/generation_loss.rs`'s
  `generations_get_duller_and_noisier` is REQ-403's *actual* current
  acceptance test (it uses track-to-track passes, not old bounce - see
  its own module doc) and needs the rewritten procedure described
  above, in place, not a new file alongside it.
- **The UI's own Bounce button, wired up this cycle, needs replacing,
  not just its handler deleted**: a "Bounce" button (`ui.rs`'s
  `on_bounce_pressed`, `main.slint`) now calls `Engine::bounce()`
  directly, exactly the `Command::Bounce` path this proposal removes.
  Once the bus lands, that button's handler becomes "arm the bus,
  then Record" (REQ-404/405) instead of one blocking call - a real UI
  behavior change on top of the engine one, not free. Two more
  interactions an eighth review flagged, non-blocking today (this
  proposal isn't implemented yet, so neither is false *yet*) but real
  once it is, so noted here rather than rediscovered later:
  - `ui.rs`'s `LiveState` doc comment currently asserts "this UI is the
    only source of [arm/fader/pan/master] commands (true today)",
    which is exactly why `Backend::send` sets `LiveState.armed`
    optimistically at send time instead of waiting for an echo back
    from the audio thread. REQ-405's auto-clear (arming the bus clears
    all 4 tracks' armed state, and vice versa, inside the engine) would
    make that assertion false the moment it lands: the engine would be
    a second source of arm-state changes the UI's mirror never sees.
    Whoever implements REQ-405 needs to either echo that auto-clear back
    through an `EngineEvent` the UI mirrors, or accept a stale
    `LiveState.armed` display until the next full resync - a real choice
    to make then, not assumed here.
  - REQ-409 (the bus's own fader/mute) has no UI surface at all yet -
    the Tapes view's Bounce button only reaches arm+Record, nothing
    reaches the bus's fader or mute. Not a blocker for landing the
    engine side, but the feature isn't usable end-to-end without it.
- **`Mixer` needs a per-track "excluded from the sum, still metered"
  flag** for REQ-408's metering clause - see the `Mixer::mix_block`
  bullet above (`sum_tracks`) for the full mechanism now that it's split
  across two functions; the short version is that today's single
  `mix_block` derives a track's meter peak from the same slice that
  feeds the sum, `peak * fader_amp` off `input`, so silencing a track's
  contribution would silence its meter with it without this flag - a
  small, new, explicit mechanism, not a free ride on an existing
  separation.
- **The bus's smoothed gain, concretely: how "tick once, reuse for
  both uses" actually gets implemented (a seventh review asked for this
  to be more than a rule in prose; a ninth pinned it down further once
  the chain's actual location - outside `mix_block` entirely, see
  above - was corrected)**: the bus's gain is needed at two points that
  straddle its own character chain within one block - once folded into
  the pre-chain print input (REQ-406, the step between `sum_tracks` and
  `finish_mix` above), again for the post-chain monitor output (REQ-408,
  inside `finish_mix`) - and `Smoothed::tick()` advances a ramp one step
  per call, so it can't just be called twice at each use site. The pass
  needs one small, pre-reserved per-block scratch buffer - `[f32;
  MAX_BLOCK]` is already the right size class for this codebase's
  existing per-block scratch buffers - allocated once, off the realtime
  thread, at cassette open/create (a tenth review corrected "alongside
  the bus's other per-pass setup": that setup runs inside
  `Engine::record()`, which this document establishes elsewhere runs *on*
  the realtime thread - the scratch buffer has to be ready before that,
  not built there). Filled once per block, by
  whichever of the two steps runs first for that block (the between-
  phases step when a bounce is open, `finish_mix` itself when it isn't),
  ticking the bus's `Smoothed` gain `n` times up front (`n` = the
  block's sample count), then read back (not re-ticked) by whichever
  step runs second. Tracks 1-4's own `Smoothed` L/R ramps need the same
  discipline during a bounce: their audible contribution goes silent
  (REQ-408 via the exclude flag), but `sum_tracks` is what ticks their
  ramps at all now - skipping that call entirely to implement "silent"
  would freeze their ramps for the whole pass and snap them to the live
  value at punch-out, a real REQ-602 click and a REQ-203 violation (the
  freeze duration would depend on block size). Tracks 1-4's ramps MUST
  keep ticking every sample during a bounce, same as ordinary playback -
  `sum_tracks` already guarantees this by construction, since it ticks
  every track's ramp unconditionally and only the exclude flag decides
  which sum a track's already-ticked value lands in.
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
  **Construction and reseeding, stated explicitly (a ninth review found
  this whole bullet never says where the bus's chains are built or how
  they're reseeded per pass without allocating; a twelfth found the
  previous version's two build-site options were *both* wrong - "if the
  manifest has ever had the bus armed" depends on manifest state that
  deliberately doesn't exist (arm is session-transient, never saved,
  `project.rs`'s own rule), and "lazily the first time the bus is
  armed" runs on the realtime thread, since arming isn't a blocking
  command - the identical REQ-902 violation rounds 9-10 diagnosed and
  fixed in `Engine::record()`, reintroduced in prose one bullet over
  from where round 11 caught the same mistake)**: both per-channel split
  chains are built **unconditionally** at cassette open/create, exactly
  as `Engine`'s constructor already builds all four track chains with a
  throwaway seed (`chains: (0..NUM_TRACKS).map(|_|
  character.build_chain(0))`, `engine.rs:100`) - no conditions, no
  laziness, nothing for a later reader to reinterpret as an
  optimization opportunity. Reseeding per pass reuses the same mechanism this
  proposal's REQ-902 audit already added and shipped for ordinary tracks
  - `AudioProcessor::reseed`/`Chain::reseed_stage`. **The full per-pass
  sequence, stated as a sequence (an eleventh review found the previous
  narrowing - "only the pre-flutter chain's `reseed_stage` needs
  calling" - dropped two real pieces of state on the floor: it read as
  though `reseed_stage` alone sufficed for the pre-flutter chain, leaving
  Bandwidth's biquad state carrying over between passes, and it never
  reset `StereoFlutter`'s two `FlutterDelay` ring buffers at all -
  ~480 samples (`CENTRE`) of the *previous* bounce's audio bleeding into
  the next pass's punch-in, silently, in a way that also breaks the
  reused-equals-fresh property `reseed_chain_matches_a_freshly_built_one`
  established for tracks)**. Mirroring `TapeCharacter::reseed_chain`'s
  own reset-then-reseed shape, per channel and per pass:
  1. `reset()` **both** per-channel `Chain`s - the pre-flutter one
     (`[Saturation, Hiss, Bandwidth]` - this is what clears Bandwidth's
     biquads) and the post-flutter one (`[Crush]` if enabled, **empty
     otherwise** - crush is opt-in, per `crush_is_opt_in`; `Chain`'s own
     `reset` iterates whatever stages it has, so the empty case is safe).
  2. `reseed_stage(HISS_STAGE, ...)` on the pre-flutter `Chain` only -
     Hiss is the only seeded stage in either sub-chain; the `HISS_STAGE`
     constant belongs to the *split* builder, kept next to it the same
     way `TapeCharacter::HISS_STAGE`/`FLUTTER_STAGE` are kept next to
     `build_chain`, so builder and reseeder can't drift apart.
  3. `StereoFlutter::reseed(seed)` - an inherent method, not a trait
     override (`FlutterModulator` isn't an `AudioProcessor`; it emits a
     delay value, not audio), which MUST clear **both** `FlutterDelay`
     rings and their write indices *and* reseed the shared modulator.
     The invariant to hold it to: `StereoFlutter::reseed` clears exactly
     the state `Flutter::reset` clears today (`ring`, `write`,
     `wow_phase`, `walk`, `walk_lp`, `state`), just distributed across
     three objects instead of one.
  Per-channel hiss still decorrelates L from R (REQ-702's `channel`
  term); the shared modulator reseeds once, at channel term 0 (see
  "Shared flutter" above). The natural regression test is the same
  property already shipped for tracks: a reused, reseeded stereo pass
  setup must produce output identical to freshly-built ones with the
  same seeds.
- **Latency accumulation across fold-forward bounces - decided, not left
  open (a sixth review correctly said this needs a real decision, not
  just an honest flag)**: `Flutter`'s ~480-sample (10ms) centre-tap delay
  is not new to this proposal - it's inherent to the character chain
  every ordinary record pass already goes through, uncompensated, today
  (`latency_samples()` exists but nothing calls it). Every track's
  recorded content is already ~10ms shifted from "true" input timing on
  its very first pass; this proposal doesn't introduce that, it just
  means a bounce compounds it across generations the same way
  overdubbing an already-flutter-affected track would. **Decision:
  accept the drift**, consistent with how the engine already treats
  every other pass - real cassette tape doesn't perfectly time-align
  across generations either (wow/flutter is itself a genuine timing
  smear on real hardware, not just a numerical artifact of this one).
  The alternative - compensating via `latency_samples()` when reading
  the bus's prior content - would mean reading at `position - 480`,
  which contradicts REQ-407's "block-local read-then-write... not a
  lookahead" as written; rejected specifically to avoid reopening REQ-407
  for a discrepancy real hardware shares anyway. This does mean the
  stereo-image and REQ-403 tests should tolerate a few hundred samples
  of drift across generations rather than asserting exact alignment -
  noted so the tests are written correctly the first time.
- **REQ-905 / M6 CPU headroom - genuinely needs on-device measurement,
  not resolvable on paper, but given a concrete task and a stated
  fallback rather than left as a bare flag**: today's bounce is an
  offline batch operation with no realtime deadline. This design makes
  it realtime, running two full character chains (independent
  per-channel saturation/hiss/bandwidth/crush plus one shared
  `StereoFlutter`) inside the same audio callback as tracks' own chains,
  on a Pi 4 at a 128-256 frame period. Task: measure actual callback
  headroom with a bounce running alongside armed tracks, as part of
  M6.2's existing performance pass (`TASKS.md`), before this ships, not
  after. Stated fallback if it doesn't fit, corrected for reachability
  (a seventh review pointed out a period change means tearing down and
  rebuilding the whole ALSA/cpal stream, which can't happen with a
  record pass open): raise the frame period *before* arming the bus,
  as a deliberate, chosen tradeoff for a bounce specifically (REQ-905
  already treats 128-256 as a target, not a hard floor) - not something
  that can adjust mid-pass, and not a decision this document needs to
  finalize, but a real, reachable fallback rather than "redesign
  everything."
- **New arm-like flag** for the bounce bus (REQ-404), plus the mutual-
  exclusion wiring with tracks 1-4's `armed` array (REQ-405).
- **`process_block`**: the bus becomes a fifth mix contributor (fader +
  mute, no pan) in both ordinary playback and while a bounce pass is
  running; its own read during a pass follows REQ-407's read-before-
  write ordering.
- **Journal**: `Entry`/`RecordPass` gain a multi-channel variant used
  only by the bus (ordinary tracks keep the existing single-channel
  shape) - see "Undo" above.
- **REQ-403's test, rewritten procedure (corrected again - v4's script
  used Arm, which does nothing to a track's mix contribution; muting
  is what's needed, and `Op::Mute` didn't exist yet)**: the old three-
  successive-bounces procedure doesn't transfer - because the bus folds
  its own prior content forward every pass, re-bouncing *unchanged,
  unmuted* source material re-injects full-bandwidth signal each
  generation, so measuring HF energy/noise floor across generations 1-3
  would measure "source material present or not," not generation loss.
  All three *measured* generations need identical input conditions:
  bounce once with tracks 1-4 unmuted (primes the bus), then mute
  tracks 1-4 for real and bounce three more times, measuring generations
  2, 3, and 4 (bus re-printing only its own prior content each time)
  for the existing monotonic HF-loss/noise-floor-rise assertion.
  Scriptable via `Op::BounceArm{on:true}`, `Op::Bounce{...}`,
  `Op::Mute{track,on:true}`x4, `Op::Bounce{...}`x3.
- **New test, named metric added (a sixth review pointed out neither of
  these two had one)**: stereo image survives a bounce and a second
  bounce - hard-pan a source fully left, bounce, bounce again; assert
  the printed right channel's band energy in the source's frequency
  range stays at least 10dB below the left channel's (via
  `porta_testkit::spectral::band_energy_db`, already used elsewhere in
  this codebase for exactly this kind of assertion) - a real floor, not
  "still sounds panned."
- **New test**: `StereoFlutter`'s two channels' delay excursions
  correlate (driven by one `FlutterModulator`), verifiable directly by
  feeding it identical input on both channels and asserting the outputs
  match - simpler and more precise than inferring it from a full bounce
  render. A second test, named metric added: hiss stays decorrelated
  between the two channels of an actual bounce pass - assert the
  Pearson correlation coefficient between the two channels' hiss-only
  regions stays below 0.1 over a multi-second window (REQ-702).
  **`porta_testkit` doesn't have a correlation-coefficient helper today
  (checked `spectral.rs`/`meter.rs` - a seventh review caught this
  wasn't flagged)** - a small new function there, alongside the
  existing `band_energy_db`/`thd_db`, not a one-off computed inline in
  the test itself.
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
  that the output contains a *sustained run* of consecutive samples
  pinned at the same extreme value (a flat-topped plateau, not a single
  boundary touch) - a seventh review correctly pointed out that a lone
  sample sitting exactly at `i16::MAX` (32767) or `i16::MIN` (-32768)
  isn't on its own unambiguous evidence of clamping (i16's range is
  asymmetric, and either boundary is also where an ordinary, un-clipped
  signal peaking at exactly full scale would naturally land) - a
  multi-sample flat top is the actual, standard clipping signature real
  audio analysis looks for, and isn't produced by an unclipped signal
  by coincidence. No upper bound on how much of the signal clips is
  asserted in this document - guessing one (v5's "under 50%") without
  having actually run
  the pass is asserting a number nobody has measured; pin a regression
  bound from the real figure once this is implemented, not before.
- **New test, specified precisely (a sixth review pointed out the
  whole-output version can't pass; a seventh found the narrowed
  version's wording still ambiguous about what's being compared; a tenth
  found the window it excluded was still wrong; an eleventh found it
  could pass vacuously on silence - see "Monitoring" above for the
  worked-out claim this test checks)**: first **prime the bus with real
  content** - record material on tracks 1-4 and bounce once with them
  unmuted, the same priming step REQ-403's rewritten procedure already
  uses. Without this, muting tracks 1-4 below leaves `P(t) = 0` for the
  whole measured pass (muted tracks contribute zero to the print sum
  too - `target()` returns zero gain for a muted track, and both sums
  accumulate from the identical scaled value), the crossfades blend
  silence into silence, and the assertion passes without demonstrating
  anything - including whether the excluded windows were even placed
  correctly. Then, for the measured pass: mute tracks 1-4 (isolating the
  bus's own now-non-silent contribution from the expected, accepted
  tracks-1-4 jump), set a **cut**, already-settled bus fader (-6dB via
  `Op::BounceFader`, set before punch-in so its smoothing ramp has
  converged - a cut, not a boost, since a boost would scale the dither
  error above the RMS bound below and the claim doesn't hold for that
  case), and bounce a region long enough that a middle stretch clear of
  *both* crossfade windows exists, **lying entirely inside the region
  the priming bounce wrote** (a twelfth review caught that a measured
  pass running past the primed region returns to the silence-vs-silence
  vacuity the priming was added to eliminate, for its unprimed tail),
  and **ending short of the tape end**
  (needed for the replay step anyway, and `finish` skips its out-fade
  entirely at the tape end - see "Monitoring" above - so stopping short
  is also what makes the punch-out boundary exist to exclude). Capture
  the monitor output *live, during* the pass, over that middle stretch
  only - clear of the first `XFADE_SAMPLES` (`write_block`'s punch-in
  fade blends toward the un-faded printed value the monitor slot holds,
  a real divergence, not dither) **and** the last `XFADE_SAMPLES`
  (`finish`'s punch-out fade retroactively rewrites what's on tape after
  those positions were already monitored at their un-faded values - the
  live capture is just a buffer, so its tail is trimmed after the pass
  closes, from the known total length). Separately, after the pass
  closes, seek back and *play back that same tape region* fresh.
  **Assert the RMS of the per-sample difference between the two captures
  is under ~0.25 LSB (the ~0.5 LSB RMS dither error derived in
  "Monitoring" above, scaled by the -6dB fader - stating the bound at
  the chosen gain rather than leaving the un-scaled figure with silent
  2x slack), not a per-sample tolerance** (a ninth review corrected
  this: a per-sample bound would need to cover the combined
  TPDF-plus-rounding worst case, +/-1.5 LSB, which is loose enough to
  hide real bugs; RMS is both the tighter and the actually-motivated
  check, since it's asserting the *distribution* dither is supposed to
  produce, not just bounding its extremes). This is "monitored live vs.
  replayed after, same position," not "during the pass vs. what that
  position would have sounded like un-bounced."
- **New test, separate from the one above (an eleventh review caught
  that folding REQ-408's metering assertion into the dither test
  contradicts its own setup: that test *mutes* tracks 1-4, and `Mixer`'s
  meter deliberately reads a muted track as silent -
  `mute_silences_a_track_without_touching_its_fader` already asserts
  exactly that - so "meters not silent" can never hold there, for a
  reason unrelated to REQ-408)**: tracks 1-4 **unmuted** and carrying
  real signal, **the bus muted** (a twelfth review pointed out the
  clean measurable form: during a bounce the audible output *does*
  carry tracks 1-4's material via the bus's printed `W`, so "output
  contains no tracks-1-4 contribution" isn't directly assertable -
  muting the bus makes the audible output exactly silent, `g = 0`,
  while the exclude flag is what's keeping the tracks out), a bounce
  pass open: assert each track's `track_level_db` reads above the meter
  floor while the block's audible output is silent - the
  exclude-flag-vs-meter separation `sum_tracks` exists to provide,
  exercised directly (REQ-408's metering clause).
- **New test**: one Undo press after a bounce restores both channels
  atomically - no reachable state with one channel reverted and the
  other not (REQ-502/505).
- **New test, corrected (the previous version tested a mechanism this
  proposal no longer uses)**: two bounces run back-to-back with nothing
  saved in between never fall back to a realtime-thread allocation
  (`pass_buffer_fallbacks()` stays 0) - the case the double-buffered
  reserve exists to cover. A second test: a *third* bounce in the same
  circumstance is allowed to fall back (documented, accepted behavior,
  not a bug) - asserts the honest boundary of the guarantee rather than
  claiming an unlimited one.
- **REQ-904 (resident memory ceiling), itemized carefully - this number
  has been wrong three times already (v3, v4, v5), each time from
  leaving out a real mechanism; the arithmetic method below is the one
  the sixth review independently verified, re-run here against
  the same code, and rounds conservatively rather than claiming false
  precision**:

  *Steady-state* (cassette open, nothing mid-flush):
  - Tape storage: 4 tracks x 172.8MB + 1 stereo bus x 172.8MB/channel
    = 691.2MB + 345.6MB = **1036.8MB**
  - Track chunk pool (already shipped, independent of this proposal):
    4 x `CHUNK_POOL_PER_TRACK`(24) x `CHUNK_SAMPLES`(240,000) x 2 bytes
    = **~46MB**
  - Bus dedicated reserve (this proposal's new mechanism, see
    "Realtime-safe allocation" above - **two** full-tape-sized buffers
    per channel, double-buffered so a second bounce with nothing saved
    in between doesn't allocate): 2 buffers x 2 channels x 172.8MB =
    **691.2MB**
  - Steady-state total: **~1774MB**

  *Additional worst-case transient*, during an Undo of a full-length
  stereo bounce entry, recomputed a second time - a seventh review
  caught that the first version of this figure assumed a per-channel
  read that doesn't exist: the on-disk layout is one file per entry,
  left-channel bytes then right-channel bytes concatenated (see
  "Persistence" above), and `read_payload` reads that whole file in one
  `f.read_to_end(&mut bytes)` call - for a stereo entry, one 345.6MB
  read, not two 172.8MB ones. `Journal::undo`'s `current` (both
  channels, read before restoring, for the atomic restore REQ-505
  requires) and `write_payload`'s byte-encoding buffer (re-encoding
  `current` as the new redo payload) are each sized the same way.
  Three separate, temporary, whole-entry allocations, all live briefly
  at once: 3 x 345.6MB = **~1037MB**, on top of the steady-state figure
  above, only while that specific operation runs. (For inventory
  completeness, a twelfth review also noted the *flush* transient:
  `write_payload_chunks` allocates a byte buffer per chunk at save
  time, up to ~346MB for a full stereo payload with both channels live
  - off the realtime thread, and immaterial inside this section's
  stated generous-additive-not-tight-bound framing against ~3GB of
  margin, but named so the inventory has no known omissions.)

  **Worst-case peak: ~1774MB + ~1037MB ≈ ~2.8GB.** Deliberately a
  generous, additive estimate, not a tight bound - the only claim that
  needs to hold is "fits comfortably in the real device's headroom,"
  which it does by a wide margin regardless of exactly how conservative
  this arithmetic is: checked against the actual deployment Pi
  (`patch@192.168.68.55`, confirmed via `free -h`: 8GB total, ~5.8GB
  free at idle with the desktop session and audio stack already
  running) - ~2.8GB peak against ~5.8GB free still leaves well over
  2.5GB of margin even in the worst case. The ceiling is revised from
  ~700MB to ~2.8GB peak (~1774MB steady-state; default 15-minute
  cassette roughly half each figure) rather than shortening max
  cassette length or the bus, because on the real target hardware
  there is no actual memory pressure to trade against. If this project
  ever targets a smaller-RAM Pi 4 variant, this whole section needs
  recomputing against that device's real headroom, not assumed to
  still hold - noted here so it isn't forgotten. (A per-channel,
  seek-based `read_payload` variant would cut this transient roughly in
  half - not specified here, since the margin already comfortably
  supports the simpler, whole-file version, and this document has
  gotten this specific number wrong three times already from describing
  a mechanism slightly ahead of what's actually specified elsewhere in
  it. Worth doing as a later optimization, not a blocker.)

  Said plainly, since a reviewer asked for the basis to be explicit:
  REQ-904's basis is changing from "tape buffers alone" to "tape buffers
  plus every realtime-safety reserve this proposal and its prerequisite
  depend on." The already-shipped 46MB track pool alone puts *today's*
  actual resident figure at ~737MB, already past the currently-
  documented ~700MB, independent of whether the bus itself is ever
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
- **`Manifest` gains `bounce_fader_db`/`bounce_muted`** (with
  `apply_to`/`capture_from` plumbing and a save-reopen roundtrip test) -
  REQ-409's persistence, see the REQ-801/802 bullet above for the full
  rationale.
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
  layered on top of the new bus instead of reusing arm+Play/Record:
  rejected because it reintroduces the pre-existing gap (bounce wasn't
  reachable from the live UI or interactive session) and, like the
  offline option, can't be ridden live.
- **A variable-length (grow-as-you-bounce) bus** instead of a fixed
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
bus, printed in real time, not reusing ordinary tracks. This avoided
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
audible 5th bus was called out as dishonest given what it actually
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
bus-fader double-application and track-metering questions open;
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
computation were both eliminated. REQ-408 gained bus-fader-output and
metering clauses; a new REQ-409 gave the bus its own fader/mute.
Shared flutter got the `FlutterModulator`/`FlutterDelay`/`StereoFlutter`
design. A fifth review verified the shipped code held up this time
(it did), but found the *new* material had its own real problems:
REQ-408's rule was mathematically backwards - tracing the actual math
showed "print directly, no bus fader" produces exactly the punch-out
discontinuity it claimed to prevent, not the reverse; the extended
chunk-pool plan for the bus doesn't work at all, because
`CHUNK_POOL_PER_TRACK` (2 minutes) is sized for an ordinary take and a
bounce is by definition close to the full tape - the "rare fallback"
becomes the normal case for this specific operation; REQ-904 was wrong
a third time (missed the resident cost of unflushed payloads and the
transient cost of undoing one); the peak-level test's "fixed" assertion
was still unfalsifiable (an `i16` cannot be outside `i16` range,
obviously in hindsight); REQ-409 didn't extend REQ-602's carve-out to
the bus's own fader/mute or state its smoothing; the stereo journal
entry's byte-accounting (`len` per-channel or total?) was undefined;
and several real implementation surfaces were still missing from Impact
on tasks (a `Chain`-splitting builder in porta-dsp, latency
accumulation across generations, REQ-905/M6 CPU headroom for two
character chains in the realtime callback, a script op to actually set
the bus's fader for REQ-408's own test to be writable).

**v6**: REQ-408 rewritten with the corrected direction, the bus's
dedicated full-tape reserve introduced, REQ-904 recomputed a third
time, REQ-409/406/407/703 and the on-disk-format questions addressed,
and two items (latency accumulation, REQ-905 CPU cost) left explicitly
open rather than papered over. A sixth review verified the shipped-code
claims this time were accurate (a first, after five rounds), and the
REQ-904 arithmetic was right *for the model it stated* - but found two
real defects in what v6 itself introduced: the corrected REQ-408
*narrative* never made it into the REQ-408 *normative bullet*, which
still had v5's inverted rule - the exact text that would get copied
into `spec.md`; and the new bus reserve, being one monolithic buffer
per channel, has no partial-use remainder to give back immediately the
way a track's chunked reserve does, so it reproduces the original
chunk-pool bug in a new shape - a second bounce with nothing saved in
between (not an edge case; it's the feature's own motivation #2)
allocates on the realtime thread again. Also found: REQ-408's own test
as specified can't pass (there's a real, correct jump in the *overall*
output at punch-out when tracks un-silence - the claim needed narrowing
to the bus's own contribution); "apply master gain as one scalar pass"
silently drops master moves out of their smoothed ramp, a real REQ-602
regression nothing in the test suite would catch; the bus's smoothed
gain is needed twice per sample (print input, monitor output) and
`Smoothed::tick()` can't be called twice without breaking REQ-203; the
stereo journal payload's on-disk layout and give-back routing were
unspecified; dither seeding for a stereo pass was unstated; and two
tests had no named numeric assertion.

**v7**: REQ-408's normative bullet now matches its corrected narrative.
The bus reserve is double-buffered - two full-tape buffers per
channel, not one - so a second bounce with nothing flushed in between
takes the second buffer instead of allocating; a third in the same
circumstance is the documented, accepted fallback boundary. REQ-904
recomputed a fourth time with the doubled reserve. REQ-408's test is
scoped to the bus's own contribution (tracks muted) rather than the
whole output, which genuinely does jump when tracks un-silence - stated
explicitly instead of asserted away. The master-gain refactor gets its
own smoothed instance instead of a bare scalar multiply; the bus's
smoothed gain is ticked once per sample and reused for both uses. The
stereo payload's on-disk layout (one file, left-then-right) and the
track-vs-bus give-back routing are pinned. Dither seeding extends the
same channel-term derivation already used for hiss. The stereo-image
and hiss-decorrelation tests get real numeric thresholds. Latency
accumulation is decided (accept the drift) rather than left open;
REQ-905's CPU-cost item gets a concrete task and a stated fallback. A
seventh review confirmed the chunk-pool fix genuinely held up this time
(the first clean pass on shipped-code claims, after six rounds), but
found: the corrected REQ-408 rule was in the narrative section but the
normative bullet still had the old, inverted one - the exact text meant
to become `spec.md`; `Journal::evict()` and `push_pass`'s redo-branch
invalidation both dropped a still-pending entry's chunks via
`Vec::retain` instead of returning them - a real deallocation on the
realtime thread and a silent, permanent leak from the reserve, the same
give-back bug shape a third time; the REQ-904 transient assumed a
per-channel disk read that doesn't exist (`read_payload` reads the
whole file at once); the print/monitor split's "tick once, reuse for
both uses" had no concrete mechanism for a chain that runs *between*
the two uses; REQ-408's own test compared the wrong two things (whole
output vs. a single tape position, live vs. replayed); `tests/bounce.rs`
and `generation_loss.rs` weren't named as affected files; `StereoFlutter`'s
one shared modulator had no pinned seed; and several smaller gaps (the
bus's `Tape`-addressing hazard, a missing correlation-coefficient
testkit helper, an unreachable REQ-905 fallback, an asymmetric-range
issue in the clamp test, an overclaimed "mathematically identical" for
the master refactor).

**v8**: the eviction/redo-invalidation leak is fixed
for real this time, in shipped code, the same day it was found - one
shared `release_entry_payload` used by both call sites, with its own
regression test; the proposal now describes it as landed, not planned.
REQ-408's normative bullet is rewritten to match the narrative for
real. REQ-904's transient is recomputed against `read_payload`'s actual
whole-file read (~2.8GB peak, still comfortable). The print/monitor
split gets a concrete mechanism: a small pre-reserved per-block gain
scratch buffer, ticked once up front and read back at both use sites,
with tracks 1-4's own ramps explicitly required to keep ticking (silent
in the sum, not frozen) during a bounce. REQ-408's claim and test are
re-derived explicitly with the chain included, and reframed as "the
same tape position, monitored live vs. replayed after" - not "during
vs. un-bounced." `StereoFlutter`'s modulator seed is pinned to channel
term 0. `tests/bounce.rs`, `generation_loss.rs`, and the UI's own newly-
wired Bounce button (shipped this cycle, calling `Engine::bounce()`
directly - see `TASKS.md`) are all named as needing to change.
REQ-603 is specified as deleted, not reworded. The remaining smaller
gaps (bus `Tape` addressing, the correlation-coefficient helper, the
REQ-905 fallback's timing, the clamp test's asymmetric-range issue, the
master refactor's steady-state-only claim) are all addressed.

**v9**: an eighth review found the Motivation section's
peak-memory summary line still said the stale ~2.3GB figure from v7
after v8 corrected the detailed section to ~2.8GB - fixed, and this
document is now the second recorded instance of that exact mistake
(round 6 also let a corrected narrative leave a stale normative bullet
standing) - worth treating as a standing failure mode, not a one-off:
after any numeric or rule correction, grep the whole document for the
old value, don't trust that fixing one section propagates. The
`Mixer::mix_block` bullet's single "pre-master intermediate sum" was
found to be genuinely ambiguous, not just imprecisely worded: REQ-406
needs tracks 1-4's *full* contribution to feed the print, REQ-408 needs
their contribution *excluded* from what's audible, during the same
bounce pass - one sum cannot mean both. Resolved by specifying two
distinct pre-master sums (the existing monitor sum, gated by the
"excluded from sum, still metered" flag; a new print sum, tracks 1-4
only, never gated by that flag) and stating explicitly which one the
flag applies to. REQ-408's central claim was found to still overclaim:
"the bus's `playback` slot holds `W(t)`... reads `W(t)` off tape" skips
that what's actually on tape is `quantize(dither(W(t)))`, not `W(t)`
itself - re-derived to state the honest bound (matches within TPDF
dither's noise floor, roughly +/-1 LSB / -90dBFS at 16-bit, not
bit-identical - a number this document would itself get wrong in this
exact revision, see v10), and noted this is the same approximation
REQ-305
already makes for ordinary track monitoring today, not a new gap the
bus introduces. The eviction fix's own `reclaim_chunks` was found to
still drop overflow chunks (beyond a track's `CHUNK_POOL_PER_TRACK`) in
place when called from the realtime-reachable `release_entry_payload`
path - fixed in shipped code the same way the original leak was: a new
`Journal.pending_frees` list parks what doesn't fit, and `flush_pending`
(always off the realtime thread) is where those chunks actually drop,
with its own regression test. Three smaller items also addressed: the
version-count line at the top of this document was stale since v8 (said
"six times... v7"); the "4-element track array" wording was corrected
(`Tape.tracks` is a `Vec`, so an out-of-range append wouldn't panic -
it would silently go unread by any `0..NUM_TRACKS` loop, a worse
failure mode than the one originally described); and the UI's Bounce
button bullet now names its two forward-looking interactions with this
proposal (REQ-405's auto-clear would make `ui.rs`'s current "only
source of arm state" doc comment false once implemented; REQ-409 has no
UI surface yet), flagged as real but not blocking since neither is
false until this proposal actually ships.

**v10 (this revision)**: a ninth review found three blocking problems.
The two-sum design from v9 was directionally right but its call site was
wrong: it put both sums inside one `mix_block` invocation despite the
print sum needing to exist *before* the character chain runs and the
monitor sum *after* - which silently requires entering `mix_block` twice
per block and double-ticks every track's fader/pan ramp, the exact bug
this document already diagnosed for the bus's own gain, reintroduced
one level up. Fixed by first correcting a fact this document had lost:
the character chain was never inside `mix_block` to begin with - it runs
in `Engine::process_block`'s existing per-track loop, before `mix_block`
is called at all. `mix_block` splits into `sum_tracks` (ticks track
ramps once, produces both sums from the same pass) and `finish_mix`
(adds the bus's contribution, ticks the master ramp once, clamps), with
the bus's own chain running between them - three steps, each ramp
ticked by exactly one of them, no re-entry. The dither bound v9 itself
introduced was found wrong against the code (TPDF spans +/-1 LSB, not
+/-0.5 as v9 said - it's the *difference* of two independent uniforms,
a triangular distribution) and inconsistent across three places in the
document besides; corrected everywhere to the derived value (~0.5 LSB
RMS, ~-96dBFS, not the -90dBFS a single-LSB peak value would be - a
different quantity), and the corresponding test respecified as an RMS
assertion rather than a per-sample tolerance, which the wrong bound
would have made either too loose or outright failing depending which
number got used. `Engine::record()` was found to still allocate on the
realtime thread - a real, pre-existing REQ-902 violation independent of
this proposal (`build_chain` rebuilding a `Chain` from scratch every
time recording engages), missed by every previous "realtime-safe
allocation" pass because it isn't part of what this proposal adds.
Fixed in shipped code, not just noted: `AudioProcessor` gained `reseed`,
`Chain` gained `reseed_stage`, `TapeCharacter::reseed_chain` mirrors
`build_chain`'s seed derivation without allocating, and `Engine` now
builds each track's chain once off-thread and reseeds in place - with
its own regression test proving the reused path matches a fresh build
exactly. Three medium findings also fixed: a citation had the actual
code order backwards (the pre-dither monitor copy runs *after*
`write_block`'s dither, not before - the conclusion held, the claimed
mechanism didn't); the v9 "Vec, not an array" correction pointed at the
wrong data structure (`Journal.chunk_pool` really is the fixed array the
give-back-routing paragraph is about and panics correctly; `Tape.tracks`
is the `Vec` with the silent-skip failure mode, a different structure
entirely) - split back into the two correct, separately-attributed
claims; and REQ-408's metering clause said tracks keep reflecting "live
signal," which REQ-405 two bullets up already rules out (no track can be
armed during a bounce) - corrected to "playback contribution." The
exclude flag's own click question (medium) is resolved: unramped, by
design, consistent with the punch-out jump this document already
defends elsewhere, not something REQ-602 or REQ-302 already governs.
The "Chain-splitting in porta-dsp" bullet, which a ninth review found
never said where the bus's own chains get built or reseeded, now
points at the same off-thread-build/in-place-reseed mechanism the REQ-902
fix above just shipped for ordinary tracks.

**v11**: a tenth review confirmed all six of round 9's
items land correctly (including re-verifying the shipped REQ-902 fix and
the dither arithmetic independently against the actual code) and found
one new blocking problem in the process. REQ-408's derivation named the
bus's own gain ramp as the only caveat to "monitored live matches
replayed-after" - the wrong mechanism, and missing the real one: REQ-302's
tape-side punch crossfade applies at *both* boundaries of a bounce pass,
not just the ramp settling at punch-in. `write_block` blends the first
`XFADE_SAMPLES` written to tape toward the displaced content, while the
monitor slot holds the un-faded value throughout; `finish` *retroactively*
rewrites the last `XFADE_SAMPLES` after they were already monitored live
at their un-faded values. Both are real, much larger than dither noise,
and the REQ-408 test as previously specified excluded only the punch-in
window - it could not have passed once punch-out's retroactive rewrite
was included. Fixed: the derivation now names REQ-302's crossfade at
both ends as the actual caveat, and the test excludes both the first and
last `XFADE_SAMPLES` of the pass (the punch-out boundary is knowable in
advance since the test controls the pass length). Four medium findings
also fixed: the print sum's own storage was unspecified (`Engine`-owned
`[f32; MAX_BLOCK]` L/R buffers, named alongside the gain scratch buffer);
the "Chain-splitting" bullet's reseed description was stale (`reseed` is
already on `AudioProcessor` as of the shipped REQ-902 fix - the real gap
was that `FlutterModulator` isn't an `AudioProcessor` and needs its own
inherent `reseed`) and had a latent panic hazard (a post-flutter
sub-chain with crush disabled is empty; only the pre-flutter sub-chain's
Hiss stage ever needs `reseed_stage`, not a uniform call across both);
citations to `engine.rs:329`/`:333` were pre-419e322 line numbers,
repointed to `:343`/`:347`; and the metering clause's "live signal" fix
from v9 hadn't propagated to two other places it appeared (the narrative
clause, the test bullet), plus the corrected wording itself overclaimed
pan (`Mixer`'s meter is post-fader, pre-pan - both now fixed). Two minor
findings: the RMS dither bound only holds for a bus fader at or below
unity, so the test now specifies a cut rather than an unspecified
"non-unity" setting; and bouncing with the bus muted produces total
monitor silence (tracks excluded by REQ-408, bus gain zero by its own
mute) - a real consequence of two already-correct rules, now stated
rather than left to be discovered. A missing test name
(`bounce_leaves_the_source_tracks_alone`) and the scratch buffer's
self-contradictory allocation site ("alongside per-pass setup," which
this document itself says runs on the realtime thread) are also
corrected.

**v12**: an eleventh review verified every one of round
10's fixes against the code (including the crossfade mechanics in
`write_block`/`finish` line by line) and found two blocking problems,
both in material earlier rounds had already touched. The chain-splitting
bullet's narrowed reseed guidance dropped two pieces of per-pass state:
it read as though `reseed_stage(HISS_STAGE)` alone sufficed for the
pre-flutter sub-chain (Bandwidth's biquads would carry over between
passes), and it never reset `StereoFlutter`'s two `FlutterDelay` rings
at all - ~480 samples of the previous bounce bleeding silently into the
next pass's punch-in, breaking the reused-equals-fresh property the
shipped REQ-902 fix established for tracks. Fixed by stating the full
per-pass sequence as a numbered sequence mirroring `reseed_chain`'s own
reset-then-reseed shape, with the invariant named: `StereoFlutter::reseed`
clears exactly the state `Flutter::reset` clears today, distributed
across three objects. Second, the REQ-408 test folded the metering
assertion into a setup that mutes tracks 1-4 - and `Mixer`'s meter
deliberately reads muted tracks as silent, so "meters not silent" could
never hold there, for a reason unrelated to REQ-408. Split into its own
test with unmuted tracks. Three medium findings: the dither-bound test
could pass vacuously on silence (muted tracks contribute zero to the
print sum too, and a fresh bus has no prior content - so the whole
measured pass was silence compared against silence, including the
crossfade windows whose exclusion it was meant to validate) - fixed by
priming the bus with a real first bounce, the same step REQ-403's
procedure already uses; the bus's `playback` slot was referenced
throughout but never in the allocation inventory (the same omission
class round 10 caught for the print sum, one step over) - resolved as a
second Engine-owned MAX_BLOCK L/R pair mirroring tracks'
`processed`/`playback` split, with `finish_mix` reading only the
playback pair; and `finish`'s retroactive punch-out rewrite was stated
as unconditional when the code skips it entirely at the tape end and
fades `min(XFADE_SAMPLES, len)`, not always the full window - the
derivation now says so and the test now ends short of the tape end.
Two minor: "settles well within that same opening window" overstated
the `g`-ramp margin (`SMOOTH_SAMPLES` and `XFADE_SAMPLES` are both
exactly 240 - coincident by construction with zero margin, now flagged
as a coupling that becomes false first if either constant moves), and
the RMS bound is now stated at the chosen -6dB fader (~0.25 LSB) rather
than leaving the un-scaled figure with silent 2x slack. The
`[f32; MAX_BLOCK]` type assertion (the codebase's per-block buffers are
`Vec`s, not fixed arrays - the third array-vs-Vec imprecision in as
many rounds) and the reviewer's closing nit (REQ-302's crossfade lives
on tape content only; the monitor slot is never faded, at either
boundary, for bus and ordinary tracks alike) are also folded in.

**v13 (this revision)**: the twelfth review returned the first
approving verdict - **APPROVE WITH NOTES** - after verifying every one
of round 11's seven fixes against the code line by line, independently
re-deriving the dither arithmetic and the undo transient, and running
the first fully clean consistency sweep over every numeric figure in
the document (the standing stale-value failure mode finally produced
nothing). Zero architectural findings remained; the reviewer stated
explicitly this should not go to a thirteenth review round. Two medium
notes folded in here: the bus chains' build site offered two options
and both were wrong (manifest-conditional build depends on arm state
the manifest deliberately never persists; lazy-on-arm runs on the
realtime thread since arming isn't blocking - the same REQ-902 mistake
in prose that rounds 9-10 fixed in code, caught one bullet over from
where round 11 caught its sibling) - resolved as: both split chains
build unconditionally at cassette open/create, exactly as `Engine`'s
constructor already builds all four track chains; and REQ-409's
fader/mute had no persistence story (save-and-reopen would silently
reset the bus to unity/unmuted - destructively load-bearing on the
next bounce, per this document's own muted-bounce analysis) - resolved
as `Manifest.bounce_fader_db`/`bounce_muted`, `#[serde(default)]`,
carried by `apply_to`/`capture_from`, the exact `Manifest::muted`
precedent. Three minor notes also folded in: the metering test's clean
measurable form (bus muted, so the audible output is exactly silent
while the meters read live); the dither test's measured region pinned
inside the primed one (its unprimed tail would have reintroduced the
vacuity priming eliminated); and the flush transient
(`write_payload_chunks`' per-chunk byte buffers, ~346MB worst case,
immaterial but named) added to REQ-904's inventory for completeness.
Per the reviewer's own closing line: fold in A and B, and this is
ready to implement. Awaiting owner sign-off.
