# circulartrackpad

![A photo of the trackpad on my Panasonic laptop.](trackpad.jpeg)

A userspace daemon for the circular Panasonic Let's Note trackpad. It
restores outer-ring scrolling under Wayland and adds compact gestures suited
to the unusually small 44 mm pad:

- inner-disc pointer movement
- outer-ring scrolling
- one-finger left-click and two-finger right-click taps
- ordinary passthrough for all physical buttons
- two-finger horizontal window switching
- native GNOME Activities and workspace swipes

The daemon exclusively grabs the real `evdev` device and creates virtual
pointer, shortcut-keyboard, and gesture-only touchpad devices through
`uinput`.

## Hardware

Developed and tested on a Panasonic Let's Note with a **Synaptics
TM3562-003** touchpad (HID `06cb:cdea`, coordinates `0..528`). Other models
may require changes to the vendor/product IDs in `install.sh` and the
geometry constants in the driver.

## Install

Requires stable Rust and a Linux system with `udev`, `uinput`, and systemd:

```bash
git clone https://github.com/kungaa/circulartrackpad
cd circulartrackpad
./install.sh
./enable-autostart.sh
```

`install.sh` builds and installs the binary and udev rules.
`enable-autostart.sh` creates the config and starts the systemd user service.
Log out and back in if the device permissions do not apply immediately.

### Upgrading

From an existing checkout:

```bash
git pull
./install.sh
./enable-autostart.sh
```

## Configuration

Persistent settings use the XDG user configuration location:

```text
${XDG_CONFIG_HOME:-$HOME/.config}/circulartrackpad/config.toml
```

`enable-autostart.sh` preserves an existing config when upgrading.

Edit `config.toml`, then validate and apply it with:

```bash
circulartrackpad restart
```

Invalid configuration does not stop the running service. A missing config
uses built-in defaults.

```toml
pointer = 1.5
scroll = 5.0
ring = 0.65
invert_scroll = false
tap = true
tap_timeout_ms = 180
tap_move_threshold = 20

[two_finger_swipe]
enabled = true
distance = 80
left = ["KEY_LEFTALT", "KEY_ESC"]
right = ["KEY_LEFTSHIFT", "KEY_LEFTALT", "KEY_ESC"]

[native_gestures]
enabled = true
```

Shortcut arrays use Linux `KEY_*` names in press order.

Older `[button_gestures]` and `two_finger_swipe.up` settings remain accepted
for upgrade compatibility, but are ignored with a warning. Remove them after
upgrading.

Command-line options override the config:

```text
circulartrackpad [OPTIONS]
circulartrackpad restart

  -d, --device <DEVICE>             Input device path [default: auto-detect]
  -p, --pointer <POINTER>           Pointer sensitivity
  -s, --scroll <SCROLL>             Scroll ticks per radian
  -r, --ring <RING>                 Ring threshold fraction (0.0-1.0)
  -i, --invert-scroll               Invert scroll direction
      --no-invert-scroll            Do not invert scroll direction
      --tap                         Enable tap-to-click
      --no-tap                      Disable tap-to-click
      --tap-timeout <MILLISECONDS>  Tap timeout
      --tap-move-threshold <UNITS>  Tap movement threshold
```

## Gestures

### Physical buttons and ring scrolling

Physical buttons pass through unchanged. Outer-ring motion scrolls, including
while a button is held.

### Two-finger swipes

Start with two fingers in the inner zone:

- swipe left: next window (`Alt+Esc` by default)
- swipe right: previous window (`Shift+Alt+Esc` by default)
- swipe up/down: GNOME's progressive Activities gesture

Horizontal switching fires after the configured distance. Vertical movement
drives GNOME's native gesture animation. A short two-finger tap remains
right-click.

### Three- and four-finger swipes

On GNOME Wayland, three- and four-finger horizontal swipes switch workspaces;
vertical swipes control Activities. Set `native_gestures.enabled = false` to
disable them.

## How it works

The daemon grabs the physical evdev device and creates separate uinput
pointer, shortcut-keyboard, and gesture-touchpad devices. A touch that starts
in the inner zone moves the pointer; one that starts on the ring scrolls.

Native gestures require GNOME on Wayland. There is no Xorg gesture fallback.

## Troubleshooting

```bash
systemctl --user status circulartrackpad.service
journalctl --user -u circulartrackpad.service -n 30 --no-pager
libinput list-devices
```

The physical device still appears under `hid-rmi`/libinput; this is normal.
`circulartrackpad gestures` should be listed as a touchpad. Log out and back
in if the service reports a device permission error.

## Development and testing

The project uses Rust 2021 and stable Rust:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
bash -n install.sh enable-autostart.sh
```

## Acknowledgements

Based on the original [mikiobraun/circulartrackpad](https://github.com/mikiobraun/circulartrackpad)
and the fork [b3r/circulartrackpad](https://github.com/b3r/circulartrackpad).

## License

MIT — see [LICENSE](LICENSE).
