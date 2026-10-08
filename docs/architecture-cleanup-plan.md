# 架构与代码整洁度修复计划

状态：第一轮 S1–S6 已在 `5c11414` 提交，Linux 集成验证通过，用户已反馈 Windows 测试完成。第二轮六项收尾改动已实现，验证记录见文末；S7 按计划暂缓。依据源码审查、独立 Claude Code 复核，以及实施时的交叉代码审查。

目标是让变更集中在对应模块：状态有明确的所有者和生命周期，调用方无需理解内部清理顺序，业务决策不依赖显示文案，重复流程只维护一份。设计模式按这个目标选择，不以模式数量、文件长度或分支数量作为验收指标。

## 实施约束

- 保持 Core / Server / GUI 的既有职责；GUI 仍是协议 Client，Server 拥有 Session 和运行时。
- 遵守 `docs/code-organization.md`：按功能组织模块，保留共享状态的现有所有者。先通过普通 struct 和窄接口收拢状态，不为此增加 crate、依赖或全局事件总线。
- 结构重构和并发行为变更分开提交。沿用现有枚举、函数入口、GPUI 通知和生命周期守卫。
- 使用新的回归用例明确行为缺陷；迁移已有测试时保留断言，不用“测试新的内部结构”替代行为验证。
- 协议调整遵循 ADR 0028 的字段号、加法扩展和未知值回退规则；不为此次重构另建兼容框架。
- 不改变可靠 Session 事件与可丢弃终端视觉流的分离，不削弱现有终端性能保障。

## 工作包与顺序

每个工作包独立验收；S4 分成两个提交，其余按实际变更规模保持单一目的。

| 顺序 | 工作包 | 设计收益 | 风险 | 依赖 |
| --- | --- | --- | --- | --- |
| S1 | 统一连接失败与重连状态转换 | 发送函数只负责发送，连接生命周期由一个入口处理 | 低至中 | 无 |
| S2 | 关联异步请求与响应 | 请求完成、取消和失效的规则集中管理 | 低至中 | S1 |
| S3 | 控制权拒绝原因类型化 | 业务策略与显示文案解耦 | 中，涉及协议 | S1 |
| S4a | 收拢 Server 外部布局操作 | 请求分发不再掌握提交与失败补偿细节 | 中 | 无；按本表串行推进 |
| S4b | 外部布局移出连接读循环 | 慢操作不阻塞终端输入和 Ping | 较高，并发语义 | S4a、S2 |
| S5 | 接收处建立已校验的 Session 模型 | 普通读取不再复制、恢复和校验整个结构 | 中 | S1 |
| S6 | 收拢 GUI 功能状态与清理 | 删除平行清理清单，缩小根实体需要知道的细节 | 中 | S2、S5 |
| S7 | 编辑器检测数据化 | 安装信息与检测流程分离 | 低 | 暂缓，扩展编辑器支持时执行 |

## S1：连接生命周期

入口：`app/server_connection.rs`、`app/server_management.rs`、`app/connection.rs`、`app/events.rs`、`app/navigation.rs`。

1. 先用现有 GUI 测试设施复现：Connecting 期间设置页发送 Status，随后成功的连接结果被拒绝。
2. `ServerConnection::send` 返回发送结果；没有 I/O 或队列已关闭时，不自行改写连接状态或丢弃接收任务。
3. 让连接失败通过已有断开处理入口统一执行取消、清理和重连。检查 writer 的所有退出路径，确保发送失败最终可到达这一入口。
4. 检查全部发送调用方。布局投影、终端命令和 pending 请求根据实际发送结果决定是否登记，不能继续通过读取 `status` 猜测发送成功。
5. 复用现有 generation 校验，使旧连接的事件与结果不能影响新连接。

验收：重连期间的 Status 不改变 Connecting；writer 失败后能够清理并重连；失败发送不留下新 pending；手动断开、协议拒绝和旧 generation 的既有行为保持正确。

本阶段只统一转换入口。传输、同步、控制权的完整枚举化留待重复规则仍然明显时再评估。

## S2：请求关联与失效

入口：`app/files.rs`、`app/changes.rs`、`app/events.rs`、`app/server_connection.rs`。

1. Diff、目录、文件的 pending 记录保存当前请求编号；仅在成功发送后登记。
2. 收到响应时先核对编号。旧响应既不能写入缓存，也不能清除新请求的 pending。
3. 将比较基准切换、文件刷新、目录折叠、Workspace 删除、Bootstrap 和连接替换纳入同一失效规则。
4. 同步更新使用固定 `request_id: 0` 的测试辅助代码，让测试模拟真实的请求生命周期。
5. 保留三种资源的明确类型；先复用必要的小函数，不引入通用异步资源框架。编号匹配足够时，不再冗余增加比较基准缓存键。

