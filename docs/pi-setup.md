# Pi setup: kiosk auto-launch and a taskbar launcher (M6.3)

Two independent, complementary pieces:

- Boots the Pi's desktop straight into openporta, full-screen, no
  window chrome, instead of a manual launch every time.
- A pinned icon on the panel/taskbar to (re)launch it manually,
  windowed - e.g. after killing the kiosk instance to test something,
  or on a machine that isn't set to auto-launch at all.

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

## Taskbar launcher

wf-panel-pi (the Pi's own panel) pins launchers by listing `.desktop`
files in `~/.config/wf-panel-pi.ini`'s `[panel]` section
(`launcher_000001=...`, `launcher_000002=...`, ...) - each name is
resolved the standard way, against `~/.local/share/applications/` and
`/usr/share/applications/`. Same additive, per-user, no-`sudo`
approach as the autostart entry:

```bash
mkdir -p ~/.local/share/applications
cp deploy/openporta-launcher.desktop ~/.local/share/applications/openporta.desktop
```

Then add one line to the `[panel]` section of
`~/.config/wf-panel-pi.ini`, picking the next unused
`launcher_NNNNNN` number (they don't have to be contiguous, just
distinct):

```ini
launcher_000004=openporta.desktop
```

wf-panel-pi only reads its config at startup, so it needs to restart
to pick up the change - it runs supervised by `lwrespawn` (see
`/etc/xdg/labwc/autostart`), which relaunches it automatically the
moment it exits:

```bash
pkill wf-panel-pi
```

This one launches windowed (no `--kiosk`) - a manual launcher is more
useful when it can still see and reach the rest of the desktop, unlike
the autostart entry above whose whole point is to take over the
screen.

## Desktop icon

```bash
cp deploy/openporta-desktop-icon.desktop ~/Desktop/openporta.desktop
chmod +x ~/Desktop/openporta.desktop
gio set ~/Desktop/openporta.desktop metadata::trusted true
```

Both the `chmod +x` and the `gio set` matter - PCManFM treats a
`.desktop` file on the desktop as untrusted until both are true, and
prompts "Execute / Execute in terminal / Open" on every double-click
until it does. This was found and fixed 2026-08-22 after exactly that
prompt showed up on real hardware; without either step the icon still
*works*, it's just annoying every time.

## On-screen keyboard

```bash
sudo apt-get install -y wvkbd
cp deploy/toggle-osk.sh ~/openporta/bin/toggle-osk.sh
chmod +x ~/openporta/bin/toggle-osk.sh
cp deploy/openporta-osk-toggle.desktop ~/.local/share/applications/openporta-osk-toggle.desktop
chmod +x ~/.local/share/applications/openporta-osk-toggle.desktop
gio set ~/.local/share/applications/openporta-osk-toggle.desktop metadata::trusted true
```

wvkbd, not `onboard`/`matchbox-keyboard` - those are X11 and this
session is Wayland (labwc); wvkbd is a layer-shell keyboard built for
exactly this kind of wlroots compositor. Add it to the taskbar the same
way as the openporta launcher above, one more `launcher_NNNNNN=` line:

```ini
launcher_000005=openporta-osk-toggle.desktop
```

The launcher runs `deploy/toggle-osk.sh`, which toggles wvkbd on and
off - except it's a silent no-op if a physical keyboard is already
attached (checked via udev's `ID_INPUT_KEYBOARD` tag on every
`/dev/input/event*` node, not guessed from a device list), so the
button doesn't pop up a redundant on-screen keyboard when there's
nothing to compensate for.

Not done, and not attempted: getting wvkbd to show automatically when
a text field gets focus in the Slint UI. That needs the app to speak
Wayland's text-input protocol, which winit's (and therefore Slint's)
support for is uncertain - the manual toggle button is the reliable
path regardless of whether that would ever work.

**Kiosk mode uses `maximized`, not Slint's `full-screen`, specifically
so this works**: a true fullscreen surface sits above wlr-layer-shell's
"top" layer (which wvkbd uses), hiding it completely - confirmed
on-device, wvkbd rendered fine over the plain desktop but not over
openporta's kiosk window before this. Visually identical either way
(no-frame is still on, so it's still edge-to-edge with no window
chrome) - only the underlying Wayland surface state differs.

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
