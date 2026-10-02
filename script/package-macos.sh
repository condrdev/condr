#!/bin/sh
set -eu

: "${CONDR_VERSION:=$(cargo metadata --no-deps --format-version 1 | sed -n 's/.*"name":"condr-server","version":"\([^"]*\)".*/\1/p' | head -n 1)}"
: "${CONDR_COMMIT:?CONDR_COMMIT is required}"
: "${CONDR_ARCH:=$(uname -m)}"
: "${DIST_DIR:=dist}"
# A Developer ID Application identity in the keychain; `-` is an ad-hoc signature,
# which only seals the bundle (see below) and cannot be notarized.
: "${CONDR_SIGNING_IDENTITY:=-}"
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

# Notarization requires the hardened runtime and a trusted timestamp on every Mach-O;
# an ad-hoc identity can carry neither.
sign_options=
if [ "$CONDR_SIGNING_IDENTITY" != - ]; then
    sign_options='--timestamp --options runtime'
fi
sign() {
    codesign --force --sign "$CONDR_SIGNING_IDENTITY" $sign_options "$@"
}

cli="$stage/condr-headless"
mkdir -p "$cli"
install -m 755 target/release/condr "$cli/condr"
sign "$cli/condr"
install -m 644 LICENSE "$cli/LICENSE"
printf '%s\n' "$CONDR_COMMIT" >"$cli/BUILD-COMMIT"
tar -C "$stage" -czf "$DIST_DIR/condr-headless-${package_version}-macos-${arch}.tar.gz" condr-headless

app="$stage/dmg/Condr.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
iconutil --convert icns --output "$app/Contents/Resources/condr.icns" assets/brand/condr.iconset
install -m 755 target/release/condr target/release/condr-gui "$app/Contents/MacOS/"
install -m 644 LICENSE "$app/Contents/Resources/LICENSE"
printf '%s\n' "$CONDR_COMMIT" >"$app/Contents/Resources/BUILD-COMMIT"
# `condr-gui` is the bundle's main executable, not a script launcher: the notarization
# ticket lists only Mach-O code, so Gatekeeper would keep blocking a script. A
# drag-and-drop DMG has no installer step, so the GUI registers the CLI beside it on
# every start (`install_bundled_cli` in condr-gui's startup).
cat >"$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>condr-gui</string>
<key>CFBundleIdentifier</key><string>dev.condr.gui</string>
<key>CFBundleName</key><string>Condr</string>
<key>CFBundleIconFile</key><string>condr.icns</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleVersion</key><string>$CONDR_VERSION</string>
<key>CFBundleShortVersionString</key><string>$CONDR_VERSION</string>
</dict></plist>
PLIST
# The signature seals the bundle (everything must sit under Contents/); an ad-hoc one
# makes Gatekeeper report "unverified developer" instead of "damaged", so the user
# can allow the app once. The notification center only serves a process whose
# signing identifier is the bundle identifier, which signing the bundle gives its
# main executable `condr-gui`. The nested `condr` is signed first, on its own: the
# x86_64 linker, unlike the arm64 one, leaves it unsigned, and `--deep` would re-sign
# it under an identifier derived from its file name.
sign "$app/Contents/MacOS/condr"
sign "$app"
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
if [ "$CONDR_SIGNING_IDENTITY" != - ]; then
    codesign --force --sign "$CONDR_SIGNING_IDENTITY" --timestamp "$dmg"
fi

# Notarizing the image covers the app inside it. The ticket is stapled to the image;
# a copied app is checked against Apple's servers on its first launch. Credentials
# are a keychain profile (`xcrun notarytool store-credentials`, on a developer's
# machine) or an App Store Connect API key (CI).
notary=
if [ -n "${CONDR_NOTARY_PROFILE:-}" ]; then
    notary="--keychain-profile $CONDR_NOTARY_PROFILE"
elif [ -n "${CONDR_NOTARY_KEY:-}" ]; then
    notary="--key $CONDR_NOTARY_KEY --key-id ${CONDR_NOTARY_KEY_ID:?} --issuer ${CONDR_NOTARY_ISSUER:?}"
fi
if [ -n "$notary" ]; then
    xcrun notarytool submit "$dmg" $notary --wait --no-progress >"$stage/notary.txt" 2>&1 || true
    cat "$stage/notary.txt"
    if ! grep -q '^ *status: Accepted$' "$stage/notary.txt"; then
        id=$(sed -n 's/^ *id: //p' "$stage/notary.txt" | head -n 1)
        [ -z "$id" ] || xcrun notarytool log "$id" $notary >&2
        exit 1
    fi
    xcrun stapler staple "$dmg"
fi
