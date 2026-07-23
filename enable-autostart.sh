#!/usr/bin/env bash
# Install and enable the circulartrackpad systemd user service.
#
# Persistent settings live in:
#   ${XDG_CONFIG_HOME:-$HOME/.config}/circulartrackpad/config.toml
#
# Edit that file and run `circulartrackpad restart` to apply changes.
set -euo pipefail

BIN="/usr/local/bin/circulartrackpad"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CONFIG_HOME="${XDG_CONFIG_HOME:-$HOME/.config}"
CONFIG_DIR="${CONFIG_HOME}/circulartrackpad"
CONFIG_PATH="${CONFIG_DIR}/config.toml"
UNIT_DIR="${CONFIG_HOME}/systemd/user"
UNIT_PATH="${UNIT_DIR}/circulartrackpad.service"

if (($# != 0)); then
    echo "error: enable-autostart.sh no longer accepts daemon arguments." >&2
    echo "Edit ${CONFIG_PATH}, then run: circulartrackpad restart" >&2
    exit 2
fi

if [[ ! -x "${BIN}" ]]; then
    echo "error: ${BIN} not found. Run ./install.sh first." >&2
    exit 1
fi

mkdir -p "${CONFIG_DIR}" "${UNIT_DIR}"

if [[ ! -e "${CONFIG_PATH}" ]]; then
    echo "==> Writing default configuration ${CONFIG_PATH}"
    cat > "${CONFIG_PATH}" <<'EOF'
# circulartrackpad user configuration
# CLI options override values in this file.

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
EOF
else
    echo "==> Preserving existing configuration ${CONFIG_PATH}"
fi

echo "==> Writing ${UNIT_PATH}"
install -m 0644 "${SCRIPT_DIR}/circulartrackpad.service" "${UNIT_PATH}"

echo "==> Reloading systemd user daemon"
systemctl --user daemon-reload

echo "==> Enabling and restarting circulartrackpad.service"
systemctl --user enable circulartrackpad.service
systemctl --user restart circulartrackpad.service

echo
echo "Configuration: ${CONFIG_PATH}"
echo "Apply future changes with: circulartrackpad restart"
echo
echo "Status:"
systemctl --user --no-pager status circulartrackpad.service || true
