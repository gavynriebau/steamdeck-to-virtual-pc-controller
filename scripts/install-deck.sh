#!/usr/bin/env bash
# Install the Deck side: the streaming client, the fullscreen status app and a
# "Deck Virtual Controller" entry in the Steam library (restarts Steam once).
#
# usage: install-deck.sh [--deck user@host] [--pc host-or-ip]
#   --deck  SSH target of the Steam Deck           (default: deck@deck)
#   --pc    address of the PC running the server   (default: this machine's
#           address on the route to the Deck)
set -euo pipefail
# shellcheck source=lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

DECK=deck@deck
PC=
while [[ $# -gt 0 ]]; do
    case $1 in
        --deck) DECK=${2:?--deck needs a value}; shift 2 ;;
        --pc) PC=${2:?--pc needs a value}; shift 2 ;;
        -h|--help) sed -n '2,9p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) die "unknown argument '$1' (see --help)" ;;
    esac
done

BIN=deck-virtual-controller-client
APP=deck-controller-app

if [[ -z $PC ]]; then
    deck_ip=$(getent ahostsv4 "${DECK#*@}" | awk 'NR==1{print $1}')
    [[ -n $deck_ip ]] || die "cannot resolve '${DECK#*@}'; pass --deck user@host"
    PC=$(ip -4 route get "$deck_ip" | sed -n 's/.* src \([0-9.]*\).*/\1/p')
    [[ -n $PC ]] || die "cannot work out this machine's address; pass --pc"
fi
echo "Deck: $DECK   PC: $PC"

client=$(get_binary "$BIN" x86_64-unknown-linux-musl)

ssh "$DECK" "mkdir -p ~/.local/bin ~/.local/share/applications ~/.config/$BIN"
# Anything still running would make the binary busy ("Text file busy").
# (The bracket patterns keep pgrep from matching this very command.)
ssh "$DECK" "systemctl --user disable --now $BIN 2>/dev/null; rm -f ~/.config/systemd/user/$BIN.service; true"
ssh "$DECK" 'for p in $(pgrep -f "[d]eck-virtual-controller-client") $(pgrep -f "[d]eck-controller-app"); do kill $p 2>/dev/null; done; true'
scp -q "$client" "$DECK:.local/bin/$BIN"
scp -q "$ROOT/deck/$APP.py" "$DECK:.local/bin/$APP"
scp -q "$ROOT/deploy/$BIN.desktop" "$DECK:.local/share/applications/"
ssh "$DECK" "chmod +x ~/.local/bin/$BIN ~/.local/bin/$APP"

# Keep an existing config; otherwise install the default pointed at the PC.
sed "s/PC_ADDRESS/$PC/" "$ROOT/deploy/client/config.toml" |
    ssh "$DECK" "f=~/.config/$BIN/config.toml; if [ -e \$f ]; then echo 'keeping existing config'; else cat > \$f; fi"

# Add the Steam library entry (restarts Steam on the Deck so it picks it up).
scp -q "$ROOT/deploy/add_steam_shortcut.py" "$DECK:/tmp/add_steam_shortcut.py"
ssh "$DECK" "python3 /tmp/add_steam_shortcut.py"
echo "Done. Start 'Deck Virtual Controller' from the Steam library on the Deck."
