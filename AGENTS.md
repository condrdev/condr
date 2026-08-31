# AGENTS.md

Guidance for coding agents working in this repository.

## 项目定位

**Condr**(由 conductor 缩写而来)— 一个 multi-agent GUI 应用,参考原型是 [herdr](https://github.com/herdrdev/herdr)(纯 TUI 的 multi-agent 编排工具)。

核心定位:**Orca 的易上手 + herdr 的架构(嵌入原生 CLI 作为 agent 后端)− 两者的缺点**。

- herdr:架构对(直接嵌原生 agent CLI),但纯 TUI 上手门槛高
- paseo:需要自己维护对话 GUI,负担重
- orca:webview 实现,卡、重

Condr 用原生 GUI 解决:跨端、轻量、快。

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

Condr 从第一版起采用独立 server/client 架构。local 不是另一种 backend,只是 GUI 在本机发现或启动同一个 `condr-server` 后连接:

```
crates/condr-core    # 领域、协议、PTY、VT、agent 检测、Git — 无 GUI 依赖,headless 可测
crates/condr-server  # 独立进程,拥有 Session、Terminal runtime、持久化与连接
crates/condr-gui     # 纯 client,连接一个或多个 server,负责 GPUI 渲染
```

GUI 关闭只断开连接。server、PTY、agent 与 Session 继续运行;重新打开 GUI 时优先连接已有本地 server。停止 server 是显式操作。

### 开发环境(双机)

- **Linux server(arm64,headless)**:condr-core 的全部开发与测试(`cargo test/clippy` 无需显示器)。GUI 无法在此运行。
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

### 终端渲染性能要求

终端是 Condr 的核心交互面,流畅度必须接近 herdr/原生终端,不能把卡顿视为可推迟的视觉问题。代表性 agent CLI(尤其 Codex)持续输出、spinner/动画刷新时,鼠标拖选、键盘输入、滚动和 Pane 操作仍需跟手,目标显示节奏为 60 Hz 且不能积压过期帧。

- PTY read chunk 只是终端状态 wakeup,不是必须逐条展示的 GUI frame。Server 必须合并连续 wakeup、发布最新状态并保证尾帧/退出帧不丢;不得让无界旧 `TerminalView` 队列增加输入延迟。
- GUI 消费终端事件时应批量处理并丢弃或覆盖已过期的中间视觉状态;控制、生命周期和布局事件仍必须可靠、有序。
- 终端视觉更新使用独立于可靠 Session event cursor 的 per-client stream:每个 client writer 只有一个可丢弃的批量视觉槽,可靠消息优先。只有视觉帧成功入槽后才能推进该 client 的 baseline;槽满时只记录待刷新的 Pane,writer drain 后必须从 Server 权威 VT 状态重新生成最新帧。Bootstrap 必须清空排队视觉帧并重置 baseline;GUI 检测到 revision gap 时只请求一次新 Bootstrap。
- 小范围终端变化不得使整屏 shaping cache 失效。缓存按 cell/row/run 的实际内容与样式失效;避免逐帧整屏字符串分配、整屏 shaping 和不必要的逐 cell paint。全量 view 成为瓶颈时,优先引入 per-client baseline/damage 增量,同时保持 reconnect bootstrap 正确。
- selection 等纯 GUI 交互必须留在 Client 本地;持续终端输出时也要复用未变化的渲染缓存,不能只优化静止画面。
- 普通开发命令 `cargo run -p condr-gui` 也必须具备可用帧率。不要移除根 `Cargo.toml` 中 GPUI、文本 shaping、VT 和 Condr 热路径的 dev profile 优化,除非有等效替代并完成 Windows 实测。
- 性能相关变更至少覆盖:burst wakeup 合并到最新 revision、尾帧不丢、跨 revision 未变化 cell 不重复 shaping、真实 GPUI 拖选。静止终端上的单次拖选测试不足以证明性能;Windows 验收还需在代表性 agent 动画/高频输出下手动观察交互和帧率。
- 优化前先定位 parse、snapshot/serialization、事件队列、prepaint/shaping、paint 中的实际热点。可以参考 herdr 的 render baseline/frame coalescing;Zed `terminal`/`terminal_view` 仅可参考思路,继续遵守 GPL-3.0 代码禁止复制的许可证边界。


## Agent skills

### Issue tracker

GitHub Issues(`gh` CLI)。See `docs/agents/issue-tracker.md`.

### Triage labels

默认五标签:needs-triage / needs-info / ready-for-agent / ready-for-human / wontfix。See `docs/agents/triage-labels.md`.

### Domain docs

Single-context:根目录 `CONTEXT.md` + `docs/adr/`。See `docs/agents/domain.md`.
