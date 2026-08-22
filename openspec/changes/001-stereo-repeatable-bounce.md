# 001: A dedicated stereo bounce buss, printed in real time

## Motivation

Requested directly by the owner while using the app, then substantially
reshaped after a first draft of this proposal was reviewed and found to
have real design holes (see "History" at the end). The underlying
problems are the same two as before:

1. **Stereo information is lost.** Today's bounce is a mono sum of
   tracks 1-3 onto track 4; anything panned comes out center.
2. **Bounce is one-shot.** A second bounce replaces track 4 with a fresh
   sum of 1-3, silently discarding the first submix. There's no way to
   free up tracks more than once without losing earlier work.

The first draft tried to solve both by turning tracks 3 and 4 into a
self-inclusive stereo destination. Review found that reuses two of the
four "real" tracks (so a bounce still only nets 2 free tracks, not the 3
a mono bounce gives you today), and doing the stereo sum by re-reading
track 3/4's own pan/mute knobs as if they were ordinary sources is a
genuine bug: the second bounce would silently collapse the stereo image
it just created (track 3's pan defaults to center; re-summing through it
spreads the previous left channel into both outputs).

**Revised approach, from the owner directly:** don't reuse tracks 3/4 at
all. Add a fifth, dedicated, always-stereo **bounce buss** - separate
storage, not one of the 4 mono tracks - that is always part of the
mix (like a permanent extra channel feeding the master) but can only
ever be *written* by bouncing, never armed for ordinary input recording.
Bouncing means: arm the bounce buss, press Record, and the transport
rolls in real time recording the current master output (tracks 1-4 at
whatever fader/pan/mute you're riding live, right now, plus the bounce
buss's own existing content, since it's already part of that mix) into
the bounce buss, replacing it as playback proceeds. This is explicitly
meant to be played, not computed: "we should just create a render in
realtime, so we can play with levels and panning while it bounces."

This is a materially different, and better, design than the first draft:

- All 4 tracks stay free, always - the bounce buss is additive, not
  consumed from the 4. No more "3 tracks vs 2 tracks" tradeoff.
- No separate pan/mute on the bounce buss re-entering the sum through
  the pan law a second time - it's written directly as the real stereo
  master output, so there's no collapse-on-second-bounce bug.
  It gets its own fader and mute (to blend it against new material
  during later bounces) but no pan (already stereo) and cannot be armed
  for input recording.
- One real-time stereo pass, not two independent mono record passes, so
  L and R naturally share the same wow/flutter (one transport doesn't
  wobble differently per channel) instead of decorrelating and smearing
  the image.
- Reuses the existing arm/record/ride-the-faders machinery almost
  entirely instead of being a special batch operation - which also
  closes the separate, already-known gap that bounce isn't reachable
  from the live UI or the interactive `live` session today.

## Change

### Storage

Add a fifth storage area to Tape: one stereo (2-channel) i16 buffer, the
cassette's fixed length, alongside the existing 4 mono track buffers.
This is new tape storage, not a reuse of an existing track, and it
increases resident memory (see Impact).

### Mix

