# Handoff

## Current state

The native GNOME gesture redesign is implemented in the working tree:

- XDG config at `${XDG_CONFIG_HOME:-$HOME/.config}/circulartrackpad/config.toml`
- `circulartrackpad restart` with validation before systemd restart
- plain passthrough for left/right/middle physical buttons
- two-finger left/right shortcuts for next/previous window
- progressive two-finger vertical GNOME Activities gestures
- native three/four-finger GNOME workspace and Activities gestures
- separate uinput pointer, shortcut-keyboard, and gesture-touchpad devices
- legacy gesture-config compatibility warnings

## Verification

The following Windows-side checks passed:

- `bash -n install.sh enable-autostart.sh`
- `git diff --check`

Rust formatting, tests, Clippy, and release build could not run because this
Windows environment currently has neither Cargo nor an installed WSL
distribution. Run those checks on Linux before installing. Real touchpad and
GNOME behavior still require the Panasonic laptop's native Linux session.

## Next steps

On the Panasonic laptop's native Linux installation:

```bash
git pull
./install.sh
./enable-autostart.sh
```

Then test ring scrolling with and without held buttons, the two-finger
left/right window swipes, progressive two-finger Activities up/down, native
three/four-finger workspace and Activities gestures, tap/right-tap behavior,
and physical buttons with both tap modes.
Tune `~/.config/circulartrackpad/config.toml` as needed and apply changes with:

```bash
circulartrackpad restart
```

Confirm that `libinput list-devices` classifies `circulartrackpad gestures`
as a touchpad before evaluating native gesture behavior.
