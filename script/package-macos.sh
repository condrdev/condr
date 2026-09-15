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

cli="$stage/condr-cli"
mkdir -p "$cli"
install -m 755 target/release/condr "$cli/condr"
install -m 644 LICENSE "$cli/LICENSE"
printf '%s\n' "$CONDR_COMMIT" >"$cli/BUILD-COMMIT"
tar -C "$stage" -czf "$DIST_DIR/condr-cli-${package_version}-macos-${arch}.tar.gz" condr-cli

app="$stage/dmg/Condr.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
iconutil --convert icns --output "$app/Contents/Resources/condr.icns" packaging/icons/condr.iconset
install -m 755 target/release/condr target/release/condr-gui "$app/Contents/MacOS/"
install -m 644 LICENSE "$app/LICENSE"
printf '%s\n' "$CONDR_COMMIT" >"$app/BUILD-COMMIT"
# A drag-and-drop DMG has no installer step, so the launcher registers the CLI on
# every start (`condr server install` is idempotent) and then becomes the GUI.
cat >"$app/Contents/MacOS/Condr" <<'EOF'
#!/bin/sh
set -u
here=$(CDPATH= cd -- "$(dirname "$0")" && pwd)
if "$here/condr" server install >/dev/null 2>&1; then
    export CONDR_SERVER_EXECUTABLE="${CONDR_INSTALL_DIR:-$HOME/.local/opt/condr}/condr"
fi
exec "$here/condr-gui" "$@"
EOF
chmod 755 "$app/Contents/MacOS/Condr"
cat >"$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>Condr</string>
<key>CFBundleIdentifier</key><string>dev.condr.gui</string>
<key>CFBundleName</key><string>Condr</string>
<key>CFBundleIconFile</key><string>condr.icns</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleVersion</key><string>$CONDR_VERSION</string>
<key>CFBundleShortVersionString</key><string>$CONDR_VERSION</string>
</dict></plist>
EOF
ln -s /Applications "$stage/dmg/Applications"
hdiutil create -quiet -volname Condr -srcfolder "$stage/dmg" -format UDZO \
  "$DIST_DIR/condr-${package_version}-macos-${arch}.dmg"
