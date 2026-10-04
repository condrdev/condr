---
title: 架构
description: Condr 由哪些进程和 crate 组成、终端画面与 Agent 状态如何流动，以及参与开发时从哪里读起。
---

Condr 把运行时和界面分成两个进程。Server 托管终端、Agent 和 Session，窗口只是连接到 Server 的一个客户端。本文档面向计划阅读或修改 Condr 代码的开发者。

---

## 进程模型

* **每台机器一个 Server**：Server 始终监听本机的私有 socket（Windows 上为命名管道）。配置 `[server] listen` 后，Server 增开一个 TCP 监听端口；开启 Peer-to-peer 时，Server 同时接受 P2P 连接。所有连接入口均访问同一个 Session。
* **Server 与命令行属于同一个二进制程序**：`condr server …` 负责管理 Server，其余子命令均作为客户端连接 Server。
* **窗口作为独立客户端运行**：`condr-gui` 启动时首先检测本机 Server。若未收到响应，则以后台进程启动 `condr server run --detached`。关闭窗口仅断开连接，Server、终端和 Agent 保持运行。
* **窗口单实例限制**：窗口启动时会锁定运行时目录下的 `condr.lock`。若锁已被占用，新启动的实例直接退出。

---

## Crate

| Crate | 构建产物 | 职责 |
| :--- | :--- | :--- |
| `condr-core` | 库 | 领域模型、协议、PTY 与终端模拟、Agent 识别、Git。不依赖 GUI，无显示器环境下亦可测试 |
| `condr-server` | `condr` | Server 进程与命令行。管理 Session、终端运行时、持久化状态及所有连接方式 |
| `condr-gui` | `condr-gui` | 窗口程序。使用 GPUI Kit 渲染，并以库形式复用 `condr-server` 的客户端连接代码 |

---

## 终端画面

1. Server 调用 `portable-pty` 创建 PTY。Windows 环境使用 ConPTY，各终端进程归入独立的 Job Object 管理，关闭 Pane 时关联进程一并结束。
2. PTY 输出进入终端模拟器前先经过 OSC 扫描：OSC 7 等序列用于更新当前工作目录，OSC 777 中的 Agent 事件被提取并从原始输出中剥离。
3. 剩余字节交由 `alacritty_terminal` 解析。
4. Server 采用 60 Hz 频率合并唤醒并读取一次变动区域：整屏变动时生成完整画面，局部变动时仅生成发生变化的单元格。
5. Server 结合各客户端现存的画面生成对应的帧，经由协议发送。
6. 窗口使用自定义 GPUI 元素绘制终端界面，并缓存未变化内容的排版计算结果。

---

## 可靠事件与画面帧

* **可靠事件**：布局变动、生命周期、Agent 状态及 Git 变更按序发送，带有重放游标，确保不丢失。
* **画面帧**：终端画面通过独立通道传输。每个客户端仅分配一个可丢弃的画面缓冲区槽位，可靠消息享有优先发送权。当画面槽位占满时，Server 仅记录需要刷新的 Pane 标识；待槽位释放后，直接基于最新终端状态生成新帧，不重放过期的历史画面。
* **重连机制**：窗口建立连接后，先请求 Bootstrap 快照（包含完整结构快照与各 Pane 当前画面），完成同步后再订阅实时事件。若检测到事件序列存在缺口，窗口重新请求一次 Bootstrap。

---

## Agent 状态

1. 运行 `condr agent hooks install` 将 Hook 写入对应 Agent 的自身配置。
2. Agent 触发 Hook 时执行 `condr agent-hook <agent> <event>`。该命令读取 Agent 输出的 JSON 数据，将其转化为 OSC 777 序列写回当前终端。
3. Server 从 PTY 输出流中拦截该序列，据此更新对应 Pane 的 Agent 状态为 Unknown、Idle、Working 或 Blocked。处于 Blocked 状态时可附带 `blocked_on` 字段，说明等待条件。
4. 若 Agent 未曾上报数据，状态保持为 Unknown。Condr 不通过分析屏幕文本推断 Agent 状态。

Done 状态仅在窗口层维护：当 Agent 状态转为 Idle 且当前焦点处于其他位置时，窗口将其呈现为 Done。

---

## 连接方式

| 方式 | 实现机制 |
| :--- | :--- |
| 本机 | 基于 `interprocess` 提供的 Unix domain socket 或 Windows 命名管道 |
| TCP | 基于 `snow` 实现的 `Noise_IKpsk2_25519_ChaChaPoly_BLAKE2s`，通信双方通过静态密钥双向认证，新设备采用一次性 invite 密钥完成配对 |
| SSH | 调用系统 `ssh -T` 命令，在远端设备执行 `condr server bridge`，将远端私有 socket 转发至本机。认证与传输加密完全由 OpenSSH 处理 |
| Peer-to-peer | 基于 `iroh` 提供的 QUIC 连接，按设备公钥直接拨号并进行 NAT 打洞；点对点直连失败时经由 `relay.condr.dev` 转发中继 |

