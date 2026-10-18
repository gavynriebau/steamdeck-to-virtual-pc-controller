#!/usr/bin/env bash
# Runs deploy/deck-ctl.sh against a throwaway HOME and a fake app process.
set -euo pipefail
# shellcheck source=lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
here=$REPO
ctl="$here/deploy/deck-ctl.sh"
layout="$here/deploy/deck-layout.env"
# Fed exactly as the installers do: the layout, then the script, over stdin.
run() { cat "$layout" "$ctl" | HOME=$home sh -s -- "$@"; }
home=$(mktemp -d)
trap 'rm -rf "$home"; kill "${fake_pid:-}" 2>/dev/null || true' EXIT

# 1. stop kills a process whose command line names the app, and nothing else.
bash -c 'exec -a "python3 /x/deck-controller-app" sleep 60' &
fake_pid=$!
sleep 0.3
kill -0 "$fake_pid" || fail "fake app did not start"
run stop
sleep 0.3
if kill -0 "$fake_pid" 2>/dev/null; then fail "stop did not kill the app"; fi
echo "ok: stop kills the app"

# 2. stop does not kill the shell that runs it (the old pgrep trap).
run stop || fail "stop killed or failed its own shell"
echo "ok: stop survives its own command line"

# 3. directories, config install keeps an existing config.
run make-dirs
[ -d "$home/.config/deck-virtual-controller-client" ] || fail "make-dirs"
echo "first" > "$home/new.toml"
run install-config "$home/new.toml"
[ "$(cat "$home/.config/deck-virtual-controller-client/config.toml")" = first ] || fail "config not installed"
echo "second" > "$home/new2.toml"
run install-config "$home/new2.toml"
[ "$(cat "$home/.config/deck-virtual-controller-client/config.toml")" = first ] || fail "existing config overwritten"
echo "ok: config installed once, never overwritten"

# 4. remove-files deletes the installed files but keeps the config.
touch "$home/.local/bin/deck-virtual-controller-client" "$home/.local/bin/deck-controller-app" \
    "$home/.local/bin/deck_supervisor.py" "$home/.local/share/applications/deck-virtual-controller-client.desktop"
run remove-files > /dev/null
for f in deck-virtual-controller-client deck-controller-app deck_supervisor.py; do
    [ ! -e "$home/.local/bin/$f" ] || fail "$f was not removed"
done
[ -e "$home/.config/deck-virtual-controller-client/config.toml" ] || fail "config was removed"
echo "ok: remove-files keeps the config"

# 5. unknown command is a usage error.
if run bogus 2>/dev/null; then fail "bogus accepted"; fi
echo "ok: unknown command rejected"

# 6. Without the layout the helper refuses to run instead of guessing paths.
if HOME=$home sh -s -- stop < "$ctl" 2>/dev/null; then fail "ran without the layout"; fi
echo "ok: refuses to run without the layout"
