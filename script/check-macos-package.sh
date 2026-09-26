#!/bin/sh
# Check the macOS DMG and headless archive natively (intended for a macOS test runner):
# mount the image, copy the app into the current user's home the way drag-and-drop
# would, run the launcher's install step twice, and verify a fresh zsh login shell
# reaches the CLI.
set -eu

test "$#" -eq 2
test "$(uname -s)" = Darwin
repo=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
commit=${CONDR_COMMIT:-${GITHUB_SHA:-$(git -C "$repo" rev-parse HEAD)}}
stage=$(mktemp -d "${TMPDIR:-/tmp}/condr-macos-check.XXXXXX")
trap 'hdiutil detach "$stage/volume" -force >/dev/null 2>&1 || true; rm -rf "$stage"' EXIT INT TERM

CONDR_COMMIT="$commit" sh "$repo/script/check-headless-package.sh" "$2"

hdiutil attach -quiet -nobrowse -readonly -mountpoint "$stage/volume" "$1"
test "$(readlink "$stage/volume/Applications")" = /Applications
app="$stage/volume/Condr.app"
test "$(cat "$app/Contents/Resources/BUILD-COMMIT")" = "$commit"
contents="$app/Contents"
test "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' "$contents/Info.plist")" = condr-launcher
test "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIconFile' "$contents/Info.plist")" = condr.icns
iconutil --convert iconset --output "$stage/condr.iconset" "$contents/Resources/condr.icns"
test -s "$stage/condr.iconset/icon_512x512@2x.png"
sh -n "$contents/MacOS/condr-launcher"
for binary in condr condr-gui; do
    test "$(lipo -archs "$contents/MacOS/$binary")" = "$(uname -m)"
done
codesign --verify --deep --strict "$app"
# Without this the notification center refuses the GUI (see package-macos.sh).
codesign -dv "$contents/MacOS/condr-gui" 2>&1 | grep -qx 'Identifier=dev.condr.gui'

mkdir -p "$HOME/Applications"
cp -R "$app" "$HOME/Applications/"
for attempt in 1 2; do
    "$HOME/Applications/Condr.app/Contents/MacOS/condr" server install
    after=$(cksum "$HOME/.zprofile")
    if [ "$attempt" = 2 ]; then test "$before" = "$after"; fi
    before=$after
    env -i HOME="$HOME" PATH=/usr/bin:/bin:/usr/sbin:/sbin /bin/zsh -lc '
        test "$(command -v condr)" = "$HOME/.local/bin/condr" && condr server --help
    ' >/dev/null
done
echo 'macOS DMG, headless installation and login-shell checks passed.'
