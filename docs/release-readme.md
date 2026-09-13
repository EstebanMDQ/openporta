# openporta

A software emulation of a 4-track cassette portastudio. This archive
holds one prebuilt program, `porta-app`, plus this file and the
licence.

## Open it

Double-click `porta-app`, or run it from a terminal with no arguments.
It opens a window on a cassette: if this is your first run it creates
one at `~/openporta/tape1` and opens that.

Before that works, macOS and Windows each need one step, because this
build is not code-signed. Both are below, and both are one-time.

### macOS: let it run

macOS marks anything downloaded with a quarantine flag, and refuses to
run an unsigned program that carries it ("cannot be opened because the
developer cannot be verified"). Remove the flag:

```bash
xattr -d com.apple.quarantine ./porta-app
```

That is one line, it works on every macOS version, and it is what a
bare Unix executable like this one actually needs.

If you would rather not use a terminal: double-click it, let it be
refused, then open System Settings > Privacy & Security, scroll to the
message about `porta-app`, and click **Open Anyway**.

(Older instructions on the internet say to right-click and choose Open.
That is the gesture for an `.app` bundle, and Apple removed it for
quarantined downloads in macOS 15. It will not help here.)

### Windows: let it run

Windows marks downloads the same way and SmartScreen shows "Windows
protected your PC". Click **More info**, then **Run anyway**.

The equivalent of the macOS command, in PowerShell:

```powershell
Unblock-File .\porta-app.exe
```

If you extracted the archive with File Explorer, the mark is copied
onto every extracted file, so it can be easier to run `Unblock-File` on
the `.zip` before extracting it.

### About the signature

These builds are **unsigned**. There is no Apple Developer ID
signature and no notarization, and no Windows code-signing
certificate. That is a cost decision, not a statement about the code.

What it means: your operating system cannot confirm who built this, so
it asks you. What it does not mean: nothing above disables any security
protection permanently, and each step applies to this one file.

If you would rather not take our word for it, the source and the exact
workflow that produced these binaries are public, and you can build
your own from the repository linked at the bottom.

## What it is

Four mono tracks and one stereo master, and never any more. Recording
over a track erases it, the way tape does.

Tape character is printed at record time, so every bounce saturates,
dulls and wobbles the material again and the noise floor climbs. Three
generations sound like three generations.

There is an undo, because losing a take to a mis-click is not the part
of tape worth emulating.

## Connect an interface

Getting sound in is the first real obstacle after launching. Interfaces
do not always order their channels the way you would expect, so do not
guess: ask.

```bash
./porta-app probe
```

Play into one input at a time and read off which channel number lights
up. Then assign those channels to the four tracks, in order, 1-based:

```bash
./porta-app live ~/openporta/tape1 --in-map 3,4,5,6
```

A `-` leaves a track with no input (`--in-map 3,-,5,6`). The example
above is a real one: on a Zoom L6, channels 1 and 2 carry its own main
mix rather than per-track sends, so the four tracks come off channels
3 to 6.

You only have to work this out once per interface. A connection that
succeeds is remembered, per input device, in
`~/.config/openporta/audio.json`, and reused the next time that device
is picked. Pass a flag explicitly to override what was remembered.

## Where cassettes live

A cassette is a directory, not a file, and the default one is
`~/openporta/tape1` (`%USERPROFILE%\openporta\tape1` on Windows).

```
tape1/
  manifest.json          tape length, character and seed, mixer settings
  tape/track{0..3}.raw   raw 16-bit samples, saved in 5-second chunks
  tape/bounce_{l,r}.raw  the stereo bounce bus, same format
  undo/                  the journal that makes undo possible
```

Copy the directory to back a cassette up. The app never writes while
the tape is rolling.

## The command line

The window is not the only way in. `./porta-app --help` lists
everything; the short version:

```bash
./porta-app new mytape --minutes 5     # make a cassette
./porta-app ui mytape                  # open one in the UI
./porta-app render mytape --out mix.wav --bits 24
```

`render` also writes `.mp3` (to share) and `.mp4` (the mix plus one
still image, for uploading somewhere that wants a video; needs ffmpeg
installed separately).

## Honest limits

Things this build does not do, stated plainly rather than discovered:

- **macOS**: launching from Finder opens a Terminal window alongside
  the app. This is a bare Unix executable, not an `.app` bundle, so it
  also has no icon and does not install into Applications. The
  quarantine step above stays manual; only notarization would remove
  it.
- **Windows**: a console window opens behind the app, because the same
  binary serves the command line.
- **Linux**: most file managers will not launch a bare executable by
  double-clicking at all. Run `./porta-app` from a terminal. No
  `.desktop` file is included in this archive.

## More

- Documentation: https://estebanmdq.github.io/openporta/
- Documentación en español: https://estebanmdq.github.io/openporta/es/
- Source, issues and the build workflow:
  https://github.com/EstebanMDQ/openporta
