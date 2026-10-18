# shellcheck shell=bash
# Shared helpers for the install scripts. Sourced, not executed.

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)

die() { echo "error: $*" >&2; exit 1; }

# --- Where things live on the Deck, relative to its home directory. The
# directories themselves are created (and removed) by deploy/deck-ctl.sh.
DECK_CLIENT=deck-virtual-controller-client
DECK_BIN_DIR=.local/bin
DECK_APPS_DIR=.local/share/applications

# Run a command of the Deck-side helper. usage: deck_ctl <deck> <command...>
deck_ctl() {
    local deck=$1
    shift
    ssh "$deck" "sh -s -- $*" < "$ROOT/deploy/deck-ctl.sh"
}

# Copy the files the Deck needs. usage: deck_copy_files <deck> <client binary>
deck_copy_files() {
    local deck=$1 client=$2
    scp -q "$client" "$deck:$DECK_BIN_DIR/$DECK_CLIENT"
    scp -q "$ROOT/deck/deck-controller-app.py" "$deck:$DECK_BIN_DIR/deck-controller-app"
    scp -q "$ROOT/deck/deck_supervisor.py" "$deck:$DECK_BIN_DIR/deck_supervisor.py"
    scp -q "$ROOT/deploy/$DECK_CLIENT.desktop" "$deck:$DECK_APPS_DIR/"
    ssh "$deck" "chmod +x $DECK_BIN_DIR/$DECK_CLIENT $DECK_BIN_DIR/deck-controller-app"
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
