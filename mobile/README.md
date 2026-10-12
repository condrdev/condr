# Condr Companion

Condr 的 iOS 与 Android App（ADR 0039、0040、0041）。两端界面都是原生的：iOS 用 SwiftUI，最低 iOS 26（ADR 0042）；Android 用 Jetpack Compose，最低 API 29。连接、密钥、Session 模型和 Agent 状态只有一份 Rust 实现，即 `crates/condr-mobile`，两端各链接这一个库：iOS 是 xcframework 里的静态库，Android 是 JNA 加载的 `libcondr_mobile.so`。

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

## iOS 原型

`ios/Condr.xcodeproj` 是 Companion 的可交互原型：界面、流程和交互都是正式 App 的设计，数据是 `Condr/Model.swift` 里的演示世界（三台 Device、六个 Agent），暂不链接 CondrKit。用 Xcode 打开后选 iPhone 模拟器运行即可，不需要先构建 Rust 库。

- 首次启动：欢迎 → 配对码说明 → 扫码 → 给手机命名 → 局域网权限说明 → 配对成功。
- 日常：Agents 首页顶部是 Score，每台电脑一列竖条，每个 Agent 一根，按紧急程度排序，点一列打开那台电脑；下面是 Needs you、Done、Working、Idle。Devices 列出每台电脑和它的迷你 Score，设备页顶部是这台电脑的 Score。进入 Pane 后可以用按键行和回复框操作，演示 Agent 会按输入推进。
- 大约 16 秒后 Codex 完成、38 秒后另一个 Claude 需要你，用来演示应用内横幅。

调试用的启动参数：`-scene <name>` 直接打开某一屏（`welcome` `pair` `scan` `name` `network` `connecting` `paired` `lock` `devices` `device` `offline` `terminal` `question` `codex` `shell` `settings` `empty` `banner` `error` `computer`，以及一台电脑跑 15 个 Agent 的 `crowd` `crowd-device`），`-static YES` 关掉自动推进，`-focus YES` 打开终端后聚焦回复框，`-press "down enter t:Ship_it"` 打开终端后依次输入。
