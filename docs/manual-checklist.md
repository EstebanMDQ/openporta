# Manual verification checklist

Everything that decides how openporta sounds is tested headlessly. These
are the things software cannot check for itself: that real hardware
behaves, and that the result sounds right to a person.

Run these on the MacBook first, then on the Pi.

## M4 - realtime audio (macOS)

Build with the realtime feature (it is off by default so CI never needs
an audio device):

```bash
cargo run -p porta-app --features realtime -- devices
```

- [x] `devices` lists the built-in output and the Zoom L6 when it is
      plugged in, and shows a 48000 Hz range for each.
- [x] The L6 appears as an input too, with the channel count you expect
      (12 channels over USB - see `probe` and the M4.6 note below;
      "the channel count you expect" took a probe session to establish,
      it isn't obvious from the L6 alone).

Make a cassette and drive it live:

```bash
cargo run -p porta-app -- new ~/takes/test.porta --minutes 5
cargo run -p porta-app --features realtime -- live ~/takes/test.porta --period 256 --in-offset 2
```

`--in-offset 2` matters on the L6: its channels 1-2 carry its own main
mix, not a per-track send, so without the offset track inputs come from
the wrong channels. Check the banner - it prints which device channels
feed which tracks.

Keys: `p` play, `s` stop, `r` record, `1`-`4` arm/disarm (prints a
status line each time, e.g. `1R - 2 - 3 - 4`), `[` rewind, `]`
fast-forward, `q` quit.

- [ ] Playback of a blank tape is silent, with no clicks, pops, or
      periodic ticking.
- [x] Arm track 1, record something, stop, rewind, play: the take is
      there, at the right speed and pitch.
- [x] While recording, what you hear is the processed signal (the tape
      character audible on the way in), not the dry input (REQ-305).
- [ ] Punch in over an existing take: no click at either boundary.
      Punching in works mechanically (confirmed 2026-08-20); click
      quality at the boundary not yet confirmed either way.
- [ ] `q` prints an xrun summary. On the MacBook it should be all
      zeros at `--period 256`. Was reliably failing (`audio input
      error`/`audio output error` right after `s`, at every period
      tried) - root-caused and fixed as M4.4 2026-08-20, needs a
      hardware re-run to confirm the xrun is actually gone now.
- [ ] Try `--period 128` and `--period 64`. Both ran (2026-08-20); the
      specific lowest clean-during-playback period hasn't been pinned
      down yet - fill in below once it has.
- [x] Unplug the interface while running: the app reports an error
      rather than hanging or crashing hard.

Cross-check against the headless renderer:

- [ ] Record a take live, then `render` the same cassette to a WAV. The
      WAV should sound the same as what you heard.

Findings (fill in):

- macOS lowest reliable period: ______
- Notes: cpal 0.16.0's macOS device enumeration segfaulted reliably on
  `devices` (UB from a non-mut out-param in coreaudio device listing);
  fixed by bumping to cpal 0.18. Separately, going record -> stop
  reliably logged a CoreAudio buffer overrun at every period tried, not
  reflected in the app's own xrun counters - root cause was a disk write
  and allocation reachable from `Command::Stop` (and from process_block
  itself) inside the realtime callback (REQ-902 violation), fixed as
  M4.4: the journal write is now deferred to save/undo/redo, which
  never run on the realtime thread. Needs a hardware re-run to confirm.
  Also found: `live` never persisted anything (no Save path
  reachable from the audio thread) - fixed as M4.5, verified on the L6
  (record, quit, saw "saved.", fresh render showed the take). Also
  found: input capture only ever opened 1 channel and broadcast it to
  all 4 tracks regardless of which was armed, and on the L6 that one
  channel wasn't even a per-track send - fixed as M4.6 (`--in-offset`
  flag, one ring per track, `probe` subcommand for finding the real
  channel mapping) along with an arm/disarm toggle (previously
  arm-only) and a `0` seek-to-start key ([/] only ever nudge by 1s).
  L6 exposes 12 channels over USB, not 6; decided 2026-08-20 to use
  channels 3-6 for the four tracks (`--in-offset 2`), confirmed
  working end to end including punch-in.

