#!/usr/bin/env bash
# Runs scripts/install-pc.sh and uninstall-pc.sh against a throwaway HOME, a fake
# systemctl and a fake /dev/uhid, from a copy of the repo with a fake server binary.
set -euo pipefail
here=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
fail() { echo "FAIL: $*" >&2; exit 1; }

# A tree shaped like an unpacked release: scripts, deploy, and a prebuilt binary.
tree=$tmp/tree
mkdir -p "$tree/bin"
cp -r "$here/scripts" "$here/deploy" "$tree/"
printf '#!/bin/sh\necho fake server\n' > "$tree/bin/deck-virtual-controller-server"
chmod +x "$tree/bin/deck-virtual-controller-server"

# systemctl only records how it was called.
mkdir -p "$tmp/fakebin"
printf '#!/bin/sh\necho "$@" >> "%s/systemctl.log"\n' "$tmp" > "$tmp/fakebin/systemctl"
chmod +x "$tmp/fakebin/systemctl"

home=$tmp/home
mkdir -p "$home"
touch "$tmp/uhid"   # writable, like /dev/uhid once the udev rule is in place
install_pc() { HOME=$home PATH="$tmp/fakebin:$PATH" UHID_DEVICE=${UHID_DEVICE:-$tmp/uhid} "$tree/scripts/install-pc.sh" "$@" 2>&1; }

bin=$home/.local/bin/deck-virtual-controller-server
cfg=$home/.config/deck-virtual-controller-server/config.toml
unit=$home/.config/systemd/user/deck-virtual-controller-server.service

# 1. Fresh install puts the binary, config and unit in place and starts the service.
out=$(install_pc)
[ -x "$bin" ] || fail "binary not installed or not executable"
cmp -s "$tree/deploy/server/config.toml" "$cfg" || fail "default config not installed"
cmp -s "$tree/deploy/deck-virtual-controller-server.service" "$unit" || fail "unit not installed"
grep -q -- "--user daemon-reload" "$tmp/systemctl.log" || fail "no daemon-reload"
grep -q -- "--user enable --now deck-virtual-controller-server" "$tmp/systemctl.log" || fail "service not enabled and started"
echo "ok: fresh install"

# 2. The root steps are printed, not run, and point into this tree.
for needle in "sudo install -m 644 \"$tree/deploy/server/70-deck-virtual-controller.rules\"" \
              "sudo modprobe uhid uinput" "sudo udevadm control --reload" "24800/udp"; do
    grep -qF -- "$needle" <<< "$out" || fail "output lacks: $needle"
done
echo "ok: root steps and firewall hint are printed"

# 3. An existing config is never overwritten.
echo "bind_addr = \"127.0.0.1:1\"" > "$cfg"
out=$(install_pc)
grep -q "keeping existing config" <<< "$out" || fail "did not say it kept the config"
[ "$(cat "$cfg")" = 'bind_addr = "127.0.0.1:1"' ] || fail "config was overwritten"
echo "ok: existing config kept"

# 4. A running instance is stopped before the binary is replaced.
grep -q -- "--user stop deck-virtual-controller-server" "$tmp/systemctl.log" || fail "did not stop the running service"
echo "ok: running service stopped first"

# 5. Without write access to the device the service is enabled but not started.
: > "$tmp/systemctl.log"
out=$(UHID_DEVICE="$tmp/missing-uhid" install_pc)
grep -q -- "--user enable deck-virtual-controller-server" "$tmp/systemctl.log" || fail "service not enabled"
if grep -q -- "enable --now" "$tmp/systemctl.log"; then fail "started a service that cannot work"; fi
grep -q "is not writable yet" <<< "$out" || fail "no explanation for the user"
echo "ok: no device access: enabled, not started, explained"

# 6. --help and a bad argument.
install_pc --help | grep -q "usage: install-pc.sh" || fail "no usage in --help"
if install_pc --bogus >/dev/null; then fail "accepted an unknown argument"; fi
echo "ok: --help and argument checking"

# 7. Uninstall removes the program and the unit, keeps the config.
HOME=$home PATH="$tmp/fakebin:$PATH" "$tree/scripts/uninstall-pc.sh" >/dev/null 2>&1
[ ! -e "$bin" ] || fail "binary not removed"
[ ! -e "$unit" ] || fail "unit not removed"
[ -e "$cfg" ] || fail "config was removed"
grep -q -- "--user disable --now deck-virtual-controller-server" "$tmp/systemctl.log" || fail "service not disabled"
echo "ok: uninstall removes everything but the config"
