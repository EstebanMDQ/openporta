---
title: How it's built
description: Crate layout, the DSP chain, realtime safety, and how audio correctness gets tested without a listener.
date: 2026-08-22
---

# How it's built

## The crates

```
crates/porta-dsp/      tape character: saturation, bandwidth, flutter, hiss
crates/porta-engine/   tape, transport, record passes, undo, mixer, projects
crates/porta-testkit/  test instruments: generators, meters, FFT, click detector
crates/porta-app/      CLI, session scripts, WAV export, realtime adapter, Slint UI
```

The dependency direction only ever goes one way: `porta-dsp` knows
nothing above it, `porta-engine` depends on `porta-dsp` but nothing
about hardware or a UI, and `porta-app` is the only crate allowed to
know cpal or Slint exist. The engine's own API is buffers in, buffers
out - no notion of a sound card, a window, or a thread anywhere inside
it.

That's not an abstraction exercise for its own sake. It's what makes
the whole engine testable without a sound card in CI, and what let
[the Raspberry Pi build](raspberry-pi.md) work the first time it ran
against real hardware - the engine itself never noticed it had moved
platforms.

## The record path

A "record pass" is the unit of both recording and undo: one continuous
engagement on one armed track, from punch-in to punch-out. Before any
sample gets overwritten, the tape content it displaces is captured, so
undo can restore the region byte-exactly - and the cost of that
capture is proportional to what was actually recorded, not to the
length of the tape.

Every pass runs the full character chain before quantization to 16-bit,
in order: saturation (tanh, with drive and makeup gain), bandwidth
limiting (a lowpass near 11kHz, a highpass near 60Hz), wow and flutter
(a modulated fractional delay, re-seeded per pass so successive
generations don't share one coherent wobble), hiss (seeded, filtered
noise printed inside the passband), and an optional bitcrush stage,
off by default. TPDF dither is applied at the very end, immediately
before quantization. None of this is cosmetic - it's what makes three
generations of a bounce measurably, not just plausibly, duller and
noisier than two, which is the actual acceptance test the DSP chain has
to pass.

## Realtime safety

The audio callback - the actual function cpal calls on its own
real-time thread - has one non-negotiable rule: no allocation, no
locking, no disk I/O, ever, on that thread. Control messages (arm,
fader, transport commands) cross a wait-free queue instead of touching
the engine directly.

This isn't a rule that only lives in a document. It's been the subject
of two real bugs found and fixed on this project: recording briefly
allocated a buffer sized to the whole remaining tape on that thread
(fixed by capturing displaced audio in small, pre-reserved chunks
instead of one large on-demand allocation), and a subsequent eviction
path was found to silently drop those chunks back to the heap instead
of returning them to the reserve - found by an adversarial spec review
of an unrelated proposal, fixed the same way the first one was: with a
regression test that fails against the old behavior.

## Testing without a listener

Audio correctness is verified by rendering offline and measuring, not
by ear:

- RMS level in dBFS, in fixed windows
- band energy via FFT, to check that a lowpass actually rolled off
  where it's supposed to
- total harmonic distortion, to check saturation actually distorts
  and by how much
- pitch deviation in cents, to check wow/flutter's depth
- a click detector, tuned to catch discontinuities across punch
  boundaries that no human listener happens to be present for

One golden render pins the exact sound of a full, scripted session
end to end. If it changes, something in the signal path changed, and
the reason has to be understood and written down before the new
render gets blessed as correct - never the other way around.

## Changing a settled decision

The product's shape - four tracks, destructive recording, the specific
DSP chain - is written down as a formal specification, and it's treated
as a constitution, not a suggestion. Anything that would reverse a
settled, user-visible decision requires a written proposal and an
adversarial review before a line of implementation gets written - not
a formality, either: a proposal for a stereo bounce buss has been
through several rounds of real review, each one finding an actual bug
or an actual gap, none of them rubber-stamped through. See
[Status](status.md) for where that stands.
