#!/bin/sh
# Toggles wvkbd (the on-screen keyboard), but only when no physical
# keyboard is currently attached - checked via udev's own
# ID_INPUT_KEYBOARD tag on every /dev/input/event* node, not a guess
# from a device/vendor list. If a real keyboard is present, this is a
# silent no-op: there's nothing to toggle.

for dev in /dev/input/event*; do
    if udevadm info --query=property --name="$dev" 2>/dev/null | grep -q '^ID_INPUT_KEYBOARD=1'; then
        exit 0
    fi
done

if pgrep -x wvkbd-mobintl >/dev/null 2>&1; then
    pkill -x wvkbd-mobintl
else
    wvkbd-mobintl &
fi
