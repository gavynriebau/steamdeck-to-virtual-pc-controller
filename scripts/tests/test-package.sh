#!/usr/bin/env bash
# Builds a release tarball from fake binaries, unpacks it, checks what is (and is
# not) in it, that every file the installers reference is there, and that the
# installers actually run from the unpacked tree.
set -euo pipefail
here=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
fail() { echo "FAIL: $*" >&2; exit 1; }

mkdir -p "$tmp/bins"
for b in deck-virtual-controller-client deck-virtual-controller-server; do
    printf '#!/bin/sh\necho fake %s\n' "$b" > "$tmp/bins/$b"
    chmod +x "$tmp/bins/$b"
done

# 1. Packaging produces the tarball and a checksum that verifies.
tarball=$("$here/scripts/package.sh" v9.9.9 --bin-dir "$tmp/bins" --out "$tmp/out")
[ "$tarball" = "$tmp/out/deck-virtual-controller-v9.9.9-x86_64-linux.tar.gz" ] || fail "unexpected tarball path: $tarball"
(cd "$tmp/out" && sha256sum -c ./*.sha256 >/dev/null) || fail "checksum does not verify"
echo "ok: tarball and checksum"

# 2. It contains what an installer needs and none of the development files.
mkdir -p "$tmp/unpacked"
tar -C "$tmp/unpacked" -xzf "$tarball"
tree=$tmp/unpacked/deck-virtual-controller-v9.9.9-x86_64-linux
for f in bin/deck-virtual-controller-client bin/deck-virtual-controller-server \
         deploy/deck-layout.env deploy/deck-ctl.sh deploy/add_steam_shortcut.py deploy/steam_vdf.py \
         deploy/deck-virtual-controller-client.desktop deploy/deck-virtual-controller-server.service \
         deploy/client/config.toml deploy/server/config.toml \
         deploy/server/70-deck-virtual-controller.rules deploy/server/deck-virtual-controller.conf \
         deck/deck-controller-app.py deck/deck_supervisor.py \
         scripts/lib.sh scripts/install-deck.sh scripts/install-pc.sh \
         scripts/uninstall-deck.sh scripts/uninstall-pc.sh README.md LICENSE; do
    [ -f "$tree/$f" ] || fail "missing from the tarball: $f"
done
if find "$tree" \( -name tests -o -name __pycache__ -o -name '*.pyc' \) | grep -q .; then
    fail "development files leaked into the tarball"
fi
[ -x "$tree/bin/deck-virtual-controller-server" ] || fail "binary lost its executable bit"
echo "ok: contents"

# 3. Every $ROOT/... path the installers mention exists in the unpacked tree.
# (Each script sets $BIN itself: the server for the PC scripts, the client otherwise.)
missing=0
check_refs() {
    local bin=$1
    shift
    while IFS= read -r ref; do
        path=$(ROOT=$tree; BIN=$bin; . "$tree/deploy/deck-layout.env"; eval "echo \"$ref\"")
        [ -e "$path" ] || { echo "  missing: $path (from $ref)" >&2; missing=1; }
    done < <(grep -ho '\$ROOT/[A-Za-z0-9_./$-]*' "$@" | sort -u | grep -v '\$ROOT/target\|\$ROOT/dist\|\$ROOT/bin/\$name')
}
check_refs deck-virtual-controller-server "$tree/scripts/install-pc.sh" "$tree/scripts/uninstall-pc.sh"
check_refs deck-virtual-controller-client "$tree/scripts/install-deck.sh" "$tree/scripts/uninstall-deck.sh" "$tree/scripts/lib.sh"
[ $missing -eq 0 ] || fail "the installers reference files the tarball does not contain"
echo "ok: every file the installers reference is in the tarball"

# 4. lib.sh finds the prebuilt binaries in the unpacked tree (no cargo needed).
for b in deck-virtual-controller-client deck-virtual-controller-server; do
    found=$(PATH=/usr/bin:/bin bash -c 'source "$1/scripts/lib.sh"; get_binary "$2"' _ "$tree" "$b")
    [ "$found" = "$tree/bin/$b" ] || fail "get_binary $b -> $found"
done
echo "ok: installers use the prebuilt binaries"

# 5. The PC installer runs from the unpacked tree.
mkdir -p "$tmp/fakebin" "$tmp/home"
printf '#!/bin/sh\necho "$@" >> "%s/systemctl.log"\n' "$tmp" > "$tmp/fakebin/systemctl"
chmod +x "$tmp/fakebin/systemctl"
touch "$tmp/uhid"
HOME=$tmp/home PATH="$tmp/fakebin:$PATH" UHID_DEVICE=$tmp/uhid "$tree/scripts/install-pc.sh" >/dev/null 2>&1 \
    || fail "install-pc.sh failed from the unpacked tree"
"$tmp/home/.local/bin/deck-virtual-controller-server" | grep -q "fake deck-virtual-controller-server" || fail "installed binary is not the packaged one"
echo "ok: install-pc.sh runs from the tarball"

# 6. The Deck installer starts from the unpacked tree too.
"$tree/scripts/install-deck.sh" --help | grep -q "usage: install-deck.sh" || fail "install-deck.sh --help"
echo "ok: install-deck.sh runs from the tarball"

# 7. Missing binaries and a missing version are errors, not empty packages.
if "$here/scripts/package.sh" v1 --bin-dir "$tmp/nowhere" --out "$tmp/o2" >/dev/null 2>&1; then fail "packaged without binaries"; fi
if "$here/scripts/package.sh" >/dev/null 2>&1; then fail "packaged without a version"; fi
echo "ok: refuses to package without binaries or a version"
