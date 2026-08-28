# AGENTS.md

Guidance for coding agents working in this repository.

## 项目定位

**Murmur**(取自 murmuration)— 一个 multi-agent GUI 应用,参考原型是 [herdr](https://github.com/herdrdev/herdr)(纯 TUI 的 multi-agent 编排工具)。

核心定位:**Orca 的易上手 + herdr 的架构(嵌入原生 CLI 作为 agent 后端)− 两者的缺点**。

- herdr:架构对(直接嵌原生 agent CLI),但纯 TUI 上手门槛高
- paseo:需要自己维护对话 GUI,负担重
- orca:webview 实现,卡、重

Murmur 用原生 GUI 解决:跨端、轻量、快。

## 技术栈

- **Rust** + **GPUI**(Zed 的 UI 框架)+ **[gpui-component](https://github.com/longbridge/gpui-component)**(组件库)
- Agent 后端:嵌入原生 agent CLI(如 Claude Code)作为子进程,而非自己实现对话循环

### 技术选型(已定,尽量与 herdr 对齐)

herdr 实际栈(v0.8.2):libghostty-vt(VT,vendor Zig 库)、portable-pty、tokio、interprocess、bincode+serde、ratatui、server/client 架构、Apache-2.0。

- **许可证:Apache-2.0**(与 herdr 一致)。⚠️ 因此 Zed 的 `terminal`/`terminal_view`(GPL-3.0)只能参考思路,禁止复制代码。
- **VT 终端模拟:`alacritty_terminal`**— 纯 Rust、免 Zig/FFI、有 Zed 的 GPUI 渲染先例;herdr 用 libghostty-vt 是 TUI 场景的选择。
- **PTY:`portable-pty`**(对齐 herdr;Unix pty / Windows ConPTY)。
- **异步:server/core 用 tokio**(对齐 herdr);**GUI 用 GPUI 自带 executor**,两者通过协议连接。
- **传输:版本化二进制协议 over transport adapters**。协议使用 `bincode + serde` 长度前缀帧与严格版本握手;本地优先使用 `interprocess` 的 Unix domain socket / Windows named pipe,远程 MVP 通过 SSH stdio bridge 或显式 trusted TCP endpoint 接入同一协议,认证/授权后置。Server 默认只暴露本地私有 endpoint,不监听公网。
- **持久化:bincode + serde**(会话),**TOML**(配置)(对齐)。
- **终端渲染:自研 GPUI element**(项目最大自研件)— gpui-component 无终端组件。
- **布局:gpui-component 的 Dock** → 映射 workspace/tab/pane 模型。
- **Agent 状态检测:** 从 terminal grid 提取底部纯文本快照,按 agent kind 启发式分类 idle/working/blocked/done(herdr 同款机制)。
- **git worktree:shell out 调 `git`**,不引 git2。
- **依赖:gpui 与 gpui-component 均为 git 依赖,锁定 rev**(gpui 不在 crates.io)。

### 架构与工程结构

Murmur 从第一版起采用独立 server/client 架构。local 不是另一种 backend,只是 GUI 在本机发现或启动同一个 `murmur-server` 后连接:

```
crates/murmur-core    # 领域、协议、PTY、VT、agent 检测、Git — 无 GUI 依赖,headless 可测
crates/murmur-server  # 独立进程,拥有 Session、Terminal runtime、持久化与连接
crates/murmur-gui     # 纯 client,连接一个或多个 server,负责 GPUI 渲染
```

GUI 关闭只断开连接。server、PTY、agent 与 Session 继续运行;重新打开 GUI 时优先连接已有本地 server。停止 server 是显式操作。

### 开发环境(双机)

- **Linux server(arm64,headless)**:murmur-core 的全部开发与测试(`cargo test/clippy` 无需显示器)。GUI 无法在此运行。
- **Windows 笔记本**:GUI 原生构建与手动验证(GPUI 不做交叉编译),同时验证 ConPTY 路径。

### 开发阶段兼容性

- 当前项目处于未发布开发阶段。允许破坏性变更,不要求向后兼容。
- 优先选择边界清晰、实现简单的最终设计;不要仅为旧实现保留兼容层、迁移路径或废弃 API。
- 发生破坏性变更时,同步更新仓库内调用方、测试和源文档。只有任务明确要求时才实现旧版本迁移或兼容。

## 常用命令

```bash
cargo build            # 构建
cargo run              # 运行
cargo test             # 全部测试
cargo test <name>      # 单个测试
cargo clippy           # lint
cargo fmt              # 格式化
```

## 架构原则

- GUI 只做编排与呈现,对话/工具循环交给嵌入的 agent CLI 子进程,不重复造轮子
- server 是 Session、PTY、VT、agent 与 Git/worktree runtime 的唯一所有者;GUI 的 local/remote 功能走同一协议。Client 重连先获取 Server/Session 的权威结构快照和各 Pane 的 live terminal view,再订阅增量事件;GUI 关闭不会停止 server 或其子进程。
- 保持轻量:避免 webview、避免不必要的依赖


## Agent skills

### Issue tracker

GitHub Issues(`gh` CLI)。See `docs/agents/issue-tracker.md`.

### Triage labels

默认五标签:needs-triage / needs-info / ready-for-agent / ready-for-human / wontfix。See `docs/agents/triage-labels.md`.

### Domain docs

Single-context:根目录 `CONTEXT.md` + `docs/adr/`。See `docs/agents/domain.md`.
