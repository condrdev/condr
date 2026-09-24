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
# The binary knows its own build: `condr <version>+<12-char commit>` (ADR 0027),
# followed by `(nightly)` exactly when this is not a release.
version_line=$("$payload/condr" --version)
identity=$(printf '%s' "$version_line" | awk '{print $2}')
if [ "${identity##*+}" != "$(printf '%s' "$commit" | cut -c 1-12)" ]; then
    echo "condr --version says '$version_line', not commit $commit" >&2
    exit 1
fi
case "$version_line" in
    *' (nightly)') [ "${CONDR_RELEASE:-0}" != 1 ] ;;
    *) [ "${CONDR_RELEASE:-0}" = 1 ] ;;
esac || { echo "condr --version says '$version_line' with CONDR_RELEASE=${CONDR_RELEASE:-0}" >&2; exit 1; }

CONDR_INSTALL_DIR="$stage/install" CONDR_PROFILE="$stage/profile" \
    HOME="$stage/home" sh "$repo/script/install-condr.sh" --from "$archive"
"$stage/home/.local/bin/condr" server --help > /dev/null
# Updating replaces the CLI without asking and leaves GUI files alone (ADR 0016).
printf '#!/bin/sh\necho previous-cli\n' >"$stage/install/condr"
printf 'keep GUI\n' >"$stage/install/condr-gui"
CONDR_INSTALL_DIR="$stage/install" CONDR_PROFILE="$stage/profile" \
    HOME="$stage/home" sh "$repo/script/install-condr.sh" --from "$archive" </dev/null
"$stage/home/.local/bin/condr" server --help > /dev/null
test "$(cat "$stage/install/condr-gui")" = 'keep GUI'
test "$(grep -Fxc '# condr user bin' "$stage/profile")" = 1
echo 'headless archive content, commit and installation checks passed.'
