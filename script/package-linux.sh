#!/bin/sh
set -eu

: "${CONDR_VERSION:=$(cargo metadata --no-deps --format-version 1 | sed -n 's/.*"name":"condr-server","version":"\([^"]*\)".*/\1/p' | head -n 1)}"
: "${CONDR_COMMIT:?CONDR_COMMIT is required}"
: "${CONDR_ARCH:=$(uname -m)}"
: "${DIST_DIR:=dist}"
: "${APPIMAGE_TOOL:=appimagetool}"
: "${LINUXDEPLOY_TOOL:=linuxdeploy}"
case "$CONDR_ARCH" in
    x86_64|amd64) arch=x86_64 ;;
    aarch64|arm64) arch=arm64 ;;
    *) echo "unsupported architecture: $CONDR_ARCH" >&2; exit 1 ;;
esac
package_version=$CONDR_VERSION
if [ "${CONDR_RELEASE:-0}" != 1 ]; then
    package_version="$package_version-$(printf '%s' "$CONDR_COMMIT" | cut -c 1-12)"
fi
: "${APPIMAGE_OUTPUT:=$DIST_DIR/condr-${package_version}-linux-${arch}.AppImage}"

stage=$(mktemp -d "${TMPDIR:-/tmp}/condr-linux.XXXXXX")
trap 'rm -rf "$stage"' EXIT INT TERM

cli="$stage/condr-headless"
mkdir -p "$cli"
install -m 755 target/release/condr "$cli/condr"
install -m 644 LICENSE "$cli/LICENSE"
printf '%s\n' "$CONDR_COMMIT" >"$cli/BUILD-COMMIT"
mkdir -p "$DIST_DIR"
tar -C "$stage" -czf "$DIST_DIR/condr-headless-${package_version}-linux-${arch}.tar.gz" condr-headless

appdir="$stage/condr.AppDir"
mkdir -p "$appdir/usr/bin" "$appdir/usr/share/applications"
install -m 755 target/release/condr target/release/condr-gui "$appdir/usr/bin/"
install -m 644 script/condr.desktop "$appdir/usr/share/applications/condr.desktop"
install -m 644 LICENSE "$appdir/LICENSE"
printf '%s\n' "$CONDR_COMMIT" >"$appdir/BUILD-COMMIT"
"$LINUXDEPLOY_TOOL" --appdir "$appdir" \
    --executable target/release/condr \
    --executable target/release/condr-gui \
    --desktop-file script/condr.desktop \
    --icon-file assets/brand/condr.svg
# linuxdeploy creates AppRun as a symlink to the GUI. Replace the link, not its target.
if [ -L "$appdir/AppRun" ]; then
    unlink "$appdir/AppRun"
fi
cat >"$appdir/AppRun" <<'EOF'
#!/bin/sh
set -eu
appdir=${APPDIR:-$(CDPATH= cd -- "$(dirname "$0")" && pwd)}
if [ -n "${APPIMAGE:-}" ] && [ -n "${HOME:-}" ]; then
    root=${CONDR_INSTALL_DIR:-"$HOME/.local/opt/condr"}
    bin_dir="$HOME/.local/bin"
    mkdir -p "$root" "$bin_dir"
    install -m 755 "$appdir/usr/bin/condr" "$root/condr.new"
    mv "$root/condr.new" "$root/condr"
    ln -sfn "$root/condr" "$bin_dir/condr"
    marker='# condr user bin'
    profile=${CONDR_PROFILE:-"$HOME/.profile"}
    if ! grep -Fqx "$marker" "$profile" 2>/dev/null; then
        printf '\n%s\nexport PATH="%s:$PATH"\n' "$marker" "$bin_dir" >>"$profile"
    fi
    export CONDR_SERVER_EXECUTABLE="$root/condr"
fi
exec "$appdir/usr/bin/condr-gui" "$@"
EOF
chmod 755 "$appdir/AppRun"
mkdir -p "$(dirname "$APPIMAGE_OUTPUT")"
"$APPIMAGE_TOOL" "$appdir" "$APPIMAGE_OUTPUT"
