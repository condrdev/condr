#!/bin/sh
# With no argument, check postinstall in temporary homes. Pass a .pkg and CLI
# archive on macOS to also check native installation (intended for a test runner).
set -eu

repo=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
stage=$(mktemp -d "${TMPDIR:-/tmp}/condr-macos-check.XXXXXX")
trap 'rm -rf "$stage"' EXIT INT TERM
mkdir "$stage/installer-home"

snapshot_profiles() {
    for name in .zprofile .bash_profile .bash_login .profile; do
        if [ -f "$1/$name" ]; then cksum "$1/$name"; fi
    done
}

for existing in '' '.profile' '.bash_login .profile' '.bash_profile .bash_login .profile'; do
    target="$stage/user $existing \"quoted\" \$dollar"
    mkdir -p "$target/Applications/Condr.app/Contents/MacOS" "$target/.local/bin"
    printf '#!/bin/sh\nexit 0\n' >"$target/Applications/Condr.app/Contents/MacOS/condr"
    chmod 755 "$target/Applications/Condr.app/Contents/MacOS/condr"
    ln -s ../../Applications/Condr.app/Contents/MacOS/condr "$target/.local/bin/condr"
    original='# existing settings
export CONDR_PROFILE_PRESERVED=yes'
    if [ -n "$existing" ]; then
        for name in .zprofile $existing; do
            printf '%s' "$original" >"$target/$name"
            chmod 600 "$target/$name"
        done
    fi
    HOME="$stage/installer-home" "$repo/packaging/macos/scripts/postinstall" Condr.pkg "$target" / >/dev/null
    before=$(snapshot_profiles "$target")
    HOME="$stage/installer-home" "$repo/packaging/macos/scripts/postinstall" Condr.pkg "$target" / >/dev/null
    test "$before" = "$(snapshot_profiles "$target")"
    selected=${existing%% *}
    selected=${selected:-.profile}
    for name in .zprofile .bash_profile .bash_login .profile; do
        profile="$target/$name"
        [ -f "$profile" ] || continue
        if [ -n "$existing" ]; then
            test "$(head -c "${#original}" "$profile")" = "$original"
            test "$(ls -l "$profile" | cut -c 1-10)" = '-rw-------'
        fi
        if [ "$name" = .zprofile ] || [ "$name" = "$selected" ]; then
            test "$(grep -Fxc '# condr user bin' "$profile")" = 1
            for shell in /bin/bash /bin/zsh; do
                [ -x "$shell" ] || continue
                env -i HOME="$target" PATH=/usr/bin:/bin "$shell" -ec '
                    . "$1"; . "$1"
                    test "$(command -v condr)" = "$HOME/.local/bin/condr"
                    test "$PATH" = "$HOME/.local/bin:/usr/bin:/bin"
                ' check "$profile"
            done
        else
            test "$(cat "$profile")" = "$original"
        fi
    done
done
if "$repo/packaging/macos/scripts/postinstall" Condr.pkg "$stage/installer-home" / >"$stage/error" 2>&1; then
    echo 'postinstall accepted a destination without the CLI' >&2
    exit 1
fi
test -z "$(ls -A "$stage/installer-home")"
echo 'macOS PATH registration checks passed.'

if [ "$#" -gt 0 ]; then
    test "$#" -eq 2
    test "$(uname -s)" = Darwin
    commit=${CONDR_COMMIT:-${GITHUB_SHA:-$(git -C "$repo" rev-parse HEAD)}}
    CONDR_COMMIT="$commit" sh "$repo/script/check-cli-package.sh" "$2" condr-cli
    package=$(CDPATH= cd -- "$(dirname "$1")" && pwd)/$(basename "$1")
    pkgutil --expand "$package" "$stage/package"
    grep -Fq "hostArchitectures=\"$(uname -m)\"" "$stage/package/Distribution"
    for attempt in 1 2; do
        /usr/sbin/installer -pkg "$package" -target CurrentUserHomeDirectory
        test "$(cat "$HOME/Applications/Condr.app/BUILD-COMMIT")" = "$commit"
        after=$(snapshot_profiles "$HOME")
        if [ "$attempt" = 2 ]; then test "$before" = "$after"; fi
        before=$after
        for shell in /bin/bash /bin/zsh; do
            env -i HOME="$HOME" PATH=/usr/bin:/bin:/usr/sbin:/sbin "$shell" -lc '
                test "$(command -v condr)" = "$HOME/.local/bin/condr" && condr server --help
            '
        done
    done
    echo 'macOS package installation and login-shell checks passed.'
fi
