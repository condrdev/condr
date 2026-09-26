---
title: 架构
description: 了解 Condr 的构成，便于阅读或修改代码。
---

如果你要为 Condr 贡献代码，请从这三个 crate 和协议边界开始阅读。

Condr 由三个 Rust crate 组成：

- `condr-core`：领域类型、协议、PTY 和终端模拟、Agent 检测、Git。它不依赖 GUI，因此没有显示器也能测试。
- `condr-server`：构建 `condr` 命令。它负责 Session、终端运行时、持久化和连接。
- `condr-gui`：构建 `condr-gui` 应用。它是 Server 的一个 Client，用 GPUI 渲染。

本机和远程 Client 使用同一协议与 Server 通信。设计决定记录在 [ADR](https://github.com/condrdev/condr/tree/main/docs/adr) 中。
