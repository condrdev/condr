#!/bin/sh
set -eu

: "${CONDR_VERSION:=$(cargo metadata --no-deps --format-version 1 | sed -n 's/.*"name":"condr-server","version":"\([^"]*\)".*/\1/p' | head -n 1)}"
: "${CONDR_COMMIT:?CONDR_COMMIT is required}"
: "${CONDR_ARCH:=$(uname -m)}"
: "${DIST_DIR:=dist}"
case "$CONDR_ARCH" in
    x86_64|amd64) arch=x86_64 ;;
    aarch64|arm64) arch=arm64 ;;
    *) echo "unsupported architecture: $CONDR_ARCH" >&2; exit 1 ;;
esac
package_version=$CONDR_VERSION
if [ "${CONDR_RELEASE:-0}" != 1 ]; then
    package_version="$package_version-$(printf '%s' "$CONDR_COMMIT" | cut -c 1-12)"
fi
mkdir -p "$DIST_DIR"

stage=$(mktemp -d "${TMPDIR:-/tmp}/condr-macos.XXXXXX")
trap 'rm -rf "$stage"' EXIT INT TERM

cli="$stage/condr-headless"
mkdir -p "$cli"
install -m 755 target/release/condr "$cli/condr"
install -m 644 LICENSE "$cli/LICENSE"
printf '%s\n' "$CONDR_COMMIT" >"$cli/BUILD-COMMIT"
tar -C "$stage" -czf "$DIST_DIR/condr-headless-${package_version}-macos-${arch}.tar.gz" condr-headless

app="$stage/dmg/Condr.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
iconutil --convert icns --output "$app/Contents/Resources/condr.icns" assets/brand/condr.iconset
install -m 755 target/release/condr target/release/condr-gui "$app/Contents/MacOS/"
install -m 644 LICENSE "$app/Contents/Resources/LICENSE"
printf '%s\n' "$CONDR_COMMIT" >"$app/Contents/Resources/BUILD-COMMIT"
# The launcher is not called `Condr`: the default case-insensitive APFS would merge it
# with the `condr` CLI binary next to it.
# A drag-and-drop DMG has no installer step, so the launcher registers the CLI on
# every start (`condr server install` is idempotent) and then becomes the GUI.
cat >"$app/Contents/MacOS/condr-launcher" <<'EOF'
#!/bin/sh
set -u
here=$(CDPATH= cd -- "$(dirname "$0")" && pwd)
if "$here/condr" server install >/dev/null 2>&1; then
    export CONDR_SERVER_EXECUTABLE="${CONDR_INSTALL_DIR:-$HOME/.local/opt/condr}/condr"
fi
exec "$here/condr-gui" "$@"
EOF
chmod 755 "$app/Contents/MacOS/condr-launcher"
cat >"$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>condr-launcher</string>
<key>CFBundleIdentifier</key><string>dev.condr.gui</string>
<key>CFBundleName</key><string>Condr</string>
<key>CFBundleIconFile</key><string>condr.icns</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleVersion</key><string>$CONDR_VERSION</string>
<key>CFBundleShortVersionString</key><string>$CONDR_VERSION</string>
</dict></plist>
EOF
# Not notarized (no Apple Developer account yet), but an ad-hoc signature seals
# the bundle (everything must sit under Contents/) so Gatekeeper reports
# "unverified developer" instead of "damaged"; the user allows it once.
# The notification center only serves a process whose signing identifier is the
# bundle identifier, and `condr-gui` is the process, so it is signed under that
# name first. No `--deep` on the bundle: it re-signs nested code and resets that
# identifier to one derived from the file name. The x86_64 linker, unlike the
# arm64 one, leaves `condr` unsigned, so it is signed here too.
codesign --force --sign - "$app/Contents/MacOS/condr"
codesign --force --sign - --identifier dev.condr.gui "$app/Contents/MacOS/condr-gui"
codesign --force --sign - "$app"
ln -s /Applications "$stage/dmg/Applications"
dmg="$DIST_DIR/condr-${package_version}-macos-${arch}.dmg"
# hdiutil sizes an auto-sized image from the source's allocated blocks, which APFS
# clones and sparse files make too small ("No space left on device" while the disk
# is nearly empty), so size it from the apparent content plus a fixed margin.
size_mb=$(( $(find "$stage/dmg" -type f -exec stat -f %z {} + | awk '{ s += $1 } END { print int(s / 1048576) }') + 64 ))
# GitHub's macOS runners occasionally fail `hdiutil create` with "Resource busy";
# retry a few times, and keep its stderr visible so the failure is diagnosable.
for attempt in 1 2 3; do
    rm -f "$dmg"
    if hdiutil create -volname Condr -srcfolder "$stage/dmg" -size "${size_mb}m" -format UDZO "$dmg"; then
        break
    fi
    [ "$attempt" = 3 ] && exit 1
    echo "hdiutil create failed (attempt $attempt), retrying..." >&2
    sleep 5
done
