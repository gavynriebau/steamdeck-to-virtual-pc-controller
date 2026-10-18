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

deck_connect "$DECK"
deck_copy_shortcut_tool "$DECK"
deck_ssh "$DECK" "python3 /tmp/add_steam_shortcut.py --remove"
deck_ctl "$DECK" remove-files
