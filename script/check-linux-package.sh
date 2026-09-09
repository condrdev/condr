#!/bin/sh
set -eu

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
test -s "$appdir/BUILD-COMMIT"

set -- "$dist"/condr-cli-*-linux-*.tar.gz
[ "$#" -eq 1 ]
CONDR_INSTALL_DIR="$stage/install" CONDR_PROFILE="$stage/profile" \
    HOME="$stage/home" sh script/install-condr.sh --from "$1"
"$stage/home/.local/bin/condr" server --help > /dev/null
printf '#!/bin/sh\necho previous-cli\n' >"$stage/install/condr"
printf 'keep GUI\n' >"$stage/install/condr-gui"
printf 'n\n' | CONDR_INSTALL_DIR="$stage/install" CONDR_PROFILE="$stage/profile" \
    HOME="$stage/home" sh script/install-condr.sh --from "$1"
test "$("$stage/install/condr")" = previous-cli
printf 'Yes\n' | CONDR_INSTALL_DIR="$stage/install" CONDR_PROFILE="$stage/profile" \
    HOME="$stage/home" sh script/install-condr.sh --from "$1"
"$stage/home/.local/bin/condr" server --help > /dev/null
CONDR_INSTALL_DIR="$stage/install" CONDR_PROFILE="$stage/profile" \
    HOME="$stage/home" sh script/install-condr.sh --from "$1" --yes
"$stage/home/.local/bin/condr" server --help > /dev/null
test "$(cat "$stage/install/condr-gui")" = 'keep GUI'
test "$(grep -Fxc '# condr user bin' "$stage/profile")" = 1
echo 'Linux package content and CLI installation checks passed.'
