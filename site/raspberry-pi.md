---
title: Raspberry Pi setup
description: What it took to turn openporta into a real, dedicated instrument on a Raspberry Pi 4 - kiosk mode, real hardware, and the rough edges that came with both.
date: 2026-08-22
---

***English** · [Español](es/raspberry-pi.html)*

# Raspberry Pi setup

The engine and UI don't know they're running on anything in particular
- but making a Raspberry Pi 4 actually *feel* like a dedicated
instrument, instead of a Linux desktop that happens to run this app,
took real work beyond the audio path itself. This is that story.

![The desktop, with a cassette-shaped launcher icon](images/desktop-icon.png)

## The hardware path

Real-time audio runs through cpal against a class-compliant USB
interface - this project's own testing has been against a Zoom L6.
Getting a clean, repeatable connection took solving a problem that
wasn't really about audio at all: ALSA enumerates one physical
interface many times over (`hw:`, `plughw:`, `dmix`, `front`, ...),
all under nearly identical display names, with no reliable way to tell
them apart by name alone. PipeWire's own host talks to the same
hardware as a single real device instead, which is what actually made
device selection by name usable on this platform.

Once a device is chosen, the app remembers it - `~/.config/openporta/`
holds the last input/output device, period, and per-track input channel
map that connected successfully, and the app tries that combination
again automatically at every launch. It's meant to come on ready, the way a
real piece of hardware does, not sit idle waiting to be told what to
plug into.

## Kiosk mode

`--kiosk` removes every bit of window chrome and takes over the
screen, launched automatically on login via a per-user autostart entry
- nothing in `/etc` gets touched, so it survives an OS update and needs
no `sudo`. Escape gets back out of it from the keyboard; killing the
process over ssh always works regardless of what has focus locally,
which matters more than it sounds like it should the first time a kiosk
window won't respond to anything else.

A taskbar launcher and a hand-drawn cassette-shaped desktop icon exist
alongside the autostart entry, for launching it manually instead of
waiting for a reboot - windowed, not kiosk, since a manual launch
usually wants to still reach the rest of the desktop.

### The keyboard, and the panel it broke

Kiosk mode originally used the compositor's real fullscreen state, not
just a borderless maximized window - visually identical, but it turned
out to matter a great deal for what else can render on screen at the
same time. An on-screen keyboard (`wvkbd`, chosen because it's built
for exactly this kind of wlroots compositor rather than needing X11)
turned out to be completely invisible behind a true fullscreen kiosk
window - Wayland's layer-shell protocol defines the keyboard's own
layer as sitting *below* an exclusive fullscreen surface, by design,
not by bug.

The fix was to stop asking for that exclusive fullscreen state at all
and use a maximized, borderless window instead - identical to look at,
but no longer high enough in the surface stack to blot out a
layer-shell keyboard. That traded one problem for a smaller one:
maximized windows respect the desktop panel's reserved space instead
of covering it, so the taskbar came back too. The actual fix ended up
being two-sided - suppress the panel specifically while kiosk mode is
active (freezing its own supervisor process rather than touching any
system file), and restore it the instant kiosk mode ends, whether
that's the whole app closing or just pressing Escape.

The keyboard itself only shows up by an explicit toggle - not
automatically when a text field gets focus, since that would need the
app to speak Wayland's text-input protocol, and whether the underlying
toolkit actually does wasn't something worth gambling the whole feature
on. The toggle also checks first: if a real, physical keyboard is
already attached (detected the same way Linux itself tags one -
`ID_INPUT_KEYBOARD` on the input device, not a guess from a vendor
list), it does nothing. There's no reason to offer a redundant
on-screen keyboard over a real one.

## What's actually been verified on hardware, not just in theory

- Full-duplex record and save against a real interface, at a 256-frame
  period
- Device auto-connect surviving a real reboot
- Kiosk autostart surviving a real reboot
- The mixer view fitting the Pi's real 800x480 kiosk display without
  scrolling
- The on-screen keyboard rendering correctly above the kiosk window,
  and the desktop panel disappearing and reappearing cleanly around it

What's still open: a formal callback-time performance pass - measuring
actual headroom with a bounce or several armed tracks running
simultaneously at a 128-256 frame period, rather than assuming it fits.

## Everything else on this Pi

Patchbox OS ships with Pure Data, SuperCollider, Audacity, Patchage,
and a Pianoteq trial already on the desktop - openporta runs alongside
all of it as one more icon, not a replacement for the rest of the
audio-software desktop already there.
