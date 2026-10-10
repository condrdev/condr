# Condr Companion

Condr 的 iOS 与 Android App（ADR 0039、0040、0041）。两端界面都是原生的：iOS 用 SwiftUI，最低 iOS 17；Android 用 Jetpack Compose，最低 API 29。连接、密钥、Session 模型和 Agent 状态只有一份 Rust 实现，即 `crates/condr-mobile`，两端各链接这一个库：iOS 是 xcframework 里的静态库，Android 是 JNA 加载的 `libcondr_mobile.so`。

`ci.yml` 在每个 PR 上为两个平台编译这个库；`mobile.yml` 在 `mobile/**`、`crates/**` 或协议变化时生成绑定，并编译 `ios/CondrKit`。

## 环境

- Rust targets：`rustup target add aarch64-apple-ios aarch64-apple-ios-sim aarch64-linux-android x86_64-linux-android`
- iOS：Xcode（`xcode-select` 指向它，或设置 `DEVELOPER_DIR`）
- Android：JDK、Android SDK 与 NDK、`cargo-ndk`；`ANDROID_HOME` 指向 SDK

## 构建库和绑定

```sh
script/build-mobile.sh ios       # ios/CondrKit：condr_mobileFFI.xcframework 与 Swift 绑定
script/build-mobile.sh android   # android/condr/src/main：jniLibs 与 Kotlin 绑定
```

`CONDR_MOBILE_PROFILE=release` 构建优化版本。产物与生成的绑定都不提交。克隆后先手动运行一次 `ios`，Xcode 解析 `CondrKit` 时就需要 xcframework；之后由 Xcode 的预构建脚本和 Gradle 的 `preBuild` 在每次构建前调用它。

只想验证 Swift 包能编译：

```sh
cd ios/CondrKit
xcodebuild -scheme CondrKit -destination 'generic/platform=iOS Simulator' build
```
