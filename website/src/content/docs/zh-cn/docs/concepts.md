---
title: 核心概念
description: Condr 背后的几个基本想法。
---

## Server 掌管运行时

Server 是 Workspace、终端和 Agent 状态的唯一来源。客户端连接到它，渲染它报告的内容。本机和远程客户端使用同一套协议。

## Agent 保持原生

Condr 把每个 Agent 自己的命令行工具作为子进程运行。对话和工具调用循环留在你原本使用的 Agent 里，Condr 只显示它的状态，并给它一个终端。

## GUI 只是客户端

关闭窗口只会断开连接。Server、它的终端和你的 Agent 继续运行。再次打开窗口，它会重新连上。
