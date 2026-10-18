#!/usr/bin/env bash
# Install the PC side: the server, its config and a systemd user service.
#
# The steps that need root (udev rule, modules, firewall) are printed as a
# block to review and run, or run for you with --sudo.
#
# usage: install-pc.sh [--sudo]
set -euo pipefail
# shellcheck source=lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

RUN_SUDO=0
case ${1:-} in
    --sudo) RUN_SUDO=1 ;;
    -h|--help) sed -n '2,8p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
    "") ;;
    *) die "unknown argument '$1' (see --help)" ;;
esac

BIN=deck-virtual-controller-server
server=$(get_binary "$BIN")

mkdir -p ~/.local/bin ~/.config/"$BIN" ~/.config/systemd/user
# Stop a running instance first so the binary can be replaced.
systemctl --user stop "$BIN" 2>/dev/null || true
install -m 755 "$server" ~/.local/bin/"$BIN"
if [[ -e ~/.config/$BIN/config.toml ]]; then
    echo "keeping existing config"
else
    install -m 644 "$ROOT/deploy/server/config.toml" ~/.config/"$BIN"/config.toml
fi
install -m 644 "$ROOT/deploy/$BIN.service" ~/.config/systemd/user/"$BIN".service
systemctl --user daemon-reload

root_steps=$(cat <<EOF
sudo install -m 644 "$ROOT/deploy/server/70-deck-virtual-controller.rules" /etc/udev/rules.d/70-deck-virtual-controller.rules
sudo install -m 644 "$ROOT/deploy/server/deck-virtual-controller.conf" /etc/modules-load.d/deck-virtual-controller.conf
sudo modprobe uhid uinput
sudo udevadm control --reload
sudo udevadm trigger --subsystem-match=misc
EOF
)

echo
echo "Steps that need root (device permissions, kernel modules):"
echo "$root_steps" | sed 's/^/    /'
if [[ $RUN_SUDO -eq 1 ]]; then
    echo "running them now (--sudo)..."
    bash -c "$root_steps"
fi
if ! id -nG | tr ' ' '\n' | grep -qx input; then
    echo
    echo "warning: you are not in the 'input' group. Run: sudo usermod -aG input $USER  (then log out and back in)"
fi
cat <<EOF

The Deck connects to UDP port 24800. If this PC has a firewall, open it, e.g.
    sudo ufw allow 24800/udp
    sudo firewall-cmd --permanent --add-port=24800/udp && sudo firewall-cmd --reload
EOF

# The device the server needs to write to (overridable for tests).
UHID=${UHID_DEVICE:-/dev/uhid}
if [[ -w $UHID ]]; then
    systemctl --user enable --now "$BIN"
    echo
    echo "Server running: systemctl --user status $BIN"
else
    systemctl --user enable "$BIN"
    echo
    echo "$UHID is not writable yet, so the service was enabled but not started."
    echo "Run the root steps above (and re-login if you were added to 'input'), then:"
    echo "    systemctl --user start $BIN"
fi