The bounce buss is always summed into the master output, at its own
fader level (muteable, not panned - it's already stereo) alongside
tracks 1-4. This is true during ordinary playback, not just while
bouncing: once you've printed something to it, you hear it every time
you play the tape, the same as any track.

### Bouncing (the "print" pass)

- A new arm-like state exists for the bounce buss, separate from the 4
  tracks' arm state, and can be toggled independently of them (nothing
  stops arming a normal track and the bounce buss at the same time, if
  that's ever useful - not disallowed).
- With the bounce buss armed, Record engages a real-time pass whose
  input is not a hardware channel but the engine's own current stereo
  master mix - computed the same way monitoring and export already
  compute it, sample-accurate, at whatever fader/pan/mute values are
  live at each instant. Riding a fader during the pass changes what
  gets printed, on purpose.
- The pass runs through the character chain like any record pass
  (generation loss still compounds - REQ-402's intent, now printing a
  stereo signal): wow/flutter shared between L and R (one modulation
  instance, not two independent ones); hiss may still be seeded
  independently per channel for a natural stereo noise floor, since
  that doesn't smear imaging the way decorrelated pitch-wobble does.
- Punch-in/out, the 5ms crossfade, and undo apply the same way they do
  to any record pass (see "Undo" below for what "the same way" means
  precisely for a stereo destination).
- Because the bounce buss's own existing content is already part of the
  master mix being printed, a second bounce naturally folds the first
  one forward - no special self-referential summing code, no separate
  "read the destination as an extra source" logic. It's the same
  monitoring/mixdown path the engine already has, aimed at a
  record-armed destination instead of played back through speakers.

### What doesn't change

- Tracks 1-4 stay exactly as they are: 4 mono, armable, recordable,
  with fader/pan/mute/monitor. REQ-601-603 apply to them unchanged -
  REQ-603 (pans ignored) is **not** reversed for tracks 1-4's ordinary
  behavior; it only ever applied to the old bounce's summing, which no
  longer exists in this form.
- Export/WAV mixdown: unaffected in shape (still the engine's stereo
  master output around whatever's currently armed/muted/faded); the
  bounce buss just becomes one more thing already folded into that
  output when present.

## Requirements affected (settled decisions being reversed or extended)

- **Definition of "Bounce"** (section 3): from "mono sum of tracks 1-3
  onto track 4" to "a real-time record pass onto a dedicated stereo
  buss, whose input is the current master mix."
- **REQ-101** ("exactly 4 mono tracks"): the cassette gains a fifth,
  always-stereo storage area that is not one of the 4 tracks and has a
  different capability set (mix-only input, no arm for ordinary
  recording). REQ-101's 4-mono-track guarantee for tracks 1-4
  themselves is unchanged; this adds a new category alongside it.
- **REQ-401**: rewritten per "Change" above - no more fixed 1-3-onto-4
  mono sum.
- **REQ-402**: intent unchanged (character chain still applies, still
  compounds); wording updated for a stereo pass with shared flutter.
- **REQ-403**: the acceptance test's *procedure* needs re-verification
  under the new bounce (see "Impact on tasks" - this is the product
  acceptance test and must be re-proven, not assumed to still hold).
- **REQ-603**: no longer describes bounce at all (bounce isn't a
  fader/pan-driven sum of existing tracks anymore); tracks 1-4's own
  REQ-601/602 behavior is untouched.
- **Section 3 "Record pass"** definition ("one continuous record
  engagement on one track") needs to explicitly cover a pass onto the
  stereo bounce buss - see "Undo" below.

### Undo

A bounce pass writes two channels (L/R) of one buss, not one track. To
keep REQ-505's "no incoherent intermediate state" guarantee, a bounce
pass's undo entry must cover both channels atomically - one Undo press
fully reverts a bounce, not a half-reverted L or R. This is a real,
scoped change to the journal (REQ-501/502): either a single journal
entry spanning both channels of the bounce buss, or two entries that are
always pushed, capped, and undone/redone as a pair. Whichever shape,
"one bounce, one undo step" is the requirement, not the implementation
detail.

## Impact on tasks

- Tape storage gains a fifth (stereo) buffer; `RecordPass`/`Tape`
  read/write paths need a stereo-aware variant or to treat it as two
  correlated mono writes sharing one pass/chain lifecycle (for the
  shared-flutter requirement above).
- A new arm-like flag for the bounce buss, independent of the 4 tracks'
  `armed` array.
- `process_block`'s mix step gains the bounce buss as a fifth
  contributor (fader + mute, no pan) to the master sum, in both
  ordinary playback and while a bounce pass is running (its *own*
  contribution during a bounce is what's currently on tape, read the
  same way any track's playback is read, right up to the point being
  overwritten - same read-before-write ordering the engine already uses
  elsewhere for undo's displaced-content capture).
- Bounce's "input" becomes the master-mix computation itself, not a raw
  device/track input - this is a real restructuring of where in
  `process_block` a bounce pass's source signal comes from, not a
  parameter change to the existing bounce() function. The existing
  `Command::Bounce` (a blocking, stop-gated, whole-tape-at-once command)
  goes away entirely in favor of arm-the-buss + ordinary Play/Record,
  which also means bounce needs no special CLI/UI wiring beyond
  exposing the new arm toggle - it rides the transport controls that
  already exist.
- Undo: see "Undo" above - a scoped journal change, not a reinterpretation
  of existing per-track undo.
- REQ-403's test: needs a procedure that isolates what it's actually
  measuring. Because the bounce buss folds its own prior content forward
  every pass, repeatedly bouncing unchanged, unmuted source material
  re-injects full-bandwidth signal each time - the monotonic HF-loss /
  noise-floor-rise assertion needs the test to bounce, then mute (or
  silence) tracks 1-4 for the next two generations, so what's being
  re-printed is only the buss's own prior content aging against itself -
  not a re-run of the old test unchanged.
- New test: stereo image survives a bounce (a hard-panned source stays
  audibly on its side after printing) and survives a *second* bounce
  (no collapse toward center/crosstalk).
- New test: two destination channels of one bounce pass share flutter
  (correlated pitch wobble), not two independent LFOs.
- Gain staging: self-inclusive summing over many bounces can accumulate
  level. The realtime output clamp added earlier this session (mixer.rs)
  already bounds what reaches hardware; tape writes go through the
  existing dither/quantize clamp to i16 the same way any record pass
  does. Worth a test bounding peak level after several bounces of hot
  material, but no new clamping mechanism needed - both paths already
  exist and already apply to any record pass.
- REQ-904 (resident memory ceiling, ~700MB worst case at 30 minutes): a
  fifth buffer (stereo, so ~2 mono tracks' worth) adds roughly 50% more
  resident tape memory. Needs recomputing against the ~700MB ceiling -
  flagged, not yet resolved; may need the ceiling itself revisited if
  it's exceeded, which is its own small decision.
- `TASKS.md` M3.1 (bounce, currently `[x]`) and its verify text need
  updating; this is a re-open of a done milestone task, not a new one -
  worth calling out explicitly when this lands there.
- `openspec/spec.md` itself needs the affected REQs, the "Bounce" and
  "Record pass" definitions, and REQ-101's track-count language
  rewritten once this proposal is accepted.

## History

An earlier draft of this proposal reused tracks 3 and 4 as a
self-inclusive stereo destination (summing 1-3's-equivalent sources
through the existing pan law into a track pair, reading the pair's own
prior content back as an extra source before overwriting). Spec review
found it reversed more of the spec than it stated (REQ-602, REQ-304, the
"Record pass" definition, REQ-502's accounting), had a real bug (the
destination tracks' own default-center pan would collapse the stereo
image on the second bounce), decorrelated wow/flutter between the two
destination channels would smear the image even without that bug, and
overclaimed "unlimited layering" when every layer re-degrades everything
already printed. The owner's dedicated-buss redesign above avoids
essentially all of it by not reusing ordinary tracks as the destination
at all. Kept here for the record, not as a live alternative under
consideration.
