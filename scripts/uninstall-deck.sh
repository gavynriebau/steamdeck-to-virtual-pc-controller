#!/usr/bin/env bash
# Remove the Deck side, including the Steam library entry (restarts Steam once).
# usage: uninstall-deck.sh [--deck user@host]
set -euo pipefail
# shellcheck source=lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

DECK=deck@deck
while [[ $# -gt 0 ]]; do
    case $1 in
        --deck) DECK=${2:?--deck needs a value}; shift 2 ;;
        -h|--help) sed -n '2,3p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) die "unknown argument '$1' (see --help)" ;;
    esac
done

BIN=deck-virtual-controller-client
scp -q "$ROOT/deploy/add_steam_shortcut.py" "$DECK:/tmp/add_steam_shortcut.py"
ssh "$DECK" "python3 /tmp/add_steam_shortcut.py --remove"
ssh "$DECK" 'for p in $(pgrep -f "[d]eck-virtual-controller-client") $(pgrep -f "[d]eck-controller-app"); do kill $p 2>/dev/null; done; true'
ssh "$DECK" "rm -f ~/.local/bin/$BIN ~/.local/bin/deck-controller-app ~/.local/bin/deck_supervisor.py ~/.local/share/applications/$BIN.desktop"
echo "Left in place: ~/.config/$BIN (your config)"
