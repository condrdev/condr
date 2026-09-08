# 代码组织与测试归档

## 模块边界

三个 crate 的职责保持不变：Core 提供领域模型和终端能力，Server 拥有运行时，GUI 是协议 Client。按功能拆分 crate 内部模块，不为文件拆分增加 crate 或依赖。

| 入口 | 职责与主要子模块 |
| --- | --- |
| `crates/condr-core/src/session.rs` | Session 模型；`workspace` 管理 Workspace/Tab，`pane` 管理 Pane 操作，`layout` 处理布局几何，`snapshot` 校验和恢复结构 |
| `crates/condr-core/src/protocol.rs` | 协议公共入口和限制；`messages` 定义消息，`framing` 编解码帧，`bootstrap` 组装分块快照 |
| `crates/condr-core/src/agent/mod.rs` | Agent 领域类型；`process` 识别进程，`event` 定义 Hook 事件，`detector` 更新状态，`hook`/`hooks` 发送和安装 Hook |
| `crates/condr-core/src/terminal.rs` | 终端公共入口；`runtime` 集中管理 PTY 启动、失败回滚与关闭，`input_queue`/`pty_io`/`resize` 处理 I/O，`osc`/`notices` 截获上报，`probes` 探测进程与 cwd，`shell` 配置 Shell，`view`/`view_source` 生成和传输视图 |
| `crates/condr-server/src/client.rs` | 公共 `ClientConnection`，供 GUI 和 CLI 使用 |
| `crates/condr-server/src/endpoint.rs` | 地址解析与选择；`local` 管理 socket/named pipe 的监听和所有权，`stream` 统一流操作与取消 |
| `crates/condr-server/src/ssh.rs` | SSH 地址和远端命令；`stream` 管理 SSH 子进程及原生管道 |
| `crates/condr-server/src/noise.rs` | 密钥类型与 TCP 安全传输入口；`identity` 管理身份、设备和 invite，`stream` 实现握手与加密记录 |
| `crates/condr-server/src/server.rs` | Server 生命周期和权威 RuntimeState；`config` 配置，`recovery` 恢复，`client` 分发请求，`layout`/`agents` 执行操作，`subscriptions` 发布可靠事件，`terminal_stream` 集中处理视觉帧准备、分块与基线提交 |
| `crates/condr-server/src/persistence.rs` | Session 快照写入与关闭时刷盘；`config` 提供跨进程 TOML 配置事务 |
| `crates/condr-server/src/cli.rs` | CLI 连接与错误输出；`workspace`、`pane`、`agent` 各自定义参数、执行命令并组织结果 |
| `crates/condr-gui/src/app.rs` | GUI 状态与初始化；`server_connection` 保存连接状态，`server_management` 管理连接，`connection` 处理 I/O，`events` 消费事件，`presentation` 同步布局呈现 |
| `crates/condr-gui/src/app/dock.rs` | Dock 布局投影；`terminal_panel.rs` 负责终端 Pane 的呈现、焦点和光标闪烁 |
| `crates/condr-gui/src/app/terminal_input.rs` | 键盘输入与终端几何；子模块处理鼠标、选择、剪贴板，`app/ime.rs` 处理输入法组合文本 |
| `crates/condr-gui/src/app/settings.rs` | 设置窗口；`appearance`、`server`、`shortcuts` 按页面功能组织 |
| `crates/condr-gui/src/app/sidebar.rs` | Sidebar 树；`drag_drop` 管理拖放，`icon` 定义状态图标，`item` 呈现树节点 |
| `crates/condr-gui/src/terminal_element.rs` | GPUI Element 及完整的 `prepaint`/`paint` 实现；`cache` 缓存 shaping，`colors` 计算颜色，`geometry` 计算绘制区域，`input` 映射输入 |

模块文件开头集中声明 `mod`、导入和重导出，之后是类型和实现。公共路径通过入口文件显式重导出；内部协作只开放到需要访问的共同父模块。共享状态仍由原来的所有者管理，相关实现可以放在子模块中。

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
