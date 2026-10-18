#!/bin/sh
# Runs ON the Steam Deck. The install scripts feed it over ssh from stdin,
# preceded by deploy/deck-layout.env (which defines the DECK_* names used here):
#
#   cat deploy/deck-layout.env deploy/deck-ctl.sh | ssh deck 'sh -s -- stop'
#
# so the remote command line never contains the names matched below and there
# is one quoting layer.
#
#   stop                     stop the app and the client (and the old systemd service)
#   make-dirs                create the directories the installer copies into
#   install-config <file>    move <file> to the client config unless one exists
#   remove-files             delete everything the installer put on the Deck
set -eu

: "${DECK_CLIENT:?run this through scripts/lib.sh deck_ctl, which sends deck-layout.env first}"
: "${DECK_APP:?missing from deck-layout.env}"
: "${DECK_SUPERVISOR:?missing from deck-layout.env}"
: "${DECK_BIN_DIR:?missing from deck-layout.env}"
: "${DECK_APPS_DIR:?missing from deck-layout.env}"
BIN_DIR="$HOME/$DECK_BIN_DIR"
APPS_DIR="$HOME/$DECK_APPS_DIR"
CONFIG_DIR="$HOME/.config/$DECK_CLIENT"

# A pgrep pattern that matches `name` but not this script's own command line
# (or any other that merely contains the pattern text): "deck" -> "[d]eck".
self_safe() {
    first=$(printf %s "$1" | cut -c1)
    rest=$(printf %s "$1" | cut -c2-)
    printf '[%s]%s' "$first" "$rest"
}

stop() {
    systemctl --user disable --now "$DECK_CLIENT" 2>/dev/null || true
    rm -f "${HOME:?}/.config/systemd/user/${DECK_CLIENT:?}.service"
    for pid in $(pgrep -f "$(self_safe "$DECK_CLIENT")") $(pgrep -f "$(self_safe "$DECK_APP")"); do
        kill "$pid" 2>/dev/null || true
    done
}

case "${1:-}" in
    stop)
        stop
        ;;
    make-dirs)
        mkdir -p "$BIN_DIR" "$APPS_DIR" "$CONFIG_DIR"
        ;;
    install-config)
        src=${2:?install-config needs a file}
        if [ -e "$CONFIG_DIR/config.toml" ]; then
            echo "keeping existing config"
            rm -f "${src:?}"
        else
            mv "$src" "$CONFIG_DIR/config.toml"
        fi
        ;;
    remove-files)
        stop
        rm -f "${BIN_DIR:?}/${DECK_CLIENT:?}" "${BIN_DIR:?}/${DECK_APP:?}" \
            "${BIN_DIR:?}/${DECK_SUPERVISOR:?}" "${APPS_DIR:?}/${DECK_CLIENT:?}.desktop"
        echo "Left in place: $CONFIG_DIR (your config)"
        ;;
    *)
        echo "usage: deck-ctl.sh stop|make-dirs|install-config <file>|remove-files" >&2
        exit 2
        ;;
esac
