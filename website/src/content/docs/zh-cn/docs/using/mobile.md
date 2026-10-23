---
title: 手机端
description: 在 iPhone 上安装 Condr，与你的电脑配对，离开工位时也能查看并回复 Agent。
---

手机版 Condr（iPhone 与 Android）可显示所有已配对电脑上的 Agent，将等待你处理的 Agent 置于最前，并支持打开任意 Pane 进行查看与回复。iPhone 版目前通过 TestFlight 公开测试；Android 版以 APK 形式在 GitHub 上单独发布，首个正式版时上架 Google Play（详见[路线图](/zh-cn/docs/help/roadmap/)）。

---

## 要求

* 运行 iOS 26 或更高版本的 iPhone，或运行 Android 10 或更高版本的 Android 手机。
* 每台要配对的电脑均运行 Condr 0.1.13 或更高版本。

---

## 安装

### iPhone

1. 从 App Store 安装 [TestFlight](https://apps.apple.com/app/testflight/id899247664)。
2. 在 iPhone 上打开 [Condr 测试邀请](https://testflight.apple.com/join/6qQE7Pbx)，轻点 **Accept**，然后轻点 **Install**。

新测试版发布后，可在 TestFlight 中直接更新。每个测试版的有效期为 90 天，通常在此之前就会推出新版本。

### Android

1. 在手机上打开最新的 [Condr Companion 版本](https://github.com/condrdev/condr/releases?q=companion)，下载 `condr-companion-<版本>.apk`。
2. 打开下载的文件，按 Android 的提示允许从浏览器安装。

更新时直接安装更新的 APK 即可：每个 APK 都用同一把密钥签名，手机会保留已有的配对。通知依赖 Google Play 服务；没有它的手机只在 App 打开时保持更新。

---

## 配对电脑

1. 在电脑端的 Condr 中打开 **Settings › Paired devices**。
2. 如果 **Add a phone or computer** 提示其他设备尚无法连接，点击 **Allow connections**，然后点击 **Restart**。这会开启适用于各种网络环境的 Peer-to-peer；重启会停止该电脑上的所有 Pane 与 Agent。若手机能直接访问电脑的 IP 地址，也可以改为在 **Connections** 下开启 **TCP**。
3. 点击 **Show pairing code**。
4. 在手机上轻点 **Pair with Your Computer**，然后轻点 **Scan Code**，并为该电脑设置一个名称。

配对码仅限单次使用，10 分钟后失效。对于未安装桌面应用的电脑，可运行 `condr server invite`，并在手机上轻点 **Paste a Link Instead**；在命令行开启 Peer-to-peer 或 TCP 的具体方法，请参见[远程连接](/zh-cn/docs/using/remote/)。

若暂无可配对的电脑，也可以轻点 **Scan Code** 下方的 **No computer to pair? Try Demo** 体验演示模式。

---

## 使用

* **Agents**：显示所有已配对电脑上的 Agent，顶部为 **Needs you** 列表：汇总处于 Blocked 或失败状态的 Agent，以及它们给出的说明。轻点即可打开对应的 Pane。
* **在 Pane 中**：手机成为该终端的一个新窗口，并根据手机屏幕自动调整终端尺寸。在电脑端的该 Pane 中点击或输入，终端尺寸将恢复为电脑尺寸；在手机上退出该 Pane 后，几秒内也会自动恢复电脑端尺寸。你可以直接使用软键盘输入，键盘上方提供按键排，包含 Escape、Tab、Shift-Tab、Control、方向键、Home、End、Page Up 与 Page Down。滑动可滚动浏览终端内容，轻点可触发点击或打开链接，长按可选中复制。
* **不支持的操作**：手机端不能创建或关闭 Workspace、Tab 与 Pane，亦无法启动 Agent。这些操作请在电脑端完成。
* **App 锁**：App 启动时，或切至后台超过 5 分钟再次返回时，会要求验证身份：iPhone 上是 Face ID、触控 ID 或锁屏密码，Android 上是指纹、人脸或锁屏密码。此锁定默认关闭，可在 App 内的 Settings 中开启。

---

## 限制

* App 仅在打开时保持连接，目前尚未发送通知。Agent 完成或需要你介入时的推送通知是[路线图](/zh-cn/docs/help/roadmap/)中的下一步计划。
* 通过 SSH 连接的电脑无法与手机配对，请使用 Peer-to-peer 或 TCP。
* 仅支持手机，暂无 iPad 或平板布局。

---

## 手机丢了怎么办

在曾经配对过的每台电脑上打开 **Settings › Paired devices**，点击手机旁边的 **Revoke**，或者运行 `condr server revoke <fingerprint 或前缀>`。更换新手机后，需作为新设备重新进行配对。
