# shellcheck shell=bash
# Shared helpers for the install scripts. Sourced, not executed.

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)

die() { echo "error: $*" >&2; exit 1; }

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
