#!/usr/bin/env bash
# Remove the PC side (server, service). Root-owned files are listed, not removed.
set -euo pipefail
# shellcheck source=lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

BIN=deck-virtual-controller-server
systemctl --user disable --now "$BIN" 2>/dev/null || true
rm -f ~/.config/systemd/user/"$BIN".service ~/.local/bin/"$BIN"
systemctl --user daemon-reload
echo "Left in place: ~/.config/$BIN (your config)"
echo "To also remove the system files:"
echo "    sudo rm /etc/udev/rules.d/70-deck-virtual-controller.rules /etc/modules-load.d/deck-virtual-controller.conf"
