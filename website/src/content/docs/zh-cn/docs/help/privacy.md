---
title: 隐私
description: Condr 的桌面端、手机端、网站和服务能看到什么，以及哪些内容只留在你自己的设备上。
---

Condr 没有账号，也没有统计分析。桌面端、Server 与手机端都不会向我们上传任何关于你个人或你工作内容的信息，唯一的例外是你在手机上允许的通知文字。本页列出所有会离开你设备的内容。

---

## 留在你设备上的内容

* **你的工作**：终端、Agent 输出、Workspace、Git 数据和文件，都留在运行它们的 Server 所在的 Device 上。其他 Device 只能通过你建立的连接看到它们，这些连接都是端到端加密的。一条通知只带有关于某个 Agent 的少量信息（见下文 Condr Cloud），从不包含终端内容。
* **Device 密钥**：电脑把密钥存在数据目录的 `device-key` 文件里。iPhone App 把密钥存放在 iOS 钥匙串中，仅本台手机可读取，且不会随备份恢复到其他手机。Android App 用 Android Keystore 中的密钥加密保存它，存放位置不参与备份。密钥从不离开设备。
* **手机端数据**：你配对的电脑、为它们设置的名称以及你的偏好设置，均保存在手机本地。

---

## Condr 运营的服务

* **Peer-to-peer 中继与 DNS**（`relay.condr.dev`、`dns.condr.dev`）：仅用于 Peer-to-peer 连接。中继无法读取你的传输流量；但当中继处在连接路径上时，它能获知通信双方的 Device 标识、连接时间以及来源 IP 地址。中继不会在磁盘上存储任何内容。详见[安全模型](/zh-cn/docs/help/security/)。
* **Condr Cloud**（`push.condr.dev`，运行在 Cloudflare Workers 上）：负责投递手机端的通知。当 Agent 完成、出错或需要你处理时，你的 Server 会请求 Condr Cloud 推送，Condr Cloud 再交给 Apple（Android 上则是 Google）。
  * **存储的内容**：对每台允许通知的手机，保存它的 Device 标识、推送令牌、哪些 Device 可以向它推送，以及它最近一次注册的时间。不保存任何通知。超过 90 天未更新的注册会被删除；Apple 或 Google 报告令牌失效时，该令牌会立即删除。
  * **推送中可见的内容**：Condr Cloud 及运行它的 Cloudflare、Apple 与 Google 能看到 Agent 的名称和状态、Workspace、Agent 给出的简短说明（如有）、终端标题、Server 的名称以及双方的 Device 标识，但看不到终端内容。推送是明文，没有端到端加密。
  * **没有账号**：每个请求都用 Device 密钥签名。
* **更新检查**：桌面端每 5 小时读取一次 `https://condr.dev/version.txt`（若跟随 nightly 渠道则查询 GitHub API），以确认是否有新版本。该请求不会携带任何能够识别你个人或当前安装实例的信息。你可以在 **Settings › About** 中关闭自动更新检查。
* **网站**（`condr.dev`）：托管于 Cloudflare Pages，不包含任何统计分析代码，亦不设置 Cookie。页面字体从 Google Fonts 加载。与其他网页请求一样，Cloudflare 与 Google 会在请求过程中收到你的 IP 地址。

---

## 手机端

* **相机**：只用来扫描电脑显示的配对码，不保存也不发送任何画面。在 Android 上，配对码由 Google Play 服务中的 Google 扫码器读取，App 本身不会获得相机权限；扫码期间，Google 会按照 [Google 隐私权政策](https://policies.google.com/privacy)收集扫码器的诊断信息，例如手机型号、App 版本、一个安装标识和扫码耗时。改为粘贴链接则不会用到扫码器。
* **Face ID、指纹或屏幕锁**：App 锁向 iOS 或 Android 请求验证，Condr 仅接收验证是否通过的结果，无法读取你的生物特征数据。
* **本地网络**：用于在同一 Wi-Fi 下直接连接电脑。
* **连接**：App 仅连接与它配对的电脑；当电脑使用 Peer-to-peer 时，还会连接上述中继与 DNS；你允许通知时，还会连接 Condr Cloud。连接仅在 App 打开时保持。
* **通知**：App 只在你第一次配对后询问一次。如果你拒绝，App 不会连接 Condr Cloud。在手机设置里关闭通知即可停止推送。Android App 暂不支持通知。
* **崩溃报告与 TestFlight 反馈**：如果你在 iOS 设置里选择与开发者共享崩溃数据，或从 TestFlight 发送反馈，Apple 会按照 [Apple 隐私政策](https://www.apple.com/legal/privacy/)把它们转交给我们。如果你从 Google Play 安装 Android App，并选择与 Google 共享使用情况和诊断信息，Google 会按照 [Google 隐私权政策](https://policies.google.com/privacy)以同样方式把崩溃报告转交给我们。

---

## 删除你的数据

Condr 没有账号，因此无需注销。Condr 保存的与你有关的数据按以下方式删除：

* **手机上**：在 App 里忘记某台电脑即可移除它。删除 App 会清除它保存的全部内容，包括 Device 密钥。
* **Condr Cloud 中**：在手机设置里关闭 Condr 的通知，或忘记所有电脑，App 下次打开时就会删除这台手机的注册。超过 90 天未更新的注册会自动删除。如需不通过 App 删除，请发邮件至 [hello@condr.dev](mailto:hello@condr.dev)。
* **中继与 DNS** 不存储任何内容，无需删除。

Apple 或 Google 收集的数据（例如崩溃报告或扫码器的诊断信息）按照它们各自的政策删除。

---

## 联系

隐私相关的问题请发邮件至 [hello@condr.dev](mailto:hello@condr.dev)。本页的每一次修改都记录在[仓库历史](https://github.com/condrdev/condr/commits/main/website/src/content/docs/zh-cn/docs/help/privacy.md)里。
