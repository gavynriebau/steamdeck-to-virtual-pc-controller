#!/usr/bin/env bash
# Runs deploy/deck-ctl.sh against a throwaway HOME and a fake app process.
set -euo pipefail
here=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
ctl="$here/deploy/deck-ctl.sh"
home=$(mktemp -d)
trap 'rm -rf "$home"; kill "${fake_pid:-}" 2>/dev/null || true' EXIT
fail() { echo "FAIL: $*" >&2; exit 1; }

# 1. stop kills a process whose command line names the app, and nothing else.
bash -c 'exec -a "python3 /x/deck-controller-app" sleep 60' &
fake_pid=$!
sleep 0.3
kill -0 "$fake_pid" || fail "fake app did not start"
# Fed over stdin exactly as the installers do.
HOME=$home sh -s -- stop < "$ctl"
sleep 0.3
if kill -0 "$fake_pid" 2>/dev/null; then fail "stop did not kill the app"; fi
echo "ok: stop kills the app"

# 2. stop does not kill the shell that runs it (the old pgrep trap).
HOME=$home sh -s -- stop < "$ctl" || fail "stop killed or failed its own shell"
echo "ok: stop survives its own command line"

# 3. directories, config install keeps an existing config.
HOME=$home sh -s -- make-dirs < "$ctl"
[ -d "$home/.config/deck-virtual-controller-client" ] || fail "make-dirs"
echo "first" > "$home/new.toml"
HOME=$home sh -s -- install-config "$home/new.toml" < "$ctl"
[ "$(cat "$home/.config/deck-virtual-controller-client/config.toml")" = first ] || fail "config not installed"
echo "second" > "$home/new2.toml"
HOME=$home sh -s -- install-config "$home/new2.toml" < "$ctl"
[ "$(cat "$home/.config/deck-virtual-controller-client/config.toml")" = first ] || fail "existing config overwritten"
echo "ok: config installed once, never overwritten"

# 4. remove-files deletes the installed files but keeps the config.
touch "$home/.local/bin/deck-virtual-controller-client" "$home/.local/bin/deck-controller-app" \
    "$home/.local/bin/deck_supervisor.py" "$home/.local/share/applications/deck-virtual-controller-client.desktop"
HOME=$home sh -s -- remove-files < "$ctl" > /dev/null
for f in deck-virtual-controller-client deck-controller-app deck_supervisor.py; do
    [ ! -e "$home/.local/bin/$f" ] || fail "$f was not removed"
done
[ -e "$home/.config/deck-virtual-controller-client/config.toml" ] || fail "config was removed"
echo "ok: remove-files keeps the config"

# 5. unknown command is a usage error.
if HOME=$home sh -s -- bogus < "$ctl" 2>/dev/null; then fail "bogus accepted"; fi
echo "ok: unknown command rejected"
