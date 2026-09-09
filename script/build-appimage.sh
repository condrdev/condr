#!/bin/sh
set -eu

: "${CONDR_VERSION:=dev}"
: "${APPIMAGE_TOOL:=appimagetool}"
: "${APPIMAGE_OUTPUT:=dist/condr-linux.AppImage}"
appdir=$(mktemp -d "${TMPDIR:-/tmp}/condr-appdir.XXXXXX")
trap 'rm -rf "$appdir"' EXIT INT TERM
mkdir -p "$appdir/usr/bin" "$appdir/usr/share/applications"
install -m 755 target/release/condr target/release/condr-gui "$appdir/usr/bin/"
install -m 644 script/condr.desktop "$appdir/usr/share/applications/condr.desktop"
cat >"$appdir/AppRun" <<'EOF'
#!/bin/sh
set -eu
appdir=${APPDIR:-$(CDPATH= cd -- "$(dirname "$0")" && pwd)}
if [ -n "${APPIMAGE:-}" ] && [ -n "${HOME:-}" ]; then
    root=${CONDR_INSTALL_DIR:-"$HOME/.local/opt/condr"}
    bin_dir="$HOME/.local/bin"
    mkdir -p "$root" "$bin_dir"
    # Keep Server and hooks on a stable path after the AppImage mount disappears.
    install -m 755 "$appdir/usr/bin/condr" "$root/condr"
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
printf '%s\n' "$CONDR_VERSION" >"$appdir/BUILD-VERSION"
mkdir -p "$(dirname "$APPIMAGE_OUTPUT")"
"$APPIMAGE_TOOL" "$appdir" "$APPIMAGE_OUTPUT"
