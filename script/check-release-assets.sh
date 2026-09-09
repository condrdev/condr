#!/bin/sh
set -eu

: "${CONDR_VERSION:?CONDR_VERSION is required}"
: "${CONDR_COMMIT:?CONDR_COMMIT is required}"
dist=${1:-dist}
package_version=$CONDR_VERSION
if [ "${CONDR_RELEASE:-0}" != 1 ]; then
    package_version="$package_version-$(printf '%s' "$CONDR_COMMIT" | cut -c 1-12)"
fi

for target in linux-x86_64.AppImage linux-arm64.AppImage \
    macos-x86_64.pkg macos-arm64.pkg windows-x86_64.exe windows-x86_64.zip; do
    test -s "$dist/condr-${package_version}-${target}"
done
for target in linux-x86_64.tar.gz linux-arm64.tar.gz \
    macos-x86_64.tar.gz macos-arm64.tar.gz windows-x86_64.zip; do
    test -s "$dist/condr-cli-${package_version}-${target}"
done
