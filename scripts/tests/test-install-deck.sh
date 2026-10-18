#!/usr/bin/env bash
# Runs install-deck.sh and uninstall-deck.sh from a copy of the repo with a fake
# ssh and scp that record every call, and checks what they were asked to do: one
# shared connection for the whole run, closed at the end even when a step
# fails, and the right things sent to the Deck.
set -uo pipefail
# shellcheck source=lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
make_tmp
tmp=$TMP

tree=$tmp/tree
release_tree "$tree"

# Fakes: log "<tool> <args>", keep the config piped to `cat >`, fail on request.
fakebin=$tmp/fakebin
mkdir -p "$fakebin"
for tool in ssh scp; do
    cat > "$fakebin/$tool" <<EOF
#!/bin/sh
echo "$tool \$*" >> "$tmp/calls"
# Only read stdin where the scripts pipe something in (the config, and the helper
# script for \`sh -s\`); a real ssh does not wait on stdin for the other calls either.
case "\$*" in
    *"cat > "*) cat > "$tmp/piped-config" ;;
    *"sh -s"*) cat > /dev/null ;;
esac
[ -n "\${FAKE_FAIL_ON:-}" ] && case "\$*" in *"\$FAKE_FAIL_ON"*) exit 1 ;; esac
exit 0
EOF
    chmod +x "$fakebin/$tool"
done

run() { # run <script> [args...]; output to $tmp/out, calls to $tmp/calls
    : > "$tmp/calls"
    local script=$1
    shift
    PATH="$fakebin:$PATH" "$tree/scripts/$script" "$@" > "$tmp/out" 2>&1
}
calls() { grep -c "^$1 " "$tmp/calls"; }
control_paths() { grep -o 'ControlPath=[^ ]*' "$tmp/calls" | sort -u; }

# 1. A full install uses one shared connection for every ssh and scp.
run install-deck.sh --deck me@steamdeck --pc 192.168.1.20 || fail "install failed: $(cat "$tmp/out")"
total=$(( $(calls ssh) + $(calls scp) ))
[ "$total" -ge 9 ] || fail "suspiciously few ssh/scp calls ($total)"
while IFS= read -r line; do
    case $line in *"-O exit"*) continue ;; esac
    [[ $line == *"ControlMaster=auto"* && $line == *"ControlPath="* ]] || fail "a call did not use the shared connection: $line"
done < "$tmp/calls"
[ "$(control_paths | wc -l)" -eq 1 ] || fail "calls used more than one control path: $(control_paths)"
echo "ok: all $total ssh/scp calls share one connection"

# 2. The shared connection is closed at the end, and its directory removed.
[ "$(grep -c -- '-O exit' "$tmp/calls")" -eq 1 ] || fail "expected exactly one '-O exit'"
[[ $(tail -1 "$tmp/calls") == *"-O exit"* ]] || fail "'-O exit' was not the last call"
sock_dir=$(control_paths | head -1 | sed 's/^ControlPath=//; s|/%C$||')
[ ! -e "$sock_dir" ] || fail "the control directory $sock_dir was left behind"
echo "ok: the connection is closed and cleaned up"

# 3. Even when a step fails the connection is closed and cleaned up.
FAKE_FAIL_ON="python3" run install-deck.sh --deck me@steamdeck --pc 192.168.1.20 && fail "a failing step still exited 0"
[[ $(tail -1 "$tmp/calls") == *"-O exit"* ]] || fail "no '-O exit' after a failure"
sock_dir=$(control_paths | head -1 | sed 's/^ControlPath=//; s|/%C$||')
[ ! -e "$sock_dir" ] || fail "the control directory survived a failure"
echo "ok: a failure still closes the connection"

# 4. The right things go to the Deck: the config with the PC's address, the files, the shortcut tool.
run install-deck.sh --deck me@steamdeck --pc 192.168.1.20 || fail "install failed"
grep -q 'server_addr = "192.168.1.20:24800"' "$tmp/piped-config" || fail "the PC address was not filled into the config"
for needle in "me@steamdeck:.local/bin/deck-virtual-controller-client" "me@steamdeck:.local/bin/deck-controller-app" \
              "steam_vdf.py" "add_steam_shortcut.py" "sh -s -- make-dirs" "sh -s -- stop" "python3 /tmp/add_steam_shortcut.py"; do
    grep -qF -- "$needle" "$tmp/calls" || fail "never sent: $needle"
done
echo "ok: config, files and shortcut tool all reach the Deck"

# 5. Uninstall shares one connection too, and asks for the removal.
run uninstall-deck.sh --deck me@steamdeck || fail "uninstall failed: $(cat "$tmp/out")"
[ "$(control_paths | wc -l)" -eq 1 ] || fail "uninstall used more than one control path"
grep -qF -- "python3 /tmp/add_steam_shortcut.py --remove" "$tmp/calls" || fail "the shortcut was not removed"
grep -qF -- "sh -s -- remove-files" "$tmp/calls" || fail "the files were not removed"
[[ $(tail -1 "$tmp/calls") == *"-O exit"* ]] || fail "uninstall left the connection open"
echo "ok: uninstall shares one connection and removes everything"

# 6. A bad argument is refused before anything is sent.
run install-deck.sh --bogus && fail "accepted an unknown argument"
[ ! -s "$tmp/calls" ] || fail "contacted the Deck despite a bad argument"
echo "ok: bad arguments are refused before connecting"
