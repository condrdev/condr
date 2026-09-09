#!/bin/sh
set -eu

source=
while [ "$#" -gt 0 ]; do
    case "$1" in
        --cli-only) ;;
        --from) shift; source=${1:?--from needs a file} ;;
        -h|--help)
            echo "usage: install-condr.sh [--cli-only] [--from ARCHIVE]"
            exit 0
            ;;
        *) echo "unknown option: $1" >&2; exit 2 ;;
    esac
    shift
done

home=${HOME:?HOME is not set}
root=${CONDR_INSTALL_DIR:-"$home/.local/opt/condr"}
bin_dir="$home/.local/bin"
stage=$(mktemp -d "${TMPDIR:-/tmp}/condr-install.XXXXXX")
trap 'rm -rf "$stage"' EXIT INT TERM
downloaded=no

if [ -z "$source" ]; then
    command -v gh >/dev/null 2>&1 || { echo "install gh or pass --from ARCHIVE" >&2; exit 1; }
    arch=$(uname -m)
    case "$arch" in
        x86_64|amd64) arch=x86_64 ;;
        aarch64|arm64) arch=aarch64 ;;
        *) echo "unsupported architecture: $arch" >&2; exit 1 ;;
    esac
    case "$(uname -s)" in
        Linux) pattern="condr-linux-${arch}-*.tar.gz" ;;
        Darwin) pattern="condr-macos-${arch}-*.tar.gz" ;;
        *) echo "unsupported platform: $(uname -s)" >&2; exit 1 ;;
    esac
    gh release download "${CONDR_VERSION:-dev}" --repo "${CONDR_REPO:-condrdev/condr}" \
        --dir "$stage/download" --pattern "$pattern" --pattern SHA256SUMS --clobber
    source=$(find "$stage/download" -type f ! -name SHA256SUMS -print -quit)
    downloaded=yes
fi
[ -f "$source" ] || { echo "archive not found: $source" >&2; exit 1; }
source=$(cd "$(dirname "$source")" && pwd)/$(basename "$source")

checksum_file=$(dirname "$source")/SHA256SUMS
if [ "$downloaded" = yes ] || [ -f "$checksum_file" ]; then
    [ -f "$checksum_file" ] || { echo "SHA256SUMS is missing" >&2; exit 1; }
    expected=$(awk -v name="$(basename "$source")" '$2 == name { print $1; exit }' "$checksum_file")
    [ -n "$expected" ] || { echo "no checksum for $(basename "$source")" >&2; exit 1; }
    if command -v sha256sum >/dev/null 2>&1; then
        actual=$(sha256sum "$source" | awk '{print $1}')
    else
        actual=$(shasum -a 256 "$source" | awk '{print $1}')
    fi
    [ "$actual" = "$expected" ] || { echo "checksum mismatch: $source" >&2; exit 1; }
fi

mkdir -p "$root" "$bin_dir"

case "$source" in
    *.tar.gz|*.tgz)
        rm -rf "$stage/payload"
        mkdir "$stage/payload"
        tar -xzf "$source" -C "$stage/payload"
        condr=$(find "$stage/payload" -type f -name condr -perm -111 | head -n 1)
        [ -n "$condr" ] || { echo "archive does not contain an executable condr" >&2; exit 1; }
        install -m 755 "$condr" "$root/condr.new"
        mv "$root/condr.new" "$root/condr"
        ln -sfn "$root/condr" "$bin_dir/condr"
        ;;
    *) echo "unsupported archive: $source" >&2; exit 2 ;;
esac

profile=${CONDR_PROFILE:-}
if [ -z "$profile" ]; then
    case "$(uname -s)" in
        Darwin) profile="$home/.zprofile" ;;
        *) profile="$home/.profile" ;;
    esac
fi
mkdir -p "$(dirname "$profile")"
marker='# condr user bin'
if ! grep -Fqx "$marker" "$profile" 2>/dev/null; then
    {
        printf '\n%s\n' "$marker"
        printf 'export PATH="%s:$PATH"\n' "$bin_dir"
    } >>"$profile"
fi

echo "Condr installed in $root"
echo "Open a new terminal, then run: condr --help"
