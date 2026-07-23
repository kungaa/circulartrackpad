# circulartrackpad

![A photo of the trackpad on my Panasonic laptop.](trackpad.jpeg)

A userspace daemon for the circular Panasonic Let's Note trackpad. It
restores outer-ring scrolling under Wayland and adds compact gestures suited
to the unusually small 44 mm pad:

- inner-disc pointer movement
- outer-ring scrolling
- one-finger left-click and two-finger right-click taps
- two-finger upward swipe to toggle GNOME Activities
- physical-button plus ring navigation for windows and workspaces

The daemon exclusively grabs the real `evdev` device and creates virtual
pointer and keyboard devices through `uinput`.

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
No `input` group membership is needed.

Log out and back in once if the new device ACLs are not immediately present.

## Configuration

Persistent settings use the XDG user configuration location:

```text
${XDG_CONFIG_HOME:-$HOME/.config}/circulartrackpad/config.toml
```

Run `./enable-autostart.sh` after installation. It creates a commented
default config only when the file does not already exist, installs an
argument-free systemd user unit, and starts it.

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

[button_gestures]
step_degrees = 30.0
left_clockwise = ["KEY_LEFTALT", "KEY_ESC"]
left_counterclockwise = ["KEY_LEFTSHIFT", "KEY_LEFTALT", "KEY_ESC"]
right_clockwise = ["KEY_LEFTMETA", "KEY_PAGEDOWN"]
right_counterclockwise = ["KEY_LEFTMETA", "KEY_PAGEUP"]

[two_finger_swipe]
enabled = true
distance = 80
up = ["KEY_LEFTMETA"]
```

Shortcut arrays use Linux `KEY_*` names in press order. This makes the
defaults replaceable for customized GNOME shortcuts or another desktop.
Unknown fields, invalid values, unknown key names, duplicate keys, and empty
shortcuts are rejected.

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
      --no-tap                      Disable taps and restore physical buttons
      --tap-timeout <MILLISECONDS>  Tap timeout
      --tap-move-threshold <UNITS>  Tap movement threshold
```

## Gestures

### Ring navigation

With tap-to-click enabled, the two physical buttons become navigation mode
buttons rather than mouse buttons:

- `BTN_LEFT` + clockwise ring: next window
- `BTN_LEFT` + counterclockwise ring: previous window
- `BTN_RIGHT` + clockwise ring: workspace right
- `BTN_RIGHT` + counterclockwise ring: workspace left

The default GNOME shortcuts are `Alt+Esc`, `Shift+Alt+Esc`,
`Super+PageDown`, and `Super+PageUp`. One action is emitted per 30 degrees by
default. Button-first and ring-first operation both work. Holding both
buttons suppresses navigation.

Set `tap = false` or pass `--no-tap` to disable button-ring navigation and
restore ordinary physical left/right clicks. The physical middle button is
always forwarded.

### Two-finger Activities swipe

Place two fingers in the inner zone and move both upward. Once their centroid
has moved 80 raw units (about 6.7 mm on the supported pad), the daemon taps
`Super` to toggle GNOME Activities.

This is a discrete shortcut, not GNOME's progressive three-finger animation.
An upward swipe while Activities is already open therefore closes it.
Downward or primarily horizontal movement is ignored. A short two-finger tap
still emits right-click; movement between the tap and swipe thresholds emits
neither action.

The two-finger swipe remains available under `--no-tap`, but it is disabled
for a contact sequence that overlaps a held physical button.

## How it works

Touches are classified by where the primary finger begins. An inner-zone
touch produces relative pointer motion. A ring-zone touch converts angular
movement into high-resolution and legacy wheel events. The zone remains
locked until lift so drift cannot change modes unexpectedly.

The physical device reports five multitouch slots, but the compositor sees a
mouse-like virtual pointer rather than a touchpad. GNOME therefore cannot
recognize native three/four-finger gestures from it. The daemon recognizes
the compact gestures itself and emits configurable shortcuts from a separate
virtual keyboard.

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
