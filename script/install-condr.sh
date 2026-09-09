#!/bin/sh
# Install the headless condr CLI/server only; GUI users use a platform package.
set -eu

source=
yes=no
while [ "$#" -gt 0 ]; do
    case "$1" in
        --from) shift; source=${1:?--from needs a file} ;;
        --yes) yes=yes ;;
        -h|--help)
            echo "usage: install-condr.sh [--from CLI_ARCHIVE] [--yes]"
            exit 0
            ;;
        *) echo "unknown option: $1" >&2; exit 2 ;;
    esac
    shift
done

user_home=${HOME:?HOME is not set}
root=${CONDR_INSTALL_DIR:-"$user_home/.local/opt/condr"}
bin_dir="$user_home/.local/bin"
stage=$(mktemp -d "${TMPDIR:-/tmp}/condr-install.XXXXXX")
trap 'rm -rf "$stage"' EXIT INT TERM
downloaded=no

if [ -z "$source" ]; then
    command -v gh >/dev/null 2>&1 || { echo "install gh or pass --from ARCHIVE" >&2; exit 1; }
    arch=$(uname -m)
    case "$arch" in
        x86_64|amd64) arch=x86_64 ;;
        aarch64|arm64) arch=arm64 ;;
        *) echo "unsupported architecture: $arch" >&2; exit 1 ;;
    esac
    case "$(uname -s)" in
        Linux) platform=linux ;;
        Darwin) platform=macos ;;
        *) echo "unsupported platform: $(uname -s)" >&2; exit 1 ;;
    esac
    pattern="condr-cli-*-${platform}-${arch}.tar.gz"
    gh release download "${CONDR_VERSION:-dev}" --repo "${CONDR_REPO:-condrdev/condr}" \
        --dir "$stage/download" --pattern "$pattern" --pattern SHA256SUMS --clobber
    set -- "$stage/download"/condr-cli-*.tar.gz
    [ "$#" -eq 1 ] && [ -f "$1" ] || { echo "expected one CLI archive" >&2; exit 1; }
    source=$1
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

case "$source" in
    *.tar.gz|*.tgz)
        mkdir "$stage/payload"
        tar -xzf "$source" -C "$stage/payload"
        condr=$(find "$stage/payload" -type f -name condr -perm -111 | head -n 1)
        [ -n "$condr" ] || { echo "archive does not contain an executable condr" >&2; exit 1; }
        if [ "$yes" != yes ] && { [ -e "$root/condr" ] || [ -e "$bin_dir/condr" ] || [ -L "$bin_dir/condr" ]; }; then
            printf 'Condr is already installed. Replace the CLI (keep other files)? [y/N] '
            answer=
            read -r answer || true
            case "$answer" in [Yy]|[Yy][Ee][Ss]) ;; *) echo 'Installation cancelled.'; exit 0 ;; esac
        fi
        mkdir -p "$root" "$bin_dir"
        install -m 755 "$condr" "$root/condr.new"
        mv "$root/condr.new" "$root/condr"
        ln -sfn "$root/condr" "$bin_dir/condr"
        ;;
    *) echo "unsupported archive: $source" >&2; exit 2 ;;
esac

profile=${CONDR_PROFILE:-}
if [ -z "$profile" ]; then
    case "$(uname -s)" in
        Darwin) profile="$user_home/.zprofile" ;;
        *) profile="$user_home/.profile" ;;
    esac
fi
mkdir -p "$(dirname "$profile")"
marker='# condr user bin'
if ! grep -Fqx "$marker" "$profile" 2>/dev/null; then
    {
        printf '\n%s\n' "$marker"
        printf '%s\n' 'export PATH="$HOME/.local/bin:$PATH"'
    } >>"$profile"
fi

echo "Condr installed in $root"
echo "Open a new terminal, then run: condr --help"
