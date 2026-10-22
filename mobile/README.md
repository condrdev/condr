# Condr Companion

Condr 的 iOS 与 Android App（ADR 0039、0040、0041）。两端界面都是原生的：iOS 用 SwiftUI，最低 iOS 26（ADR 0042）；Android 用 Jetpack Compose，最低 API 29。连接、密钥、Session 模型和 Agent 状态只有一份 Rust 实现，即 `crates/condr-mobile`，两端各链接这一个库：iOS 是 xcframework 里的静态库，Android 是 JNA 加载的 `libcondr_mobile.so`。

`ci.yml` 在每个 PR 上为两个平台编译这个库；`mobile.yml` 在 `mobile/**`、`crates/**` 或协议变化时生成绑定，编译 `ios/CondrKit`，并在模拟器上跑 iOS App 的测试。

## 环境

- Rust targets：`rustup target add aarch64-apple-ios aarch64-apple-ios-sim aarch64-linux-android x86_64-linux-android`
- iOS：Xcode 26（`xcode-select` 指向它，或设置 `DEVELOPER_DIR`）；`mise use -g xcodegen` 装 XcodeGen
- Android：JDK 17 或更新（本机用 25）、Android SDK（platform 37.2、build-tools 36.0.0）与 NDK、`cargo-ndk`；`ANDROID_HOME` 指向 SDK

## 构建库和绑定

```sh
script/build-mobile.sh ios       # ios/CondrKit：condr_mobileFFI.xcframework 与 Swift 绑定
script/build-mobile.sh android   # android/condr/src/main：jniLibs 与 Kotlin 绑定
```

`CONDR_MOBILE_PROFILE=release` 构建优化版本。产物与生成的绑定都不提交。克隆后先手动运行一次 `ios`，Xcode 解析 `CondrKit` 时就需要 xcframework；之后 Xcode 工程的预构建脚本在每次构建前调用它。

只想验证 Swift 包能编译：

```sh
cd ios/CondrKit
xcodebuild -scheme CondrKit -destination 'generic/platform=iOS Simulator' build
```

## iOS App

