# shellcheck shell=bash
# Shared by the scripts/tests/test-*.sh suites: the fake environment they build
# (a scratch directory, a release-shaped copy of the repo with fake binaries, a
# systemctl that only records) and the one place `fail` is defined. Sourced, not
# run: scripts/check.sh runs only the test-*.sh files.

REPO=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
CLIENT_BIN=deck-virtual-controller-client
SERVER_BIN=deck-virtual-controller-server

fail() { echo "FAIL: $*" >&2; exit 1; }

# make_tmp: a scratch directory in $TMP, removed when the test exits.
make_tmp() {
    TMP=$(mktemp -d)
    trap 'rm -rf "${TMP:?}"' EXIT
}

# fake_binaries <dir>: stand-ins for the prebuilt programs that announce themselves.
fake_binaries() {
    local dir=$1 bin
    mkdir -p "$dir"
    for bin in "$CLIENT_BIN" "$SERVER_BIN"; do
        printf '#!/bin/sh\necho fake %s\n' "$bin" > "$dir/$bin"
        chmod +x "$dir/$bin"
    done
}

# release_tree <dir>: a copy of the repo shaped like an unpacked release
# (scripts, deploy, deck, and fake prebuilt binaries in bin/).
release_tree() {
    local tree=$1
    mkdir -p "$tree"
    cp -r "$REPO/scripts" "$REPO/deploy" "$REPO/deck" "$tree/"
    fake_binaries "$tree/bin"
}

# fake_systemctl <bindir> <logfile>: a systemctl that only records how it was called.
fake_systemctl() {
    local bindir=$1 log=$2
    mkdir -p "$bindir"
    printf '#!/bin/sh\necho "$@" >> "%s"\n' "$log" > "$bindir/systemctl"
    chmod +x "$bindir/systemctl"
}