验收：HEAD 请求在切换基准后返回时被忽略；连续刷新只接受最新请求；折叠目录不会被迟到响应重新缓存；重连后可以重新请求；相同文件内容仍保持 Editor 和滚动位置。

## S3：控制权拒绝原因

入口：`proto/condr/v1/messages.proto`、`condr-core/src/protocol/{messages.rs,pb/messages.rs}`、Server 控制权处理、GUI 控制权重试。

1. 为控制权拒绝补充机器可读的原因，区分 Busy 与其他拒绝；显示文案保留为详情。
2. 基于现有 wire message 做加法扩展，保留字段号；更新所有构造、编解码与测试。
3. GUI 仅根据原因决定是否重试。未知原因采用有文档说明的保守回退，显示详情且不因未知值循环重试。
4. 删除行为代码对 `CONTROL_BUSY_REASON` 文案的比较。文案改变不再影响控制权获取。

验收：Busy 自动重试；其他拒绝不误重试；改变文案不改变行为；未知枚举可解码；协议 round-trip、Buf lint 和 breaking 检查通过。

`connection_advice` 的字符串匹配主要影响提示，暂不扩大成全项目错误类型重构。明确误提示可作为单独的小修复。

## S4：Server 请求分发与操作执行

入口：`condr-server/src/server/{client.rs,layout.rs,agents.rs}` 及现有生命周期、订阅模块。

S4a 保持执行方式，先移动完整业务流程：

- `client` 保留握手、读帧、请求路由和连接生命周期。
- `layout` 对外提供完整的外部布局操作入口，内部负责准备、重新校验、执行、提交、失败补偿、终端恢复和结果产生。
- 将通用地址校验与命令权限规则提取成有语义的函数，保留 Focus、Copy、输入和 viewer 状态各自的现有语义。
- 接口隐藏锁的获取顺序和补偿细节；避免只把大函数机械搬到另一个文件，仍要求调用者逐步编排。

S4b 再隔离慢操作：

- 复用现有线程和 `OperationGuard`，不新增任务框架。每连接限制同时运行的外部布局操作数量。
- 终端输入与 Ping 可继续处理；冲突的后续结构操作明确返回“操作进行中”，不隐式乱序提交，不建立无界等待队列。此行为需在同一提交中更新相关文档和测试。
- 后台操作持有生命周期守卫，并负责终端 monitor 的启动与资源收尾。关闭连接和停止 Server 时仍能正确完成或补偿。
- 保持 Session 提交、可靠事件发布及 `LayoutApplied` 的顺序；任何锁外 I/O 返回后都重新校验目标。
- 列出其他同步慢路径（Hooks、发现、文件读取、Diff、图片暂存），分别确认资源顺序和取消语义，再按同一原则处理；它们不混入第一份并发变更。

验收：利用现有慢 smudge/checkout 测试设施，证明同连接的 Terminal 输入和 Ping 在操作期间得到处理；停机等待与回滚、删除失败后的终端恢复、断开后的资源释放、并发请求限制、事件先于成功响应均有覆盖。

## S5：Session 读取模型

入口：`app/server_connection.rs`、`app/events.rs`、侧栏、导航、Dock 及其他 Session 查询调用方。

1. 将 Bootstrap 与 LayoutChanged 作为模型更新入口，在替换当前状态前完成 `Session::restore` 校验。
2. 连接保存一个已校验的 Session；查询返回借用。避免同时维护可分别修改的 Snapshot 与 Session 两份真相。
3. Dock 需要局部布局推演时显式 clone，线上快照在需要编码或持久化时派生。
4. 无效快照不得部分应用或静默变成空页面：保留上一份有效模型，进入已有的协议错误与断开处理，避免无上限重新 Bootstrap。
5. 删除渲染和普通 getter 中重复的 restore 调用。

验收：多次查询不会重新恢复模型；合法结构事件更新视图且不重置终端；无效结构不会部分替换；跨 Device 的视图和本地 Dock 投影保持正确。比较代表性多 Workspace 查询的开销，记录结果，不预设帧率收益。

## S6：GUI 状态所有权和清理

入口：`app.rs`、`app/navigation.rs`、`app/presentation.rs`、`app/files.rs`、`app/changes.rs`、`app/terminal_input*`、`app/updates.rs`。

