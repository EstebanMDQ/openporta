#!/bin/sh
# Wraps the kiosk launch to also suppress the desktop panel while
# openporta is running - kiosk mode switched from Slint's full-screen
# to maximized (see main.slint) so the on-screen keyboard can render
# above it, but maximized respects the panel's reserved space instead
# of covering it the way full-screen used to. This restores that
# "nothing else on screen" feel without giving up the keyboard.
#
# Purely additive and reversible: freezes the *existing* per-user
# lwrespawn supervisor (started by labwc's own autostart) rather than
# touching any system file, and restores it - which relaunches the
# panel fresh - the moment this script exits for any reason, including
# a crash. See docs/pi-setup.md.

restore_panel() {
    lw=$(pgrep -f 'lwrespawn /usr/bin/wf-panel-pi' | head -1)
    [ -n "$lw" ] && kill -CONT "$lw" 2>/dev/null
}
trap restore_panel EXIT INT TERM

lw=$(pgrep -f 'lwrespawn /usr/bin/wf-panel-pi' | head -1)
if [ -n "$lw" ]; then
    kill -STOP "$lw" 2>/dev/null
    pkill -x wf-panel-pi 2>/dev/null
fi

# Not `exec` - this script's own process has to stay alive (as
# porta-app's parent) for the EXIT trap above to run once porta-app
# quits, which is what actually restores the panel.
"$HOME/openporta/bin/porta-app" ui "$HOME/openporta/tape1" --kiosk
