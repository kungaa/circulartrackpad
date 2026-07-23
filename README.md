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
git clone https://github.com/mikiobraun/circulartrackpad
cd circulartrackpad
./install.sh
```

The installer builds the release binary, copies it to
`/usr/local/bin/circulartrackpad`, and installs a udev rule granting the
active local-seat user access to the Panasonic trackpad and `/dev/uinput`.
The same rule classifies the gesture-only uinput device as a touchpad. No
`input` group membership is needed.

Log out and back in once if the new device ACLs are not immediately present.

## Configuration

Persistent settings use the XDG user configuration location:

```text
${XDG_CONFIG_HOME:-$HOME/.config}/circulartrackpad/config.toml
```

Run `./enable-autostart.sh` after installation. It creates a commented
default config only when the file does not already exist, installs an
argument-free systemd user unit, and restarts it so upgrades take effect.

```bash
./enable-autostart.sh
```

Edit `config.toml`, then validate and apply it with:

```bash
circulartrackpad restart
```

Invalid configuration is reported without stopping the currently running
service. A missing config is valid and uses built-in defaults.

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

Shortcut arrays use Linux `KEY_*` names in press order. This makes the
window-switching defaults replaceable for customized GNOME shortcuts.
Unknown fields, invalid values, unknown key names, duplicate keys, and empty
shortcuts are rejected.

Older `[button_gestures]` and `two_finger_swipe.up` settings remain accepted
for upgrade compatibility, but are ignored with a warning. Remove them after
upgrading.

Command-line options remain useful for temporary testing and override the
config:

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

Left, right, and middle physical buttons are always forwarded as ordinary
mouse buttons. Tap-to-click does not change their meaning.

One-finger outer-ring motion always scrolls, including while a physical
button is held. Set `tap = false` or pass `--no-tap` only when tap-to-click
itself is unwanted.

### Two-finger swipes

Start with two fingers in the inner zone:

- swipe left: next window (`Alt+Esc` by default)
- swipe right: previous window (`Shift+Alt+Esc` by default)
- swipe up/down: GNOME's progressive Activities gesture

Horizontal switching fires once after the centroid moves 80 raw units (about
6.7 mm). Vertical movement is translated into a native three-contact stream,
so the animation follows the fingers and GNOME decides whether to open or
close Activities. A short two-finger tap remains right-click.

### Three- and four-finger swipes

Three- and four-finger contacts take priority across the full pad and are
forwarded through the gesture-only virtual touchpad. On current GNOME Wayland,
horizontal swipes switch workspaces and vertical swipes control Activities.
Five-finger and interrupted sequences are ignored until every finger lifts.

Set `[native_gestures] enabled = false` to skip the gesture touchpad. Pointer,
ring, taps, physical buttons, and two-finger horizontal window switching
remain available, but native vertical/workspace gestures do not.

## How it works

Single touches are classified by where the primary finger begins. An
inner-zone touch produces relative pointer motion. A ring-zone touch converts
angular movement into high-resolution and legacy wheel events. The zone
remains locked until lift so drift cannot change modes unexpectedly.

The compositor sees normal pointer events from one virtual device and native
multitouch gesture sequences from a separate four-slot virtual touchpad.
Two-finger vertical motion is represented as three virtual contacts; real
three/four-finger positions retain stable virtual slots. The gesture device
is intentionally silent for taps, pointer movement, and ring scrolling, so
it cannot duplicate those actions.

Native gestures require GNOME on Wayland. There is no Xorg gesture fallback.

## Development and testing

On Windows, Ubuntu WSL2 can build and test the Linux code:

```bash
rustup toolchain install stable
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
bash -n install.sh enable-autostart.sh
```

WSL does not expose the Panasonic `/dev/input` device, so final gesture and
GNOME Wayland testing must be performed in the laptop's native Linux session.

## License

MIT — see [LICENSE](LICENSE).
