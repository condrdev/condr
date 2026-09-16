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
cmp "$repo/packaging/icons/condr.svg" "$appdir/condr.svg"
grep -Fxq 'Icon=condr' "$appdir/condr.desktop"
grep -Fxq 'StartupWMClass=condr' "$appdir/condr.desktop"
test -s "$appdir/LICENSE"
test "$(cat "$appdir/BUILD-COMMIT")" = "$commit"

set -- "$dist"/condr-headless-*-linux-*.tar.gz
[ "$#" -eq 1 ]
CONDR_COMMIT="$commit" sh "$repo/script/check-headless-package.sh" "$1"
echo 'Linux package content and headless installation checks passed.'