执行 `condr --device <名称>` 可直接连接配置项 `[[client.servers]]` 中记录的设备，不经过本机 Server 转发。

---

## 协议

* **数据编码**：数据帧采用 Protocol Buffers 格式，使用 varint 长度前缀做消息分包，单帧最大限制为 2 MiB。协议定义文件位于 `proto/condr/v1/`，由 `condr-core` 的 `build.rs` 编译。生成的结构仅在 `protocol::pb` 内部使用，并在模块边界处映射为系统领域类型。
* **握手流程**：客户端发起 `ClientHandshake` 请求，Server 返回 `Welcome` 响应（握手失败时包含拒绝原因）。握手消息的 Protobuf 字段编号保持向后兼容且固定不变，双方在握手阶段互报各自的构建版本号。
* **版本兼容**：双方连接时各自声明能接受的对方最低协议版本。协议字段与 oneof 联合体字段仅允许追加，禁止移除；解析到未识别的枚举值时，按 `.proto` 定义的默认值处理；解析到未识别的 oneof 成员时，丢弃对应消息且不断开连接。两端构建版本不一致时，窗口仅显示警告标记。

---

## 持久化

| 文件 | 格式 | 内容说明 |
| :--- | :--- | :--- |
| `condr-server-<id>.snapshot` | Protocol Buffers | Session 核心结构：Workspace、Tab、Pane 布局及其分割比例、工作目录、Agent 类型与会话恢复 ID、Worktree 映射关系。不包含终端历史回滚数据 |
| `condr-gui.state` | Protocol Buffers | 窗口位置坐标、侧边栏宽度，以及各设备与 Workspace 的视图展示状态。仅供窗口读取，Server 不访问此文件 |
| `config.toml` | TOML | 窗口、Server 与命令行共享的配置文件。所有读写操作均通过 `condr_core::{read_config_value, update_config_values}` 处理，具备跨进程文件锁保护并保留原有排版格式 |

---

## 主要依赖

| 依赖项 | 作用 |
| :--- | :--- |
| `gpui-kit` | GPUI 框架集成、UI 组件、Dock 布局引擎与资源管理 |
| `alacritty_terminal` | 终端仿真与 VT 序列解析 |
| `portable-pty` | 跨平台 PTY（Unix pty 与 Windows ConPTY） |
| `gix` | 处理所有 Git 查询以及 Worktree 的创建与清理。不依赖外部 `git` 命令，不链接 libgit2 |
| `snow` | TCP 连接层 Noise 握手协议实现 |
| `iroh` | P2P QUIC 传输与打洞支持 |
| `syntect` 与 `two-face` | Preview 和 Diff 视图的语法高亮引擎 |
| `tracing` | 结构化日志收集 |
| `tokio` | Server 与 core 组件的异步并发运行时。窗口前端采用 GPUI 内置执行器 |

---

## 从哪里读起

| 功能领域 | 代码入口 |
| :--- | :--- |
| 终端渲染 | `crates/condr-gui/src/terminal_element.rs`、`crates/condr-server/src/server/terminal_stream.rs`、`crates/condr-server/src/client_writer.rs` |
| 终端输入 | `crates/condr-gui/src/app/terminal_input.rs`、`crates/condr-core/src/terminal/input.rs` |
| 传输协议 | `proto/condr/v1/`、`crates/condr-core/src/protocol.rs`、`crates/condr-server/src/server/client.rs` |
| Agent 识别 | `crates/condr-core/src/agent/`、`crates/condr-core/src/terminal/osc.rs`、`crates/condr-server/src/server/agents.rs` |
| Git 与 Worktree | `crates/condr-core/src/git.rs`、`crates/condr-server/src/server/workspace_git.rs` |
| 设置面板 | `crates/condr-gui/src/app/settings.rs`、`crates/condr-gui/src/app/config.rs` |

模块入口详见 [`docs/code-organization.md`](https://github.com/condrdev/condr/blob/main/docs/code-organization.md)。

---

## 构建与测试

```sh
cargo build                                  # 构建全部
cargo run -p condr-gui                       # 运行窗口，需要显示器
cargo test --workspace --features condr-gui/test-support
cargo clippy --workspace --all-targets --features condr-gui/test-support -- -D warnings
cargo fmt --all
```

* **GUI 构建环境限制**：GPUI 不支持交叉编译，窗口必须在目标系统本地构建。`condr-core` 与 `condr-server` 支持在无显示器环境下编译与测试。
* **开发构建性能优化**：项目根目录 `Cargo.toml` 针对 GPUI、文本排版、终端模拟及 Condr 核心热点路径启用了编译优化，确保 `cargo run -p condr-gui` 在开发构建下保持可用的帧率。

代码贡献规范见 [CONTRIBUTING.md](https://github.com/condrdev/condr/blob/main/CONTRIBUTING.md)。

---

## 设计决定

项目重大架构决策记录于 [ADR](https://github.com/condrdev/condr/tree/main/docs/adr) 目录。修改相关模块前，请先查阅对应的决策文档。
