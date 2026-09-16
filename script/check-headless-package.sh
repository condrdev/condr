#!/bin/sh
# Check a native Unix headless archive and install it in an isolated user directory.
set -eu

archive=${1:?usage: check-headless-package.sh ARCHIVE}
payload_directory=condr-headless
repo=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
commit=${CONDR_COMMIT:-${GITHUB_SHA:-$(git -C "$repo" rev-parse HEAD)}}
stage=$(mktemp -d "${TMPDIR:-/tmp}/condr-headless-check.XXXXXX")
trap 'rm -rf "$stage"' EXIT INT TERM

# Reject missing, duplicate or extra members, including a bundled GUI.
tar -tzf "$archive" >"$stage/members"
LC_ALL=C sort "$stage/members" >"$stage/actual"
printf '%s\n' "$payload_directory/" "$payload_directory/condr" \
    "$payload_directory/LICENSE" "$payload_directory/BUILD-COMMIT" | LC_ALL=C sort >"$stage/expected"
diff -u "$stage/expected" "$stage/actual"
mkdir "$stage/payload"
tar -xzf "$archive" -C "$stage/payload"
payload="$stage/payload/$payload_directory"
test -x "$payload/condr"
test -s "$payload/LICENSE"
if [ "$(cat "$payload/BUILD-COMMIT")" != "$commit" ]; then
    echo 'headless archive has the wrong commit' >&2
    exit 1
fi
"$payload/condr" server --help > /dev/null

CONDR_INSTALL_DIR="$stage/install" CONDR_PROFILE="$stage/profile" \
    HOME="$stage/home" sh "$repo/script/install-condr.sh" --from "$archive"
"$stage/home/.local/bin/condr" server --help > /dev/null
printf '#!/bin/sh\necho previous-cli\n' >"$stage/install/condr"
printf 'keep GUI\n' >"$stage/install/condr-gui"
printf 'n\n' | CONDR_INSTALL_DIR="$stage/install" CONDR_PROFILE="$stage/profile" \
    HOME="$stage/home" sh "$repo/script/install-condr.sh" --from "$archive"
test "$("$stage/install/condr")" = previous-cli
printf 'Yes\n' | CONDR_INSTALL_DIR="$stage/install" CONDR_PROFILE="$stage/profile" \
    HOME="$stage/home" sh "$repo/script/install-condr.sh" --from "$archive"
"$stage/home/.local/bin/condr" server --help > /dev/null
CONDR_INSTALL_DIR="$stage/install" CONDR_PROFILE="$stage/profile" \
    HOME="$stage/home" sh "$repo/script/install-condr.sh" --from "$archive" --yes
"$stage/home/.local/bin/condr" server --help > /dev/null
test "$(cat "$stage/install/condr-gui")" = 'keep GUI'
test "$(grep -Fxc '# condr user bin' "$stage/profile")" = 1
echo 'headless archive content, commit and installation checks passed.'