1. 将终端交互、Files/Changes 请求资源、更新检查等内聚字段收进各自普通状态 struct，仍由现有根实体或连接持有。
2. 每个状态模块提供少量语义入口，内部负责清理；字段分组同时移动行为，避免形成只能由外部逐字段操作的数据袋。
3. 合并 Pane/Workspace 删除与整条连接清空的平行清理规则。收集存活 ID 后，由各模块按同一保留规则处理。
4. 核对 `changes_open`、IME、转发按键等字段的完整生命周期；先检查焦点转换已有的清理，再修真正遗漏，不能机械合并清单。
5. 更新 `docs/code-organization.md` 的模块说明及持久化相关过期注释。跨模块有实际独立订阅需求时才引入新的 GPUI Entity 或语义事件。

验收：关闭 Pane/Workspace、同 Server 重连、Server 替换分别清理正确的状态；不同 Device 使用相同 ID 时互不干扰；关闭目标不会遗留选择、IME 或请求；新增模块内部状态无需在根实体的多处清理函数同步登记。

## S7：低优先级扩展整理

下次增加编辑器预设时，把 Zed、VS Code、Cursor 的名称、平台路径、别名和参数整理为描述表；JetBrains 的特殊检测继续使用专用函数。沿用现有函数指针策略表和可注入的安装根目录测试。

暂不安排：抽象工厂体系、全局单例、通用责任链、继承式模板流程、访问者框架、全局事件总线，以及没有测量依据的终端缓存改造。现有 AgentSpec、传输适配、PaneLayout 和终端绘制缓存作为已有设计继续使用。

## 验证与完成标准

