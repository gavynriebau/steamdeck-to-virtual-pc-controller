#!/usr/bin/env bash
# Assemble the release tarball from already-built binaries.
#
# usage: package.sh <version> [--bin-dir DIR] [--out DIR]
#   --bin-dir  where deck-virtual-controller-{client,server} are
#              (default: target/x86_64-unknown-linux-musl/release)
#   --out      where to put the tarball and its checksum (default: dist)
#
# The tree it builds is what scripts/lib.sh expects of an unpacked release:
# bin/ (prebuilt binaries), deploy/, deck/, scripts/, README.md, LICENSE.
# scripts/tests/test-package.sh checks that the installers run from it.
set -euo pipefail
# shellcheck source=lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

version=${1:-}
case $version in
    ""|-h|--help) sed -n '2,11p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; [[ -n $version ]] && exit 0; exit 2 ;;
esac
shift
bin_dir=$ROOT/target/x86_64-unknown-linux-musl/release
out=$ROOT/dist
while [[ $# -gt 0 ]]; do
    case $1 in
        --bin-dir) bin_dir=${2:?--bin-dir needs a value}; shift 2 ;;
        --out) out=${2:?--out needs a value}; shift 2 ;;
        *) die "unknown argument '$1' (see --help)" ;;
    esac
done

name="deck-virtual-controller-$version-x86_64-linux"
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
tree=$stage/$name
mkdir -p "$tree/bin"

for binary in deck-virtual-controller-client deck-virtual-controller-server; do
    [[ -x $bin_dir/$binary ]] || die "$bin_dir/$binary not found (build first)"
    cp "$bin_dir/$binary" "$tree/bin/"
done
# Everything an installer needs; the tests and caches stay behind.
for dir in deploy deck scripts; do
    (cd "$ROOT" && find "$dir" -type f ! -path '*/tests/*' ! -path '*/__pycache__/*' ! -name '*.pyc' -print0) |
        while IFS= read -r -d '' file; do
            mkdir -p "$tree/$(dirname "$file")"
            cp -p "$ROOT/$file" "$tree/$file"
        done
done
cp "$ROOT/README.md" "$ROOT/LICENSE" "$tree/"

mkdir -p "$out"
tar -C "$stage" -czf "$out/$name.tar.gz" "$name"
(cd "$out" && sha256sum "$name.tar.gz" > "$name.tar.gz.sha256")
echo "$out/$name.tar.gz"
