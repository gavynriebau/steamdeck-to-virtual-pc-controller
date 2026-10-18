#!/usr/bin/env bash
# Run every check the project has: what CI runs, in the same way.
#
# usage: check.sh [group...]
#   groups: rust    fmt, clippy, tests, the static Deck/PC builds
#           python  syntax and the unit tests (Deck app, Steam shortcut tool)
#           shell   shellcheck and the installer/helper/package tests
#   With no group, all of them. Every step runs even if an earlier one fails;
#   the summary lists what failed and the exit status is non-zero if anything did.
#
# A step whose tool is missing (shellcheck, the musl target) is SKIPPED and said
# so. Set CHECK_STRICT=1 (CI does) to count a skip as a failure.
set -uo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

export PYTHONDONTWRITEBYTECODE=1   # checks must not leave __pycache__ behind
passed=() failed=() skipped=()

# step <name> <command...>: run it, record the outcome, keep going.
step() {
    local name=$1
    shift
    echo "== $name"
    if "$@"; then passed+=("$name"); else failed+=("$name"); echo "-- FAILED: $name" >&2; fi
}

# skip <name> <why>
skip() {
    echo "== $1: SKIPPED ($2)"
    skipped+=("$1 ($2)")
}

have() { command -v "$1" >/dev/null 2>&1; }

check_rust() {
    step "rust: fmt" cargo fmt --check
    step "rust: clippy" cargo clippy --workspace --all-targets -- -D warnings
    step "rust: tests" cargo test --workspace
    if have rustup && ! rustup target list --installed 2>/dev/null | grep -q '^x86_64-unknown-linux-musl$'; then
        skip "rust: static build" "musl target not installed: rustup target add x86_64-unknown-linux-musl"
    else
        step "rust: static build" cargo build --release --target x86_64-unknown-linux-musl
    fi
}

compile_python() {
    python3 - deck/*.py deploy/*.py <<'PY'
import sys
for path in sys.argv[1:]:
    with open(path) as f:
        compile(f.read(), path, "exec")
PY
}

check_python() {
    step "python: syntax" compile_python
    step "python: Deck app tests" python3 -W error -m unittest discover -s deck/tests
    step "python: Steam shortcut tests" python3 -m unittest discover -s deploy/tests
}

check_shell() {
    if have shellcheck; then
        step "shell: shellcheck" shellcheck --severity=warning scripts/*.sh scripts/tests/*.sh deploy/deck-ctl.sh
    else
        skip "shell: shellcheck" "shellcheck is not installed"
    fi
    local test
    for test in scripts/tests/test-*.sh; do
        step "shell: $(basename "$test" .sh)" "$test"
    done
}

groups=("$@")
[[ ${#groups[@]} -gt 0 ]] || groups=(rust python shell)
for group in "${groups[@]}"; do
    case $group in
        rust|python|shell) ;;
        -h|--help) sed -n '2,13p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "error: unknown group '$group' (rust, python, shell)" >&2; exit 2 ;;
    esac
done
for group in "${groups[@]}"; do "check_$group"; done

echo
echo "passed: ${#passed[@]}   failed: ${#failed[@]}   skipped: ${#skipped[@]}"
for s in "${skipped[@]}"; do echo "  skipped: $s"; done
for f in "${failed[@]}"; do echo "  FAILED:  $f"; done
if [[ ${#failed[@]} -gt 0 ]]; then exit 1; fi
if [[ ${CHECK_STRICT:-0} = 1 && ${#skipped[@]} -gt 0 ]]; then
    echo "CHECK_STRICT is set: skipped checks count as failures" >&2
    exit 1
fi
