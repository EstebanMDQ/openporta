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

- [ ] `devices` lists the built-in output and the Zoom L6 when it is
      plugged in, and shows a 48000 Hz range for each.
- [ ] The L6 appears as an input too, with the channel count you expect.

Make a cassette and drive it live:

```bash
cargo run -p porta-app -- new ~/takes/test.porta --minutes 5
cargo run -p porta-app --features realtime -- live ~/takes/test.porta --period 256
```

Keys: `p` play, `s` stop, `r` record, `1`-`4` arm, `[` rewind, `]`
fast-forward, `q` quit.

- [ ] Playback of a blank tape is silent, with no clicks, pops, or
      periodic ticking.
- [ ] Arm track 1, record something, stop, rewind, play: the take is
      there, at the right speed and pitch.
- [ ] While recording, what you hear is the processed signal (the tape
      character audible on the way in), not the dry input (REQ-305).
- [ ] Punch in over an existing take: no click at either boundary.
- [ ] `q` prints an xrun summary. On the MacBook it should be all
      zeros at `--period 256`.
- [ ] Try `--period 128` and `--period 64`. Note the lowest period that
      still gives zero xruns over a two-minute run, and record it below.
- [ ] Unplug the interface while running: the app reports an error
      rather than hanging or crashing hard.

Cross-check against the headless renderer:

- [ ] Record a take live, then `render` the same cassette to a WAV. The
      WAV should sound the same as what you heard.

Findings (fill in):

- macOS lowest reliable period: ______
- Notes: cpal 0.16.0's macOS device enumeration segfaulted reliably on
  `devices` (UB from a non-mut out-param in coreaudio device listing);
  fixed by bumping to cpal 0.18. Separately, going record -> stop
  reliably logged a CoreAudio buffer overrun at --period 256, not
  reflected in the app's own xrun counters - root cause is a disk write
  and allocation reachable from `Command::Stop` inside the realtime
  callback (REQ-902 violation), tracked as M4.4 in TASKS.md. Re-run the
  period/xrun steps above once M4.4 lands.

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