`ios/project.yml` 是工程的来源，`xcodegen generate` 生成 `Condr.xcodeproj`（不提交）。工程有三个 target：`Condr`、`CondrTests`（Swift Testing）和 `CondrUITests`（XCUITest），都是同步文件夹，新文件不用重新生成。依赖本地包 `CondrKit` 和 [SwiftTerm](https://github.com/migueldeicaza/SwiftTerm)（终端渲染器，钉在 1.99.0）。

```sh
cd ios
xcodegen generate
xcodebuild build -project Condr.xcodeproj -scheme Condr \
  -destination 'platform=iOS Simulator,name=iPhone 17' -skipPackagePluginValidation
xcodebuild test -project Condr.xcodeproj -scheme Condr \
  -destination 'platform=iOS Simulator,name=iPhone 17' -skipPackagePluginValidation
xcrun swift-format lint --strict --recursive Condr CondrTests CondrUITests
```

SwiftTerm 带一个构建插件，命令行构建要加 `-skipPackagePluginValidation`；Xcode 里第一次构建时点一次信任即可。模拟器构建别关掉签名（`CODE_SIGNING_ALLOWED=NO`），Keychain 测试需要临时签名带上的 entitlements。

连真实 Server 的端到端测试 `LivePairingUITests` 默认跳过；给它一个邀请就会在模拟器上走完配对、打开 Pane、输入命令、Forget：

```sh
TEST_RUNNER_CONDR_INVITE='tcp://…@127.0.0.1:7418' TEST_RUNNER_CONDR_DEVICE_NAME="$(hostname)" \
  xcodebuild test … -only-testing:CondrUITests/LivePairingUITests
```

### 结构

- `Condr/App`：入口、各 store 的组装（`Stores`）、生命周期、锁与遮罩的窗口、横幅与提示
- `Condr/Model`：`Session`（库的状态与变更）、`Router`、`Preferences`、`AppLock`、`Alerts`、CondrKit 类型的展示扩展，以及演示世界 `Demo.swift`
- `Condr/Persistence`：Keychain 里的 Device 密钥、app group 里的配对列表
- `Condr/Views`：Agents 首页与 Score、Devices、Settings、配对流程
- `Condr/Terminal`：`ScreenEncoder` 把 Server 的帧按行差分编成 ANSI，`TerminalHost` 喂给 SwiftTerm，`KeyRow` 和 `TerminalPage`
- `Condr/Theme`：颜色、Bars、标与头像、Logo、主按钮

### 演示场景

首次配对的二维码说明页在 **Scan Code**、**Paste a Link Instead** 下方提供 **No computer to pair? Try Demo** 入口；已配对用户从 Devices 页添加电脑时不显示。点击演示入口后可浏览示例 Agents、Devices 和终端，顶部始终注明示例数据并提供 **Exit Demo**；退出或点击 **Pair a Computer** 会关闭演示，回到真实配对页。演示独立保存临时偏好，不连接真实设备，也不修改已有配对。当前沿用设计评审的静态场景，终端输入不执行命令，不代表完整的联网审核流程。

`-scene <name>` 启动参数让 App 跑在一个静止的演示世界上，供设计评审与 UI 测试：`welcome` `pair` `scan` `name` `connecting` `paired` `agents` `lock` `devices` `device` `offline` `terminal` `terminal-error` `question` `codex` `shell` `settings` `empty` `banner` `error` `computer`，以及一台电脑跑 15 个 Agent 的 `crowd` `crowd-device` 和五台电脑的 `many`。`-focus YES` 打开终端后聚焦回复框，`-offset <pt>` 让首页一开始就滚过一段，`-scrub "<设备序号>:<条序号>"` 定住一次 Score 的滑动。

### 真机

模拟器没有相机，扫码页会退回到粘贴链接。真机运行在 Xcode 里选自己的开发团队即可，`project.yml` 里的团队是作者的。

## Android App

`android/` 是一个 Gradle 工程（Kotlin DSL，版本目录在 `gradle/libs.versions.toml`）：`app` 是 App，`condr` 是库模块，它的 `preBuild` 会先跑 `script/build-mobile.sh android`，所以 Gradle 构建永远链接当前的库。界面按 ADR 0048：Jetpack Compose 与 Material 3 Expressive（取自 `material3` 1.5.0 的 beta，钉死版本，不开全局 opt-in），Navigation 3，没有 ViewModel，Application 级的 `Session` 把库的状态投影给 Compose；终端由 `ui/terminal/TerminalCanvas.kt` 自绘 Server 的格子。

```sh
cd android
./gradlew :app:installDebug                                   # 装到已连接的设备或模拟器
./gradlew :app:testDebugUnitTest :app:lintDebug :app:ktfmtCheck
./gradlew ktfmtFormat                                         # 格式化（Google 风格）
./gradlew :app:assembleRelease -Prelease -PversionCode=N      # Rust release 库 + R8；有 ANDROID_KEYSTORE_FILE/ANDROID_KEYSTORE_PASSWORD/ANDROID_KEY_PASSWORD 时用上传密钥签名，否则 debug 密钥
```

推送要 `app/google-services.json`（不提交；没有它也能构建，App 显示通知不可用）。`compileSdk` 是 37.2（Compose 1.13 的 beta 需要），`targetSdk` 36，`minSdk` 29。

### 结构

- `CompanionApplication` / `MainActivity` / `Stores`：进程级的对象装配、前后台与网络回调、通知点开进 Pane
- `model/`：`Session`（库的状态与变更）、`Router`、`Preferences`（Preferences DataStore）、`AppLock`（系统的 BiometricPrompt）、`Alerts`、`Push`（Condr Cloud 登记）
- `persistence/`：Keystore 包裹的 Device 密钥、Proto DataStore 里的配对列表
- `demo/`：演示世界与 `DemoClient`，和 iOS 的 `Demo.swift` 一一对应
- `push/`：通知渠道与 FCM 服务
- `ui/`：`agents`（首页与 Score）、`devices`、`settings`、`pairing`、`terminal`（Canvas 渲染、键排、输入栏）、`theme`

### 演示场景

和 iOS 同一份场景名单，经 Intent extra 启动：

```sh
adb shell am start -n dev.condr.companion/.MainActivity -e scene agents
adb shell am start -n dev.condr.companion/.MainActivity -e scene agents -e scrub 0:1   # 定住一次 Score 的滑动
adb shell am start -n dev.condr.companion/.MainActivity -e scene terminal -e focus true
```

`scan` 在 Android 上只到配对页：扫码是 Google Play services 的系统界面，没有 Play services 的手机只能粘贴链接。

### 测试

JVM 单测只测纯 Kotlin（`Push` 的登记逻辑对假发送器）。模拟器上的 `connectedDebugAndroidTest` 跑每个演示场景（`ScenesTest`）和终端渲染器对位图的检查（`RendererTest`：格子尺寸、宽字符占两列、选区与光标的颜色）。连真实 Server 的端到端配对测试（`LivePairingTest`）只在给了邀请时才进入运行，它在模拟器上走完配对并看到 Agents 页（App 须未配对，`adb uninstall dev.condr.companion`；模拟器经 `10.0.2.2` 访问宿主机）：

```sh
./gradlew :app:connectedDebugAndroidTest \
  -Pandroid.testInstrumentationRunnerArguments.invite='tcp://…@10.0.2.2:7419' \
  -Pandroid.injected.androidTest.leaveApksInstalledAfterRun=true
```

Agent 的标由 `script/generate-android-agent-marks.py` 从 `assets/agents/` 生成到 `app/src/main/res/drawable/agent_*.xml`，改了标就重跑它。终端字体是 JetBrains Mono（OFL，`res/raw/ofl_jetbrains_mono.txt`）。