- 每包先运行能够证明其行为的现有测试与必要回归测试，再做对应 crate 的检查；不为字段搬家新增镜像实现的测试。
- Linux 无头环境可验证 Core、Server 及 GPUI TestAppContext；Windows 原生 GUI、ConPTY、焦点、IME、拖选和实际帧率单独验收。
- 涉及终端响应性的变更，保留 burst 合并、最终帧、shaping 复用和真实 GPUI 拖选覆盖；Windows 使用持续输出和 Agent 动画场景检查。
- 阶段集成遵循 CI：

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --features condr-gui/test-support --locked -- -D warnings
cargo test --workspace --features condr-gui/test-support --locked --no-fail-fast
```

协议包另通过 `.github/workflows/ci.yml` 的 Buf 检查。跨平台检查失败与既有环境问题分别记录，不把未执行的验收标记为通过。

完成以维护面为准：连接失败有一个处理入口；资源响应有一个接受规则；布局提交与补偿由 layout 隐藏；显示文案不参与控制策略；Session 查询不执行恢复；GUI 状态清理由所属模块维护。长枚举、必要的分支及有明确含义的常量可以保留。

## 实施记录

提交安排调整：S1–S6 按最终通过验证的整体状态合并提交，未按原计划拆分各工作包及 S4a/S4b；下述验收结果覆盖这一集成状态。

- S1：发送结果与连接生命周期分离；断开会退休旧 generation，拒绝同批迟到 Bootstrap/控制权消息。订阅、快照、布局、目录浏览、图片粘贴和唤醒 Ping 只在成功入队后登记。
- S2：`WorkspaceResources` 在同一个连接中持有三种资源的缓存及当前请求；按 Workspace、路径和 request_id 接受响应，失效同时移除 pending。
- S3：新增 `ControlDenialReason::{Other, Busy}` 和 protobuf 加法字段；提示与定时重试均按类型判断，包括 Busy 后收到 Other 的情况。
- S4：`layout::operation::ClientLayouts` 拥有完整事务与每连接一个在途操作；慢准备阶段不阻塞输入/Ping，冲突 Layout 明确拒绝。连接级 Bootstrap 锁防止旧快照覆盖后台提交；失败创建的 Git 补偿在 Session 锁外完成。
- S5：连接持有唯一的已校验 Session；读取借用，Dock 推演显式复制。非法 Bootstrap/Layout 保留旧模型与游标，进入协议断开路径。
- S6：`TerminalInputState`、`FilesViewState`、`WorkspaceResources`、`UpdateCheck` 管理自己的状态变更和生命周期。Pane/连接清理共享保留规则，Workspace 关闭清理侧栏选择与 Editor；删除每次 Dock 重建时重复扫描 Editor 的清理流程。

本轮保留的执行边界：Shell 启动仍按既有同步提交方式执行。Hooks 配置写入、Agent 安装/进程发现、Diff/目录/文件读取、图片暂存没有混入此次并发改动；这些路径需要分别明确序列化及取消语义。`BrowseDirectory` 原有独立线程仍需另行限制并发。没有增加依赖、crate、全局总线或设计模式框架。

Session 查询微基准已运行（Linux arm64，当前测试构建，64 个 Workspace，2,000 次读取）：旧 Snapshot 克隆及 restore 路径约 207.9 ms，借用已校验模型约 5.8 μs。用例 `app::tests::session_model::session_model_read_cost` 默认忽略，需手动运行；它测量模型查询，不能据此声称 GUI 帧率提升。

最终验证结果（Linux arm64）：

- `cargo fmt --all -- --check` 通过。
- `cargo clippy --workspace --all-targets --features condr-gui/test-support --locked -- -D warnings` 通过。
- `cargo test --workspace --features condr-gui/test-support --locked --no-fail-fast`：586 项通过、0 项失败、4 项默认忽略。其中 GUI 206 项和 Server 单元测试 125 项通过；Session 查询微基准已另行执行通过。
- `buf lint proto` 与相对实施前 protobuf 基线的 `buf breaking` 通过。
- 覆盖慢 checkout 下输入/Ping、Snapshot/提交排序、停机与断连、失败 worktree 清理、迟到资源响应、非法模型停止重连、IME 迟到提交、GPUI 拖选和 shaping 缓存复用。
- 修正两项旧 GUI 测试的等待条件：标签栏等待 `LayoutApplied` 完成选择；鼠标测试等待编码模式启用后的真实就绪输出，保留原行为断言。

第一轮提交后，用户已反馈 Windows 测试完成；未在此逐项记录实机测试结果。

## 第二轮：资源事务与单一所有者

本轮处理后续架构审查确认的六项问题，沿用普通结构体、枚举和现有生命周期守卫，没有增加依赖或通用模式框架。

| 工作项 | 最终实现 | 行为验证 |
| --- | --- | --- |
| Worktree 创建事务 | `WorktreeCreation` 从独占创建目录起持有所有权，注册、checkout、验证任一失败均清理本次目录；清理失败保留原始错误并报告详情 | 注册中途失败与缺失 tree 对象导致的 checkout 失败均不留半成品，不删除已有 Worktree，允许重试 |
| Hooks 操作事务 | Server 先验证目标并登记生命周期操作，再在 Session 锁外执行；Core 按真实路径排序加跨进程锁，用已有 `atomicwrites` 原子替换 | 无效或停机后的请求无配置副作用；停机等待已接受操作并返回真实结果；并发更新、符号链接、用户字段及缺失目录行为均有覆盖 |
| Settings 提交结果 | `CommitOutcome` 区分无效、不可用、暂存和已排队，失败保留草稿；切换 Device 清理 Saved 提示和计时任务 | 发送失败、未连接、监听地址暂存以及跨 Device 提示均有 GPUI 回归 |
| Git 扫描所有权 | 文件通知与终端活动统一交给一个 worker；终端唤醒按 Workspace 合并，独立保留扫描间隔与文件去抖；停机在 Session 锁外发停止信号并 join | 连续活动合并、多 Workspace 调度、指纹未变化时文件事件仍触发扫描、无 notify 回退、空闲退出与析构不自等待 |
| PTY 分阶段构造 | 创建工作线程前由 `TerminalRuntime` 接管资源，各阶段失败复用正常关闭流程，删除四份独立补偿代码 | 注入 writer、resizer、reader、child-watch 启动失败，验证子进程回收及 I/O 资源释放 |
| GUI 配置快照 | 一次读取 `[client]`，纯解码生成 `LoadedConfig`，保留逐字段容错和设备配置写保护 | 配置 11 项定向测试通过，覆盖默认值、字段容错、编辑器错误和设备列表保护 |

边界：Hooks 仍在所属连接的请求循环中同步执行，其他连接不持有 Session 锁等待；没有扩展成通用后台任务系统。Git 停止会等待当前 gix 扫描完成，不能中断扫描内部 I/O。Worktree 回滚只清理本次独占创建的目录，保留共享父目录和已创建分支。

第二轮验证结果（Linux arm64）：

- 全工作区测试：605 项通过、0 项失败、6 项默认忽略；不重复统计子进程输出。其中 GUI 211 项、Server 单元测试 133 项通过；两个新增的忽略项是由父测试实际执行的子进程辅助用例。
- `cargo fmt --all -- --check`、`git diff --check` 和全工作区严格 Clippy 通过。
- Worktree/PTY 与 Hooks/Git worker 完成交叉代码审查，没有遗留的阻断发现。本轮没有协议或依赖变更。

第一轮的 Windows 反馈不覆盖本轮：需要重新验证 Settings 切换 Device 与失败保存、Git 刷新及退出，以及 ConPTY 启动失败和持续输出下关闭。尤其旧版 Windows 的 ConPTY 关闭仍沿用原有顺序，尚未原生验证启动早期失败且存在输出积压的情况；[Microsoft 的关闭说明](https://learn.microsoft.com/en-us/windows/console/closepseudoconsole) 要求关闭输出管道或在关闭期间继续读取。

S7 编辑器描述表和终端协议 reader 状态机继续暂缓。
