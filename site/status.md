---
title: Status
description: What's done, what's in progress, and how a change to a settled decision actually gets made on this project.
date: 2026-08-22
---

***English** · [Español](es/status.html)*

# Status

## Milestones

| Milestone | State |
|-----------|-------|
| Scaffolding, CI, test instruments | done |
| Tape engine: transport, record, punch, undo, persistence | done |
| Lo-fi DSP and generation loss | done |
| Bounce, mixdown, WAV/MP3/MP4 export, CLI | done |
| Realtime audio (cpal) | verified on macOS and Raspberry Pi hardware |
| Slint UI: transport, track strips (arm/mute/monitor/fader/pan), meters, tape position bar, save/undo, cassette management, export, real audio | done |
| Raspberry Pi deployment | in progress - see below |
| Stereo bounce bus (change 001) | done - shipped in v0.1.0 |

The Raspberry Pi milestone covers a lot of ground already: aarch64
build, the ALSA/PipeWire device layer, full-duplex record/save,
remembered-device auto-connect, kiosk auto-launch with taskbar and
desktop icons, an on-screen keyboard, and autosave-on-stop are all
verified on real hardware. A formal callback-time performance
measurement is the piece still open - see [Raspberry Pi
setup](raspberry-pi.md) for the detail.

## Real bugs, found and fixed

Three separate violations of the project's own realtime rule - no
allocation on the audio callback thread, ever - have been found and
fixed here, none of them by a crash:

- Recording briefly allocated a buffer sized to the whole remaining
  tape on that thread. Fixed by capturing displaced audio in small,
  pre-reserved chunks instead.
- An eviction path silently dropped those chunks back to the heap
  instead of returning them to the reserve, which both deallocated on
  the audio thread and leaked the reserve over time.
- Engaging recording rebuilt the entire DSP chain from scratch - four
  or five heap allocations - every single time. This one had been
  shipping unnoticed for months.

The second and third were found by adversarial reviews of a proposal
that had nothing to do with them. Each was fixed with a regression test
that fails against the old behavior.

The rule is no longer argued structurally, either. A test-only counting
global allocator now measures the realtime path directly, and the first
time it ran it found four more allocations that careful reasoning had
missed - a pass object rebuilt per take, and three separate places
where handing a container away lost the capacity the next take needed.
The path is now measured at zero allocations and zero deallocations,
for both an ordinary record pass and a bounce.

## Changing a settled decision

This project treats its specification as a constitution: the number of
tracks, the destructive workflow, the DSP chain - none of it gets
casually "improved" in passing. Reversing or extending a settled,
user-visible decision requires a written proposal and a pass by an
adversarial spec review before any implementation begins.

The proposal that replaced the original bounce with a dedicated,
real-time-printed stereo bounce bus is the clearest example of that
process working as intended rather than as a formality. It fixed two
real limitations: the old bounce collapsed stereo information to mono,
and it was one-shot, so a second bounce silently discarded the first.

It took **twelve rounds of review across thirteen revisions** before it
was approved. Every round but the last found something real: a design
flaw in an early version of the destination storage, a realtime
allocation risk in how a stereo pass would capture undo data, a
mathematically inverted rule for what you'd hear while bouncing, a
resident-memory estimate that had to be recomputed more than once
against the code as it actually shipped rather than as it was assumed
to work, and - twice - a test specified in the proposal that could
never have passed as written. None of it was rubber-stamped through.

It is now fully implemented and shipped: a real-time stereo pass,
atomic two-channel undo, the bus folding its own prior content forward
so bounces layer instead of replacing, the master fader provably never
reaching tape, and a Bus strip in the UI with its own fader and mute.
Verified on a Raspberry Pi with a real interface, not just in tests.

That's slower than just writing the feature. It's also exactly the
tradeoff a project like this is supposed to make.
