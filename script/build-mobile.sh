#!/usr/bin/env bash
# Builds condr-mobile, the Companion's one Rust library, and its bindings (ADR 0039).
#
#   script/build-mobile.sh ios       mobile/ios/CondrKit: the xcframework and its Swift
#   script/build-mobile.sh android   mobile/android/condr/src/main: jniLibs and Kotlin
#
# CONDR_MOBILE_PROFILE=release builds optimized. Xcode and Gradle run it before every
# build; Cargo's incremental build makes the unchanged case cheap. Run it once by hand
# after a fresh clone, since the iOS package needs its xcframework before Xcode resolves it.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
profile="${CONDR_MOBILE_PROFILE:-debug}"
release=()
if [ "$profile" = release ]; then
  release=(--release)
fi
target_dir="$root/target"

# The generators run on the host and read the built library's metadata.
bindgen() {
  local tool="$1"
  shift
  (unset SDKROOT IPHONEOS_DEPLOYMENT_TARGET; cargo run --quiet --manifest-path "$root/Cargo.toml" \
    -p condr-mobile --features bindgen --bin "$tool" -- "$@")
}

case "${1:-}" in
ios)
  # The C in ring and blake3 and the Rust around it link for one iOS, the app's oldest.
  export IPHONEOS_DEPLOYMENT_TARGET=17.0
  device=aarch64-apple-ios
  simulator=aarch64-apple-ios-sim
  for target in "$device" "$simulator"; do
    cargo build --manifest-path "$root/Cargo.toml" -p condr-mobile --lib --target "$target" ${release[@]+"${release[@]}"}
  done
  library="$target_dir/$device/$profile/libcondr_mobile.a"
  generated="$target_dir/condr-mobile/swift"
  package="$root/mobile/ios/CondrKit"
  rm -rf "$generated"
  mkdir -p "$generated/headers"
  bindgen uniffi-bindgen-swift --swift-sources "$library" "$generated"
  bindgen uniffi-bindgen-swift --headers --modulemap \
    --module-name condr_mobileFFI --modulemap-filename module.modulemap \
    "$library" "$generated/headers"
  rm -rf "$package/condr_mobileFFI.xcframework"
  xcodebuild -create-xcframework \
    -library "$library" -headers "$generated/headers" \
    -library "$target_dir/$simulator/$profile/libcondr_mobile.a" -headers "$generated/headers" \
    -output "$package/condr_mobileFFI.xcframework" >/dev/null
  cp "$generated/condr_mobile.swift" "$package/Sources/CondrKit/condr_mobile.swift"
  ;;
android)
  module="$root/mobile/android/condr/src/main"
  # Google Play requires 16 KB pages (ADR 0039).
  export CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS="-C link-arg=-Wl,-z,max-page-size=16384"
  export CARGO_TARGET_X86_64_LINUX_ANDROID_RUSTFLAGS="-C link-arg=-Wl,-z,max-page-size=16384"
  cargo ndk --manifest-path "$root/Cargo.toml" -t arm64-v8a -t x86_64 -o "$module/jniLibs" \
    build -p condr-mobile --lib ${release[@]+"${release[@]}"}
  # cargo-ndk copies every shared library the build produced, iroh's own cdylibs among
  # them; the app loads one (ADR 0039).
  find "$module/jniLibs" -name '*.so' ! -name libcondr_mobile.so -delete
  rm -rf "$module/kotlin"
  bindgen uniffi-bindgen generate --library "$module/jniLibs/arm64-v8a/libcondr_mobile.so" \
    --language kotlin --out-dir "$module/kotlin"
  ;;
*)
  echo "usage: $0 ios|android" >&2
  exit 2
  ;;
esac
