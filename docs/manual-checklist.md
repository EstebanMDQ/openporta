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
- [ ] The window is usable at the Pi's screen size.

## M6 - Raspberry Pi 4

- [ ] Builds on aarch64.
- [ ] `devices` sees the Zoom L6 through ALSA.
- [ ] Playback is clean at `--period 256` for at least ten minutes.
- [ ] Find the lowest reliable period on the Pi and record it here. The
      spec expects 128-256, not 64 (REQ-905).
- [ ] Record a three-minute take, stop, and time the save. microSD
      writes should not stall the interface.
- [ ] Reboot: the machine comes back up running.

Findings (fill in):

- Pi lowest reliable period: ______
- Save time for a three-minute take: ______
- Notes: ______
