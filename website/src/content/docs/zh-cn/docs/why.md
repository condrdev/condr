---
title: 为什么选 Condr
description: 判断 Condr 是否适合你的工作方式，并了解它与其他工具的区别。
---

用这一页判断 Condr 是否适合你运行 Agent 的方式。

## Condr 能帮你解决的问题

- 三个终端窗口跑三个 Agent，你看不出哪个在等批准。
- 合上笔记本后，Agent 正在执行的任务中断。
- 你在公司服务器上用 tmux 跑 Agent，却无法用手机查看。
- 每个 Agent 厂商都有自己的远程控制，而你使用多个厂商的 Agent。

## 关掉窗口后任务仍会运行

- Server 会继续运行，窗口只是视图。关掉窗口或断开网络后，Agent 仍会工作。
- 再打开窗口即可继续查看。侧栏会显示你离开时完成的 Agent。
- 每台机器运行一个 Server。本机和远程使用同一种方式。

## 使用已有的 Agent

- Condr 运行厂商自己的 CLI：Claude Code、Codex、OpenCode 和另外七种。
- Condr 不改变对话，不代理模型调用，也不接触你的订阅和 API key。
- 每个 Pane 都是真实终端，不是聊天框。任何终端程序都能在其中运行。

## 一眼看到谁需要你

- Agent 的 hook 报告状态，Condr 不从屏幕文字推断状态。
- Agent 等待批准时，侧栏显示它在等待的命令或问题。
- 侧栏顶部的列表汇总所有 Device 上等待你处理的 Agent。
- Agent 完成或需要你时，系统通知会提醒你。

## 一个窗口查看所有机器

- 通过 SSH、TCP 或 Peer-to-peer 连接。
- 两台都在路由器后的机器可以直连，不需要固定 IP 或 VPN。
- 每种连接都加密。TCP 和 Peer-to-peer 使用 WireGuard 风格的握手，并用一次性 invite 配对。

## 使用原生桌面应用

- Condr 启动快，Agent 持续输出时仍保持响应，安装包也很小。
- macOS、Windows 和 Linux 使用同一个产品，没有哪个是移植版。
- Condr 用 Rust 编写，用 GPUI 渲染，不使用 Electron 或 webview。

## 与其他方案比较

| 方案 | 擅长 | Condr 的不同 |
| --- | --- | --- |
| tmux + ssh | 到处可用，键盘操作快 | 提供图形界面和鼠标，可见 Agent 状态，不用自己配置连接 |
| Herdr | 终端里的 Agent 运行时，设计相近 | 原生图形界面，Windows 是一等平台，自带加密配对和 Git 侧栏 |
| 厂商自己的远程控制 | 与自家 Agent 深度集成 | 跨厂商，在一个窗口查看多台机器 |
| Agent 编排框架 | 复杂的多 Agent 工作流 | 不替你编排。Agent 使用 `condr` 命令互相调用 |

Condr 的 Agent 检测和布局语义借鉴了 Herdr。

## 了解 Condr 不做什么

- 不是 Agent，不和模型对话。
- 不是编辑器，只显示 diff 和文件，不修改它们。
- 不是模型代理，从不处理你的请求或密钥。
- 不是网页应用，没有浏览器版本，也没有账号。
