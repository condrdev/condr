#!/bin/sh
set -eu

repo=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
commit=${CONDR_COMMIT:-${GITHUB_SHA:-$(git -C "$repo" rev-parse HEAD)}}
dist=${1:-dist}
dist=$(cd "$dist" && pwd)
stage=$(mktemp -d "${TMPDIR:-/tmp}/condr-package-check.XXXXXX")
trap 'rm -rf "$stage"' EXIT INT TERM
set -- "$dist"/*.AppImage
[ "$#" -eq 1 ]
(cd "$stage" && "$1" --appimage-extract > /dev/null)
appdir="$stage/squashfs-root"
readelf -h "$appdir/usr/bin/condr" > /dev/null
readelf -h "$appdir/usr/bin/condr-gui" > /dev/null
"$appdir/usr/bin/condr" server --help > /dev/null
test -f "$appdir/condr.desktop"
test -f "$appdir/condr.svg"
test -s "$appdir/LICENSE"
test "$(cat "$appdir/BUILD-COMMIT")" = "$commit"

set -- "$dist"/condr-cli-*-linux-*.tar.gz
[ "$#" -eq 1 ]
CONDR_COMMIT="$commit" sh "$repo/script/check-cli-package.sh" "$1" condr
echo 'Linux package content and CLI installation checks passed.'
