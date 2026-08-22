---
title: Status
description: What's done, what's in progress, and how a change to a settled decision actually gets made on this project.
date: 2026-08-22
---

# Status

## Milestones

| Milestone | State |
|-----------|-------|
| Scaffolding, CI, test instruments | done |
| Tape engine: transport, record, punch, undo, persistence | done |
| Lo-fi DSP and generation loss | done |
| Bounce, mixdown, WAV export, CLI | done |
| Realtime audio (cpal) | verified on macOS and Raspberry Pi hardware |
| Slint UI: transport, track strips (arm/mute/monitor/fader/pan), meters, tape position bar, save/undo, cassette management, export, real audio | done |
| Raspberry Pi deployment | in progress - see below |

The Raspberry Pi milestone covers a lot of ground already: aarch64
build, the ALSA/PipeWire device layer, full-duplex record/save,
remembered-device auto-connect, kiosk auto-launch with taskbar and
desktop icons, an on-screen keyboard, and autosave-on-stop are all
verified on real hardware. A formal callback-time performance
measurement is the piece still open - see [Raspberry Pi
setup](raspberry-pi.md) for the detail.

## A real bug, found and fixed this cycle

Recording briefly allocated memory on the realtime audio callback
thread - the project's own specification explicitly forbids that,
since an allocation can block unpredictably and the audio thread has a
hard deadline every callback. It was fixed by capturing displaced
audio in small, pre-reserved chunks instead of one large allocation
sized to the whole remaining tape. A second, related bug (an eviction
path silently dropping those chunks instead of returning them) was
found afterward by an adversarial review of an unrelated proposal, and
fixed with its own regression test.

## Changing a settled decision

This project treats its specification as a constitution: the number of
tracks, the destructive workflow, the DSP chain - none of it gets
casually "improved" in passing. Reversing or extending a settled,
user-visible decision requires a written proposal and a pass by an
adversarial spec review before any implementation begins.

A proposal for a dedicated, real-time-printed stereo bounce buss - to
fix two real limitations of today's bounce (it collapses stereo
information to mono, and it's one-shot, so a second bounce silently
discards the first) - is a live example of that process working as
intended rather than as a formality. It has been through several
rounds of review. Each round has found something real: a genuine
design flaw in an early version of the destination storage, a realtime
allocation risk in how a stereo pass would capture undo data, a
mathematically inverted rule for what you'd hear while bouncing, and a
resident-memory estimate that had to be recomputed more than once
against the code as it actually shipped, not as it was assumed to
work. None of it has been rubber-stamped through, and none of it ships
until a review comes back clean.

That's slower than just writing the feature. It's also exactly the
tradeoff a project like this is supposed to make.
