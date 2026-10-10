# 代码组织与测试归档

## 模块边界

五个 crate 各有职责：Core 提供领域模型和终端能力，`runtime` feature 之外只剩协议与领域类型；Client 是所有 Client 共用的连接与读取模型；Server 拥有运行时；GUI 和 Mobile 分别是桌面与手机的 Client（ADR 0039）。按功能拆分 crate 内部模块，不为文件拆分增加 crate 或依赖。

| 入口 | 职责与主要子模块 |
| --- | --- |
| `crates/condr-core/src/session.rs` | Session 模型；`workspace` 管理 Workspace/Tab，`pane` 管理 Pane 操作，`layout` 处理布局几何，`snapshot` 校验和恢复结构 |
| `crates/condr-core/src/protocol.rs` | 协议公共入口和限制；`messages` 定义消息，`framing` 编解码帧，`bootstrap` 组装分块快照 |
| `crates/condr-core/src/agent/mod.rs` | Agent 领域类型和 `AgentKind::spec()`；每种 Agent 一个模块（如 `claude.rs`）放它的 `AgentSpec`（ADR 0035），`process` 识别进程，`event` 定义 Hook 事件，`detector` 更新状态，`hook` 发送 Hook，`hooks` 是各 Agent 共用的四类 Hook 安装格式 |
| `crates/condr-core/src/terminal.rs` | 终端公共入口；`runtime` 集中管理 PTY 启动、失败回滚与关闭，`input_queue`/`pty_io`/`resize` 处理 I/O，`osc`/`notices` 截获上报，`probes` 探测进程与 cwd，`shell` 配置 Shell，`view`/`view_source` 生成和传输视图 |
| `crates/condr-client/src/connection.rs` | `ClientConnection<S>`、`Cancellation<S>` 与 `Refused`；`ServerStream`、`Connector` 两个 trait 把它们与具体传输分开，Server 的 `client.rs` 用别名接上桌面的 `Endpoint` |
| `crates/condr-client/src/noise.rs` | Noise 握手与加密记录的两端；接受连接的一侧经 `Authority` 读取本机密钥和已配对 Device，`key` 定义密钥类型，`tcp` 定义 `TcpEndpoint` 与拨号 |
| `crates/condr-client/src/p2p.rs` | `P2pNode`、`P2pStream` 与 `P2pEndpoint`：绑定、拨号、接受和流本身；Server 的 `p2p.rs` 只剩 `Tunnel` 的 `splice` |
| `crates/condr-client/src/io.rs` | 一条连接的读写线程与心跳，`frames` 处理分块、栅栏、视觉槽与帧合并，`model` 是 `SessionModel`：校验后的 Session、终端、Agent 跟踪与「Needs you」 |
| `crates/condr-mobile/src/companion.rs` | Companion 的 uniffi 接口：每个 Device 的连接与模型、合并通知（`changed` 加 `take_changes`）、一次性结果队列与终端命令；`views` 定义给 Swift/Kotlin 的记录 |
| `crates/condr-server/src/logging.rs` | `tracing` subscriber、按天滚动的非阻塞文件层、`CONDR_LOG` 过滤和 panic hook；`condr server run` 与 GUI 共用（ADR 0019） |
| `crates/condr-server/src/endpoint.rs` | 地址解析与选择；`local` 管理 socket/named pipe 的监听和所有权，`stream` 统一流操作与取消 |
| `crates/condr-server/src/ssh.rs` | SSH 地址和远端命令；`stream` 管理 SSH 子进程及原生管道 |
| `crates/condr-server/src/noise.rs` | 本机身份目录：`device-key` 的读写，`identity` 管理已配对设备与 invite，并作为 `Authority` 回答 Noise 与 Peer-to-peer 的接受方 |
| `crates/condr-server/src/server.rs` | Server 生命周期和权威 RuntimeState；`config` 配置，`recovery` 恢复，`client` 分发请求，`layout`/`agents` 执行操作，`subscriptions` 发布可靠事件，`terminal_stream` 集中处理视觉帧准备、分块与基线提交 |
| `crates/condr-server/src/persistence.rs` | Session 快照写入与关闭时刷盘；`config` 提供跨进程 TOML 配置事务 |
| `crates/condr-server/src/cli.rs` | CLI 连接与错误输出；`workspace`、`pane`、`agent` 各自定义参数、执行命令并组织结果 |
| `crates/condr-gui/src/app.rs` | GUI 根实体与初始化；持有连接、Dock 和功能状态，协调窗口与呈现；`server_management` 管理连接生命周期，`connection` 把 `condr-client` 的读写线程接到 GPUI 并处理剪贴板图片，`events` 消费事件，`presentation` 同步布局呈现 |
| `crates/condr-gui/src/app/server_connection.rs` | 一条连接的 GUI 状态：视图、订阅、重连与 Settings 所需的 Server 状态，读取模型放在内嵌的 `SessionModel` 里；Bootstrap 和 LayoutChanged 由它校验后替换，无效时保留旧状态并交由连接层断开 |
| `crates/condr-gui/src/app/workspace_resources.rs` | 连接持有的 `WorkspaceResources`；集中管理 Diff、目录、文件缓存及当前请求编号，处理响应匹配、刷新、折叠、Workspace 移除与连接重置 |
| `crates/condr-gui/src/app/dock.rs` | Dock 布局投影；`terminal_panel.rs` 负责终端 Pane 的呈现、焦点和光标闪烁 |
| `crates/condr-gui/src/app/terminal_input.rs` | 键盘输入与终端几何；`TerminalInputState` 管理目标、选择、链接、鼠标捕获、焦点、已转发按键和 IME 状态，统一按存活 Pane 清理；子模块处理鼠标、选择、剪贴板，`app/ime.rs` 处理输入法组合文本 |
| `crates/condr-gui/src/app/files.rs` | Files 侧栏与 Preview Tab；`FilesViewState` 管理 Files/Changes 的本地展示选择、目录展开、Editor 和跳转位置，并按 Session 统一保留或清理；`changes.rs` 负责 Changes 侧栏与 Diff Tab |
| `crates/condr-gui/src/app/updates.rs` | `UpdateCheck` 管理更新渠道、结果、已读状态及自动检查任务；设置窗口和侧栏读取同一个状态 |
| `crates/condr-gui/src/app/settings.rs` | 设置窗口；`appearance`、`server`、`shortcuts` 按页面功能组织 |
| `crates/condr-gui/src/app/sidebar.rs` | Sidebar 树；`drag_drop` 管理拖放，`icon` 定义状态图标，`item` 呈现树节点 |
| `crates/condr-gui/src/terminal_element.rs` | GPUI Element 及完整的 `prepaint`/`paint` 实现；`cache` 缓存 shaping，`colors` 计算颜色，`geometry` 计算绘制区域，`input` 映射输入 |

