#!/usr/bin/env bash
# Tests scripts/check.sh's orchestration: groups, keep-going, the summary, exit
# status, and how it treats a missing tool. Runs it from a copy of the scripts
# with stub test suites and fake cargo/rustup/python3, so nothing real is run.
set -uo pipefail
# shellcheck source=lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
make_tmp
tmp=$TMP
here=$REPO

# A tree with the real check.sh and stub shell suites (none of them is test-check.sh).
tree=$tmp/tree
mkdir -p "$tree/scripts/tests" "$tree/deck" "$tree/deploy"
cp "$here/scripts/check.sh" "$tree/scripts/"
for name in test-alpha test-beta; do
    printf '#!/bin/sh\necho "stub %s" >> "%s/log"\n[ "${FAKE_FAIL:-}" = "%s" ] && exit 1\nexit 0\n' "$name" "$tmp" "$name" > "$tree/scripts/tests/$name.sh"
    chmod +x "$tree/scripts/tests/$name.sh"
done

# Fake tools that log their use and fail when told to: FAKE_FAIL=<word in their args>.
fakebin=$tmp/fakebin
mkdir -p "$fakebin"
for tool in cargo python3 shellcheck; do
    cat > "$fakebin/$tool" <<EOF
#!/bin/sh
echo "$tool \$*" >> "$tmp/log"
case " \$* " in *" \${FAKE_FAIL:-no-such-word} "*) exit 1 ;; esac
exit 0
EOF
    chmod +x "$fakebin/$tool"
done
cat > "$fakebin/rustup" <<EOF
#!/bin/sh
echo "rustup \$*" >> "$tmp/log"
[ "\${FAKE_NO_MUSL:-}" = 1 ] || echo x86_64-unknown-linux-musl
EOF
chmod +x "$fakebin/rustup"

# A PATH with only the basics, so a real shellcheck on this machine stays hidden.
basics=$tmp/basics
mkdir -p "$basics"
for tool in bash env sed basename dirname grep cat sort; do ln -s "$(command -v "$tool")" "$basics/$tool"; done

# check [env...] -- [args...]: run check.sh, capture output to $tmp/out, return its status.
check() {
    : > "$tmp/log"
    local -a vars=()
    while [[ $# -gt 0 && $1 != -- ]]; do vars+=("$1"); shift; done
    shift
    env "${vars[@]}" PATH="$fakebin:$basics" "$tree/scripts/check.sh" "$@" > "$tmp/out" 2>&1
}
ran() { grep -q -- "$1" "$tmp/log"; }
said() { grep -q -- "$1" "$tmp/out"; }

# 1. Everything passes: every tool and stub suite runs, exit 0.
check -- || fail "all-green run exited non-zero: $(cat "$tmp/out")"
for needle in "cargo fmt --check" "cargo clippy --workspace --all-targets -- -D warnings" "cargo test --workspace" \
              "cargo build --release --target x86_64-unknown-linux-musl" "shellcheck --severity=warning" \
              "stub test-alpha" "stub test-beta" "python3 -W error -m unittest discover -s deck/tests"; do
    ran "$needle" || fail "never ran: $needle"
done
said "failed: 0" || fail "summary lacks 'failed: 0'"
echo "ok: all groups run and a clean run exits 0"

# 2. A failing step does not stop the ones after it, and is named in the summary.
check FAKE_FAIL=clippy -- && fail "a failing clippy still exited 0"
ran "cargo test --workspace" || fail "tests were not run after the clippy failure"
ran "stub test-beta" || fail "later groups were not run after the failure"
said "FAILED:  rust: clippy" || fail "the failure is not named in the summary"
echo "ok: keeps going after a failure and names it"

# 3. Groups can be chosen; an unknown one is a usage error.
check -- python
if ran "cargo"; then fail "the python group ran cargo"; fi
ran "python3 -W error" || fail "the python group did not run the python tests"
check -- bogus; [ $? -eq 2 ] || fail "an unknown group did not exit 2"
echo "ok: group selection"

# 4. A shell suite that fails is reported by name.
check FAKE_FAIL=test-beta -- shell && fail "a failing suite still exited 0"
said "FAILED:  shell: test-beta" || fail "the failing suite is not named"
said "shell: test-alpha" || fail "the passing suite was not run"
echo "ok: shell suites are reported individually"

# 5. A missing shellcheck is skipped out loud, and fails under CHECK_STRICT.
rm "$fakebin/shellcheck"
check -- shell || fail "a missing shellcheck failed a non-strict run"
said "shell: shellcheck: SKIPPED" || fail "the skip was not announced"
said "skipped: 1" || fail "the skip is not in the summary"
check CHECK_STRICT=1 -- shell && fail "CHECK_STRICT did not turn the skip into a failure"
said "skipped checks count as failures" || fail "no explanation under CHECK_STRICT"
echo "ok: a missing tool is skipped out loud, and fails when strict"

# 6. A missing musl target skips the static build with the fix in the message.
check FAKE_NO_MUSL=1 -- rust || fail "a missing musl target failed a non-strict run"
if ran "cargo build"; then fail "tried to build without the musl target"; fi
said "rustup target add x86_64-unknown-linux-musl" || fail "no hint for installing the target"
echo "ok: a missing musl target is skipped with a hint"

# 7. --help.
check -- --help; said "usage: check.sh" || fail "--help prints no usage"
echo "ok: --help"
