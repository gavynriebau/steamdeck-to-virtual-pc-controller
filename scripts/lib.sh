# shellcheck shell=bash
# Shared helpers for the install scripts. Sourced, not executed.

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)

die() { echo "error: $*" >&2; exit 1; }

# --- One SSH connection per run. The first ssh becomes a master that every later
# ssh and scp reuses (ControlMaster), so a password or key passphrase is asked for
# once, not once per step. deck_connect <user@host> starts it; the master is
# closed when the script exits, even on failure.
DECK_SSH_OPTS=()
DECK_SSH_DIR=
DECK_TARGET=

deck_connect() {
    DECK_TARGET=$1
    DECK_SSH_DIR=$(mktemp -d)
    DECK_SSH_OPTS=(-o ControlMaster=auto -o "ControlPath=$DECK_SSH_DIR/%C" -o ControlPersist=60)
    trap deck_disconnect EXIT
}

deck_disconnect() {
    [[ -n $DECK_SSH_DIR ]] || return 0
    ssh "${DECK_SSH_OPTS[@]}" -O exit "$DECK_TARGET" >/dev/null 2>&1 || true
    rm -rf "${DECK_SSH_DIR:?}"
    DECK_SSH_DIR=
}

deck_ssh() { ssh "${DECK_SSH_OPTS[@]}" "$@"; }
deck_scp() { scp "${DECK_SSH_OPTS[@]}" "$@"; }

# --- Where things live on the Deck: DECK_CLIENT, DECK_APP, DECK_SUPERVISOR,
# DECK_BIN_DIR, DECK_APPS_DIR, defined once in deploy/deck-layout.env.
DECK_LAYOUT="$ROOT/deploy/deck-layout.env"
# shellcheck source=../deploy/deck-layout.env
. "$DECK_LAYOUT"

# Run a command of the Deck-side helper. usage: deck_ctl <deck> <command...>
# The layout is sent ahead of the script so the helper uses the same names.
deck_ctl() {
    local deck=$1
    shift
    cat "$DECK_LAYOUT" "$ROOT/deploy/deck-ctl.sh" | deck_ssh "$deck" "sh -s -- $*"
}

# Copy the Steam shortcut tool, its format module and the layout it reads, to the Deck's /tmp.
deck_copy_shortcut_tool() {
    local deck=$1
    deck_scp -q "$ROOT/deploy/add_steam_shortcut.py" "$ROOT/deploy/steam_vdf.py" "$DECK_LAYOUT" "$deck:/tmp/"
}

# Copy the files the Deck needs. usage: deck_copy_files <deck> <client binary>
deck_copy_files() {
    local deck=$1 client=$2
    deck_scp -q "$client" "$deck:$DECK_BIN_DIR/$DECK_CLIENT"
    deck_scp -q "$ROOT/deck/deck-controller-app.py" "$deck:$DECK_BIN_DIR/$DECK_APP"
    deck_scp -q "$ROOT/deck/deck_supervisor.py" "$deck:$DECK_BIN_DIR/$DECK_SUPERVISOR"
    deck_scp -q "$ROOT/deploy/$DECK_CLIENT.desktop" "$deck:$DECK_APPS_DIR/"
    deck_ssh "$deck" "chmod +x $DECK_BIN_DIR/$DECK_CLIENT $DECK_BIN_DIR/$DECK_APP"
}

# Print the path of a binary: prebuilt (release tarball: $ROOT/bin) or built from source.
# usage: get_binary <name> [cargo target]
get_binary() {
    local name=$1 target=${2:-}
    if [[ -x "$ROOT/bin/$name" ]]; then
        echo "$ROOT/bin/$name"
        return
    fi
    command -v cargo >/dev/null || die "no prebuilt binary in $ROOT/bin and cargo is not installed"
    if [[ -n $target ]]; then
        (cd "$ROOT" && cargo build --release --target "$target" --bin "$name") >&2
        echo "$ROOT/target/$target/release/$name"
    else
        (cd "$ROOT" && cargo build --release --bin "$name") >&2
        echo "$ROOT/target/release/$name"
    fi
}