模块文件开头集中声明 `mod`、导入和重导出，之后是类型和实现。公共路径通过入口文件显式重导出；内部协作只开放到需要访问的共同父模块。共享状态仍由原来的所有者管理，相关实现可以放在子模块中。

功能状态使用普通 struct，由连接或 GUI 根实体持有；生命周期规则放在状态所属模块，根实体调用其清理入口。Pane/Workspace 移除与整条连接替换复用同一保留规则，并始终用连接与领域 ID 共同标识目标。Session 的 Server 权威不变：GUI 只在接收结构时恢复和校验，普通查询借用模型，Dock 需要本地推演时显式复制。

涉及资源的多阶段操作从首次取得资源起就拥有失败清理：Core 的 Worktree 创建守卫只回收本次创建的目录；PTY 在启动线程前建立完整所有者，各启动阶段失败复用正常关闭流程。Server 的 Hooks 入口先校验并登记在途操作，再执行锁外 I/O；Core 的 Hooks 模块按实际文件路径串行化读改写，并原子发布结果。

Workspace Git 的文件通知和终端活动都交给同一个后台工作线程处理，扫描和发布由它串行执行；终端 probe 只提交刷新意图。Watcher 的所有者负责显式停止，Server 在 Session 锁外等待退出。GUI Settings 用明确的提交结果区分验证失败、连接不可用、本地暂存与成功入队；Device 切换同时清理提交提示。GUI 启动配置从 Core 读取一次 `[client]` 快照，再独立解码各字段，保留字段级容错。

`gui_state.rs` 保存 Client 的 View、窗口、侧栏宽度，以及每个 Workspace 的侧栏展开、Changes 开关、Files/Changes 选择和比较基准；这些是 GUI 状态文件中的展示记忆，不属于 Server 的 Session Snapshot。目录展开、Editor、请求和终端输入状态只在本次运行中保留。

按职责拆分，不按固定行数切块。消息枚举、请求分发或一个完整的验收场景可以较长；不要为了缩短文件把同一流程拆成难以追踪的小片段。

## 测试放置

- **只依赖公开 API 的测试**放在所属 crate 的 `tests/`，与 `src/` 同级。按功能命名，如 `protocol.rs`、`terminal_view.rs`、`agent_hooks.rs`、`config.rs`；CLI 验证通过 `CARGO_BIN_EXE_condr` 启动实际二进制。
- **需要私有实现的测试**保留为 `#[cfg(test)]` 单元测试。小组放在实现末尾；较大的测试组用 `mod tests;` 放在模块自己的 `tests.rs` 或 `tests/` 下，仍属于源模块，不是 Cargo 集成测试。
- 私有测试也按功能归档，例如 `terminal/tests/{input,osc,view}.rs`、`server/tests/{control,subscriptions,shutdown}.rs`。共享测试数据和辅助函数放在该测试模块的入口；只有一组测试使用的辅助函数留在该组。
- 不为迁移测试扩大生产 API，不通过 `#[path]` 或 `include!` 将私有生产文件重新编译进集成测试，也不复制生产实现。直接调用 `#[cfg(test)]` 辅助方法的测试仍是单元测试。
- GUI 目前是二进制 crate，状态与交互 API 都是私有实现，因此测试保留在模块内。需要 GPUI 测试环境的用例继续由 `test-support` feature 启用。
- 重构时保留测试名称、断言和平台/feature 条件；移动进程自启动测试后，同步修改 `--exact` 使用的完整测试名。

## 验证

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --features condr-gui/test-support -- -D warnings
cargo test --workspace --features condr-gui/test-support
```

Linux 无头环境可以运行 Core、Server 和 GPUI TestAppContext 测试。Windows 分支需另行编译检查；原生 GUI、ConPTY 和真实交互验收仍在 Windows 上执行。
