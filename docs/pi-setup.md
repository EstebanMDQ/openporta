# Pi setup: kiosk auto-launch (M6.3)

Boots the Pi's desktop straight into openporta, full-screen, no window
chrome, instead of a manual launch every time.

## Assumed layout

This assumes the deployment convention used throughout M6:

```
~/openporta/bin/porta-app   # the aarch64 release binary
~/openporta/tape1           # the cassette to open on launch
```

Adjust `deploy/openporta-kiosk.desktop`'s `Exec=` line if your install
uses different paths.

## Install

The Pi's desktop (Raspberry Pi OS's "LXDE-pi-labwc" session) already
processes standard [XDG autostart][xdg-autostart] entries via
`lxsession-xdg-autostart` - `/etc/xdg/labwc/autostart` runs it as part
of its own startup. Dropping a `.desktop` file into
`~/.config/autostart/` is the standard, additive, per-user way to hook
into that - no system file is touched, so it can't be clobbered by an
OS update and needs no `sudo`.

```bash
mkdir -p ~/.config/autostart
cp deploy/openporta-kiosk.desktop ~/.config/autostart/
```

Takes effect on the next login/reboot. The 3-second delay (both in the
`Exec=` line itself and via `X-GNOME-Autostart-Delay`, in case the
autostart runner doesn't honor the latter) gives the panel and
PipeWire time to finish coming up first.

[xdg-autostart]: https://specifications.freedesktop.org/autostart-spec/autostart-spec-latest.html

## Getting out of kiosk mode

`--kiosk` removes every bit of window chrome (no titlebar, no close
button), so two ways back out:

- **Escape** toggles it off from the keyboard - the window returns to
  normal size and decoration. Pressing it again does *not* re-enter
  kiosk mode (that only happens via `--kiosk` at launch); use the
  window manager or restart the app to go back.
- **Over ssh**, from another machine: `pkill -f "porta-app ui"`. Always
  works regardless of what has focus locally - this is the reliable
  fallback if a keyboard isn't handy or the app has wedged.

## Disabling autostart

Delete the file, or add a line to make it inert without deleting it:

```bash
rm ~/.config/autostart/openporta-kiosk.desktop
# or, to keep it around but turn it off:
echo 'X-GNOME-Autostart-enabled=false' >> ~/.config/autostart/openporta-kiosk.desktop
```

## Not covered here

Reboot-to-confirm (does the Pi actually come back up running this,
not just "does the file look right") needs an actual reboot of the
hardware - do that by hand when convenient rather than as a background
task. Performance headroom under this launch path (M6.2) and a real
microSD save-timing check under sustained kiosk use are still open.
