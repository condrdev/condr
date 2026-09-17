#!/bin/sh
# Install the Condr headless build (CLI/Server) on Linux or macOS.
#   curl -fsSL https://condr.dev/install.sh | sh
#   curl -fsSL https://condr.dev/install.sh | sh -s -- --start
#   CONDR_VERSION=v0.1.0 curl -fsSL https://condr.dev/install.sh | sh
# Downloads the archive for this machine from GitHub Releases, verifies SHA256SUMS,
# then runs `condr server install` with any extra arguments.
set -eu

main() {
    repo=${CONDR_REPO:-condrdev/condr}
    version=${CONDR_VERSION:-nightly}
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

    stage=$(mktemp -d "${TMPDIR:-/tmp}/condr-install.XXXXXX")
    trap 'rm -rf "$stage"' EXIT INT TERM
    archive="$stage/$(basename "$archive_url")"
    echo "Downloading $(basename "$archive_url")"
    curl -fSL --progress-bar -o "$archive" "$archive_url"
    curl -fsSL -o "$stage/SHA256SUMS" "$sums_url"

    expected=$(awk -v name="$(basename "$archive")" '$2 == name { print $1; exit }' "$stage/SHA256SUMS")
    [ -n "$expected" ] || { echo "no checksum for $(basename "$archive")" >&2; exit 1; }
    if command -v sha256sum >/dev/null 2>&1; then
        actual=$(sha256sum "$archive" | awk '{print $1}')
    else
        actual=$(shasum -a 256 "$archive" | awk '{print $1}')
    fi
    [ "$actual" = "$expected" ] || { echo "checksum mismatch: $(basename "$archive")" >&2; exit 1; }

    tar -xzf "$archive" -C "$stage"
    condr="$stage/condr-headless/condr"
    [ -x "$condr" ] || { echo "archive does not contain condr-headless/condr" >&2; exit 1; }
    "$condr" server install "$@"
}

main "$@"
