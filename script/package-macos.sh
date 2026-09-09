#!/bin/sh
set -eu

: "${CONDR_VERSION:=$(cargo metadata --no-deps --format-version 1 | sed -n 's/.*"name":"condr-server","version":"\([^"]*\)".*/\1/p' | head -n 1)}"
: "${CONDR_COMMIT:?CONDR_COMMIT is required}"
: "${CONDR_ASSET_ID:=$CONDR_COMMIT}"
: "${CONDR_ARCH:=$(uname -m)}"
: "${DIST_DIR:=dist}"
asset_id=$(printf '%s' "$CONDR_ASSET_ID" | cut -c 1-12)
mkdir -p "$DIST_DIR"

stage=$(mktemp -d "${TMPDIR:-/tmp}/condr-macos.XXXXXX")
trap 'rm -rf "$stage"' EXIT INT TERM

cli="$stage/condr-cli"
mkdir -p "$cli"
install -m 755 target/release/condr "$cli/condr"
install -m 644 LICENSE "$cli/LICENSE"
printf '%s\n' "$CONDR_COMMIT" >"$cli/BUILD-COMMIT"
tar -C "$stage" -czf "$DIST_DIR/condr-macos-${CONDR_ARCH}-${CONDR_VERSION}-${asset_id}-cli.tar.gz" condr-cli

app="$stage/payload/Applications/Condr.app"
mkdir -p "$app/Contents/MacOS" "$stage/payload/.local/bin"
install -m 755 target/release/condr target/release/condr-gui "$app/Contents/MacOS/"
install -m 644 LICENSE "$app/LICENSE"
printf '%s\n' "$CONDR_COMMIT" >"$app/BUILD-COMMIT"
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
sed "s/@VERSION@/$CONDR_VERSION/g" packaging/macos/Distribution.xml >"$stage/Distribution.xml"
pkgbuild --root "$stage/payload" --identifier dev.condr.condr --version "$CONDR_VERSION" \
  --install-location / "$stage/condr-component.pkg"
productbuild --distribution "$stage/Distribution.xml" --package-path "$stage" \
  "$DIST_DIR/condr-macos-${CONDR_ARCH}-${CONDR_VERSION}-${asset_id}.pkg"
