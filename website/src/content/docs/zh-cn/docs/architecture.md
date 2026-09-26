---
title: 架构
description: Condr 内部的结构。
---

Condr 分为三个 crate：

- `condr-core`：领域类型、协议、PTY、VT、Agent 检测和 Git。
- `condr-server`：Session、终端运行时、持久化和连接。
- `condr-gui`：负责渲染和交互的原生客户端。

本机和远程客户端使用同一套协议。
