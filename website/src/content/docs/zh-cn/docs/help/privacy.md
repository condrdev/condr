---
title: 隐私
description: Condr 的桌面端、手机端、网站和服务能看到什么，以及哪些内容只留在你自己的设备上。
---

Condr 没有账号，也没有统计分析。桌面端、Server 与手机端都不会向我们上传任何关于你个人或你工作内容的信息。本页列出所有会离开你设备的内容。

---

## 留在你设备上的内容

* **你的工作**：终端、Agent 输出、Workspace、Git 数据和文件，都留在运行它们的 Server 所在的 Device 上。其他 Device 只能通过你建立的连接看到它们，这些连接都是端到端加密的。
* **Device 密钥**：电脑把密钥存在数据目录的 `device-key` 文件里。iPhone App 把密钥存放在 iOS 钥匙串中，仅本台手机可读取，且不会随备份恢复到其他手机。密钥从不离开设备。
* **手机端数据**：你配对的电脑、为它们设置的名称以及你的偏好设置，均保存在手机本地。

---

## Condr 运营的服务

* **Peer-to-peer 中继与 DNS**（`relay.condr.dev`、`dns.condr.dev`）：仅用于 Peer-to-peer 连接。中继无法读取你的传输流量；但当中继处在连接路径上时，它能获知通信双方的 Device 标识、连接时间以及来源 IP 地址。中继不会在磁盘上存储任何内容。详见[安全模型](/zh-cn/docs/help/security/)。
* **更新检查**：桌面端每 5 小时读取一次 `https://condr.dev/version.txt`（若跟随 nightly 渠道则查询 GitHub API），以确认是否有新版本。该请求不会携带任何能够识别你个人或当前安装实例的信息。你可以在 **Settings › About** 中关闭自动更新检查。
* **网站**（`condr.dev`）：托管于 Cloudflare Pages，不包含任何统计分析代码，亦不设置 Cookie。页面字体从 Google Fonts 加载。与其他网页请求一样，Cloudflare 与 Google 会在请求过程中收到你的 IP 地址。

---

## 手机端

* **相机**：只用来扫描电脑显示的配对码，不保存也不发送任何画面。
* **Face ID**：App 锁向 iOS 请求验证，Condr 仅接收验证是否通过的结果，无法读取你的面容数据。
* **本地网络**：用于在同一 Wi-Fi 下直接连接电脑。
* **连接**：App 仅连接与它配对的电脑；当电脑使用 Peer-to-peer 时，还会连接上述中继与 DNS。连接仅在 App 打开时保持。
* **通知**：目前不发送任何通知。推送通知上线前，本页会提前补充说明负责投递通知的服务与机制。
* **崩溃报告与 TestFlight 反馈**：如果你在 iOS 设置里选择与开发者共享崩溃数据，或从 TestFlight 发送反馈，Apple 会按照 [Apple 隐私政策](https://www.apple.com/legal/privacy/)把它们转交给我们。

---

## 联系

隐私相关的问题请发邮件至 [hello@condr.dev](mailto:hello@condr.dev)。本页的每一次修改都记录在[仓库历史](https://github.com/condrdev/condr/commits/main/website/src/content/docs/zh-cn/docs/help/privacy.md)里。
