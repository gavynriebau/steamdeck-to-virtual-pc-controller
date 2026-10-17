#!/usr/bin/env bash
# Build the client, install it on the Steam Deck and add it to the Steam library.
# usage: ./install.sh [deck-ssh-target] [pc-ip]
set -euo pipefail
cd "$(dirname "$0")"

DECK=${1:-deck@deck}
BIN=deck-virtual-controller-client
TARGET=x86_64-unknown-linux-musl

cargo build --release --target "$TARGET" --bin "$BIN"

# Default PC address: the local IP used to reach the Deck.
if [[ -n ${2:-} ]]; then
    PC_IP=$2
else
    PC_IP=$(ip -4 route get "$(getent ahostsv4 "${DECK#*@}" | awk 'NR==1{print $1}')" | sed -n 's/.* src \([0-9.]*\).*/\1/p')
fi
echo "Deck: $DECK   PC: $PC_IP"

ssh "$DECK" 'mkdir -p ~/.local/bin ~/.config/systemd/user ~/.local/share/applications ~/.config/'$BIN
# Stop first so the binary isn't busy ("Text file busy").
ssh "$DECK" "systemctl --user stop $BIN 2>/dev/null || true"
scp "target/$TARGET/release/$BIN" "$DECK:.local/bin/$BIN"
scp deck/deck-controller-app.py "$DECK:.local/bin/deck-controller-app"
scp deploy/$BIN.desktop "$DECK:.local/share/applications/"
ssh "$DECK" "chmod +x ~/.local/bin/deck-controller-app"
# Streaming now runs only while the app is open in Steam; drop the always-on service.
ssh "$DECK" "systemctl --user disable --now $BIN 2>/dev/null || true; rm -f ~/.config/systemd/user/$BIN.service; systemctl --user daemon-reload"

# Keep an existing config; otherwise install the default pointed at this PC.
sed "s/^server_addr = .*/server_addr = \"$PC_IP:24800\"/" deploy/client/config.toml |
    ssh "$DECK" "f=~/.config/$BIN/config.toml; [ -e \$f ] && echo 'keeping existing config' || cat > \$f"

# Add the Steam library entry (restarts Steam on the Deck so it picks it up).
scp deploy/add_steam_shortcut.py "$DECK:/tmp/add_steam_shortcut.py"
ssh "$DECK" "python3 /tmp/add_steam_shortcut.py"
