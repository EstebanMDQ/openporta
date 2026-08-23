---
title: openporta
description: A software emulation of a 4-track cassette portastudio - four mono tracks, a destructive workflow, and real generation loss.
date: 2026-08-22
---

***English** · [Español](es/index.html)*

# openporta

A software emulation of a 4-track cassette portastudio, written in Rust.

Four mono tracks. One stereo master. A fixed, small control set. A
destructive workflow: recording over a track erases it, and bouncing
three tracks down to one costs you a generation of tape hiss and
wobble, same as it always did on the real thing. The constraint is the
point. This is an instrument, not a DAW, and it is not trying to grow
into one.

![The mixer view, running on a Raspberry Pi with a real Zoom L6 audio interface connected](images/mixer.png)

## Why

Four-track cassette portastudios forced a kind of commitment that
modern software makes easy to avoid: undo is expensive, tracks are
scarce, and every bounce genuinely degrades the material. That
scarcity was never a limitation musicians tolerated - it was part of
the instrument. openporta rebuilds that instrument in software, for
people who want the sound and the workflow without maintaining
40-year-old hardware, rather than building another endlessly flexible
DAW.

## What's actually in it

- **Four mono tracks, one stereo master. No more, ever.** Not
  configurable. Adding a track isn't a feature request, it's a
  different product.
- **Destructive recording**, the same way tape always was. Punch-in and
  punch-out get a real 5ms crossfade so it never clicks, but what gets
  overwritten is genuinely gone from the tape - only a hidden undo
  journal, not a visible history, gets it back.
- **Real generation loss.** Every record pass - including a bounce -
  runs through a full lo-fi signal chain before it hits (virtual) tape:
  saturation, a narrowed 11kHz top end, wow and flutter that
  decorrelates between passes so successive generations don't just
  repeat the same wobble, and hiss seeded inside the passband so it
  accumulates the way real analog noise does. Bounce three generations
  and you can hear three generations - that's the actual acceptance
  test the engine has to pass, not a nice-to-have.
- **Per-track mute and input monitor**, independent of arm - check a
  level or audition a mic before committing to it, silence a track
  without touching its fader.
- **A tape position bar, meters, and mixer that fit a small touchscreen**
  without a mouse. Tactile, high-contrast controls sized for fingers,
  not pointer precision.

Deliberately out of scope: MIDI, network sync, plugins, variable track
counts, non-destructive editing, anything that starts turning this back
into a DAW. See the project's own settled specification if you want the
exact, enforced boundary.

## Where it runs

The engine itself doesn't know audio hardware exists - it's pure
buffers in, buffers out, which is what makes it possible to test
without a sound card and to run identically on a laptop or a
[Raspberry Pi](raspberry-pi.md) booting straight into a dedicated,
kiosk-mode instrument. See [Getting started](getting-started.md) for
the CLI and session-script path, or the [Raspberry Pi](raspberry-pi.md)
page for what it looks like as a real, dedicated piece of hardware.

## More

- [Getting started](getting-started.md) - running it, the CLI, session
  scripts.
- [How it's built](architecture.md) - the engine, the DSP chain,
  realtime safety, how correctness gets tested without a listener.
- [Raspberry Pi setup](raspberry-pi.md) - kiosk mode, the on-screen
  keyboard, the icons, and what it took to get all of it working on
  real hardware.
- [Status](status.md) - what's done, what's in progress, and how
  changes to settled decisions get made.
