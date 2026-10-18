#!/bin/sh
# Runs ON the Steam Deck. The install scripts feed it over ssh from stdin
# (`ssh deck 'sh -s -- stop' < deploy/deck-ctl.sh`), so the remote command line
# never contains the names matched below, and there is one quoting layer.
#
#   stop                     stop the app and the client (and the old systemd service)
#   make-dirs                create the directories the installer copies into
#   install-config <file>    move <file> to the client config unless one exists
#   remove-files             delete everything the installer put on the Deck
set -eu

CLIENT=deck-virtual-controller-client
BIN_DIR="$HOME/.local/bin"
APPS_DIR="$HOME/.local/share/applications"
CONFIG_DIR="$HOME/.config/$CLIENT"

stop() {
    systemctl --user disable --now "$CLIENT" 2>/dev/null || true
    rm -f "$HOME/.config/systemd/user/$CLIENT.service"
    # Bracketed patterns keep pgrep from matching this script's own shell.
    for pid in $(pgrep -f "[d]eck-virtual-controller-client") $(pgrep -f "[d]eck-controller-app"); do
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
            rm -f "$src"
        else
            mv "$src" "$CONFIG_DIR/config.toml"
        fi
        ;;
    remove-files)
        stop
        rm -f "$BIN_DIR/$CLIENT" "$BIN_DIR/deck-controller-app" "$BIN_DIR/deck_supervisor.py" \
            "$APPS_DIR/$CLIENT.desktop"
        echo "Left in place: $CONFIG_DIR (your config)"
        ;;
    *)
        echo "usage: deck-ctl.sh stop|make-dirs|install-config <file>|remove-files" >&2
        exit 2
        ;;
esac
