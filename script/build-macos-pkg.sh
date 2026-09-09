#!/bin/sh
set -eu

: "${CONDR_VERSION:=0.1.0}"
: "${MACOS_PKG_OUTPUT:=dist/condr-macos.pkg}"
stage=$(mktemp -d "${TMPDIR:-/tmp}/condr-pkg.XXXXXX")
trap 'rm -rf "$stage"' EXIT INT TERM
app="$stage/payload/Applications/Condr.app"
mkdir -p "$app/Contents/MacOS" "$stage/payload/.local/bin"
install -m 755 target/release/condr target/release/condr-gui "$app/Contents/MacOS/"
cat >"$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>condr-gui</string>
<key>CFBundleIdentifier</key><string>dev.condr.gui</string>
<key>CFBundleName</key><string>Condr</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleVersion</key><string>$CONDR_VERSION</string>
</dict></plist>
EOF
ln -s ../../Applications/Condr.app/Contents/MacOS/condr "$stage/payload/.local/bin/condr"
pkgbuild --root "$stage/payload" --identifier dev.condr.condr --version "$CONDR_VERSION" \
  --install-location / "$stage/condr-component.pkg"
mkdir -p "$(dirname "$MACOS_PKG_OUTPUT")"
productbuild --distribution packaging/macos/Distribution.xml --package-path "$stage" "$MACOS_PKG_OUTPUT"