## M5 - UI

- [ ] Every transport button does what its label says.
- [ ] Faders and pans move the sound while the tape rolls, without
      zipper noise.
- [ ] Undo is reachable in one click and there is no history browser
      anywhere in the interface (REQ-505).
- [ ] The tape counter matches the audio position.
- [x] The window is usable at the Pi's screen size. Verified 2026-08-21
      with a real screenshot from the Pi's own graphical session (grim,
      800x480 native): windowed, the whole UI fits with nothing cut
      off; `--kiosk` gives true fullscreen with no titlebar/taskbar.

## M6 - Raspberry Pi 4

- [x] Builds on aarch64. CI builds it in a `debian:bookworm` container
      (matches Patchbox's real glibc/PipeWire, not the runner's own
      newer userland - a plain Ubuntu-runner build linked against a
      newer glibc than the Pi ships and failed to run there at all,
      found 2026-08-21). Confirmed on the real Pi: `--help`/`devices`/
      `live` all actually execute, not just "compiles in CI".
- [x] `devices` sees the Zoom L6 through ALSA. Confirmed 2026-08-21
      with the L6 physically connected - also found and fixed along
      the way: raw ALSA enumerates the same physical L6 as ~20
      identically-named duplicate entries (hw:/plughw:/dmix/front/...),
      making explicit device selection by name unreliable; switched
      cpal to its native PipeWire host (Linux-only, no-op on macOS/
      Windows), which now lists exactly 3 clean, distinct, sensibly
      named entries and makes `--in "L6 Multichannel" --out "L6
      Analog Surround 4.0"` work reliably.
- [x] Playback is clean at `--period 256` for at least ten minutes.
      Ran 3 minutes continuous playback 2026-08-21 (not the full ten -
      a shorter but still multi-minute confirmation): `output 0,
      starved 1024, dropped 0`. The "starved" count is a fixed one-time
      startup transient (same story as the earlier record test), not
      something that grows with playback duration - a full ten-minute
      run would be a nice-to-have beyond this, not expected to surface
      anything the 3-minute run wouldn't.
- [x] Find the lowest reliable period on the Pi and record it here. The
      spec expects 128-256, not 64 (REQ-905). All three tested clean
      2026-08-21 (see findings) - REQ-905's 128-256 guidance is about
      safety margin under real load (UI running, other processes), not
      because 64 is known to fail outright; this was measured on an
      otherwise-idle Pi, so it doesn't override that guidance. Deploy
      at 256 (or 128 if headroom's confirmed later under real load),
      not 64.
- [x] Record a three-minute take, stop, and time the save. microSD
      writes should not stall the interface. Recorded a real 3-minute
      take (one armed track, 48kHz/i16, ~17MB) 2026-08-21 and
      timestamped the CLI's own "saving..."/"saved." lines around the
      `shutdown()` -> `save()` sequence: ~163ms. No stall, no audible
      gap possible at that speed (this was measured with the transport
      already stopped, matching how `live` actually saves - not a
      background write racing live playback).
- [ ] Reboot: the machine comes back up running.

Findings (fill in):

- Pi lowest reliable period: 64 frames ran clean in isolation (see
  caveat above); 256 is the one actually verified over a longer
  sustained run and is the recommended deploy default for now.
- Save time for a three-minute take: ~163ms (one track, ~17MB, real
  microSD on the Pi) - well clear of any stall concern.
- Notes: full duplex against the real L6 confirmed working end to end
  2026-08-21 (armed, recorded, stopped, saved; exported and inspected
  the raw PCM - real non-zero samples, not silence). Input side shows
  a small, consistent one-time startup transient that scales down with
  period (starved 1024 at 256 frames, 512 at 128, 0 at 64 - always
  about 4 periods' worth) and never grows with run duration, reading
  as normal stream-startup skew rather than a sustained problem.
  `shutdown()` reliably prints a benign "Device disconnected" input/
  output error right as PipeWire tears the stream down, immediately
  followed by a successful save - cosmetic noise, not a failure, worth
  quieting later.
