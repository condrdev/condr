#!/bin/sh
# Install the Condr headless build (the condr CLI/Server binary) on Linux or macOS.
#   curl -fsSL https://condr.dev/install.sh | sh
#   curl -fsSL https://condr.dev/install.sh | sh -s -- --start
#   curl -fsSL https://condr.dev/install.sh | sh -s -- --force
#   CONDR_VERSION=nightly curl -fsSL https://condr.dev/install.sh | sh
#   CONDR_VERSION=v0.1.0 curl -fsSL https://condr.dev/install.sh | sh
#   sh script/install-condr.sh --from ./condr-headless-<version>-linux-x86_64.tar.gz
# Downloads the archive for this machine from GitHub Releases (or takes --from),
# verifies it against SHA256SUMS, then runs `condr server install` with the
# remaining arguments (ADR 0016). With no other arguments it first compares the
# installed condr with condr.dev/version.txt and stops when the latest release is
# already there; --force reinstalls. Desktop users use a platform package instead.
set -eu

main() {
    repo=${CONDR_REPO:-condrdev/condr}
    version=${CONDR_VERSION:-latest}
    force=
    for arg do
        shift
        if [ "$arg" = --force ]; then force=1; else set -- "$@" "$arg"; fi
    done
    source=
    if [ "${1:-}" = --from ]; then
        source=${2:?--from needs an archive}
        shift 2
    fi

    # Nothing else asked and the latest release already installed: nothing to do.
    installed=${CONDR_INSTALL_DIR:-${HOME:-}/.local/opt/condr}/condr
    if [ -z "$source$force" ] && [ $# -eq 0 ] && [ "$version" = latest ] \
        && [ "$repo" = condrdev/condr ] && [ -x "$installed" ]; then
        latest=$(curl -fsSL https://condr.dev/version.txt 2>/dev/null || true)
        # Only a release prints nothing after its version; a nightly adds `(nightly)`.
        current=$("$installed" --version 2>/dev/null | awk 'NF == 2 {print $2}')
        if [ -n "$latest" ] && [ "${current%%+*}" = "$latest" ]; then
            echo "condr $current is already the latest release; pass --force to reinstall"
            exit 0
        fi
    fi

    stage=$(mktemp -d "${TMPDIR:-/tmp}/condr-install.XXXXXX")
    trap 'rm -rf "$stage"' EXIT INT TERM

    if [ -z "$source" ]; then
        command -v curl >/dev/null 2>&1 || { echo "curl is required" >&2; exit 1; }
        case "$(uname -s)" in
            Linux) platform=linux ;;
            Darwin) platform=macos ;;
            *) echo "unsupported platform: $(uname -s); download a package from https://github.com/$repo/releases" >&2; exit 1 ;;
        esac
        case "$(uname -m)" in
            x86_64|amd64) arch=x86_64 ;;
            aarch64|arm64) arch=arm64 ;;
            *) echo "unsupported architecture: $(uname -m)" >&2; exit 1 ;;
        esac
        case "$version" in
            latest) api="https://api.github.com/repos/$repo/releases/latest" ;;
            *) api="https://api.github.com/repos/$repo/releases/tags/$version" ;;
        esac
        urls=$(curl -fsSL -H 'Accept: application/vnd.github+json' "$api" \
            | sed -n 's/.*"browser_download_url": *"\([^"]*\)".*/\1/p')
        archive_url=$(printf '%s\n' "$urls" | grep "/condr-headless-.*-${platform}-${arch}\.tar\.gz$" | head -n 1)
        sums_url=$(printf '%s\n' "$urls" | grep '/SHA256SUMS$' | head -n 1)
        [ -n "$archive_url" ] && [ -n "$sums_url" ] || {
            echo "no headless build for ${platform}-${arch} in release '$version' of $repo" >&2; exit 1; }
        mkdir "$stage/download"
        source="$stage/download/$(basename "$archive_url")"
        echo "Downloading $(basename "$archive_url")"
        curl -fSL --progress-bar -o "$source" "$archive_url"
        curl -fsSL -o "$stage/download/SHA256SUMS" "$sums_url"
    fi
    [ -f "$source" ] || { echo "archive not found: $source" >&2; exit 1; }
    name=$(basename "$source")

    # A downloaded archive always has SHA256SUMS beside it; a --from archive may.
    checksums=$(dirname "$source")/SHA256SUMS
    if [ -f "$checksums" ]; then
        expected=$(awk -v name="$name" '$2 == name { print $1; exit }' "$checksums")
        [ -n "$expected" ] || { echo "no checksum for $name" >&2; exit 1; }
        if command -v sha256sum >/dev/null 2>&1; then
            actual=$(sha256sum "$source" | awk '{print $1}')
        else
            actual=$(shasum -a 256 "$source" | awk '{print $1}')
        fi
        [ "$actual" = "$expected" ] || { echo "checksum mismatch: $name" >&2; exit 1; }
    fi

    mkdir "$stage/payload"
    tar -xzf "$source" -C "$stage/payload"
    condr="$stage/payload/condr-headless/condr"
    [ -x "$condr" ] || { echo "$name does not contain condr-headless/condr" >&2; exit 1; }
    # `curl | sh` leaves stdin on the pipe; hand install the terminal so it can ask.
    if (true </dev/tty) 2>/dev/null; then
        "$condr" server install "$@" </dev/tty
    else
        "$condr" server install "$@"
    fi
}

main "$@"
