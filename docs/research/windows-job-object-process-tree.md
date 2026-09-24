# Windows 上用 Job Object 管理 Pane 的进程树

日期：2026-09-24。本文是调研与实现草案，不是 ADR。Condr 对照 HEAD `d53348f`。

## 结论

**建议实现。** Windows 上 Pane 的进程归属现在靠进程表快照沿父 PID 反推，天生会误认和漏认：CI 上已经因此误杀 runner 进程、job 卡死丢日志。`d53348f` 加了「父进程不得晚于子进程启动」的校验，堵住了误认，但漏认和竞态还在。Job Object 是 Windows 为这个场景提供的内核机制（相当于 Unix 的会话），能同时解决归属、原子终止和退出通知，不需要新依赖。Linux 和 macOS 保持现状。

## 这段逻辑的用途

Server 需要知道「哪些进程属于这个 Pane」，用在三处：

- **关闭**：关闭 Pane、删除 worktree、停止 Server 时杀掉整棵树（shell 和它启动的 agent、dev server 等），否则残留进程继续运行并占着 worktree 里的文件；Windows 上文件被占用时目录删不掉。[关闭](../../crates/condr-core/src/terminal/process.rs)（`attempt_process_tree_shutdown`、`refresh_owned_processes`），[重试](../../crates/condr-core/src/terminal/runtime.rs)（`attempt_process_tree_shutdown` 的两处调用）
- **空闲判断**：`agent start` 只往没有子进程的 shell 里输入命令。[空闲判断](../../crates/condr-core/src/terminal/launch.rs)（`#[cfg(windows)]` 分支）
- **识别 agent**：Windows 没有前台进程组，只能在 shell 的后代里找最顶层的 agent。[识别](../../crates/condr-core/src/terminal/process.rs)（Windows 版 `probe_agent`）

## 各平台现状

**Unix**：归属是内核属性。portable-pty 在 exec 前 `setsid`，shell 是会话首进程，Pane 内进程的 `getsid` 都等于 shell PID；空闲判断和识别 agent 用 PTY 的前台进程组。孤儿会被 init 收养，PID 递增分配、很少很快复用。关闭时对会话成员依次发 HUP、TERM、KILL，并用 PID 加启动时间校验身份。

**Windows**：每次拍一张 sysinfo 进程表，沿 `parent()`（即 `InheritedFromUniqueProcessId`）找 shell 的后代，再逐个 `TerminateProcess`。Windows 的 PID 是 4 的倍数，回收后很快复用；子进程记着的父 PID 在父进程死后不会更新。

## Windows 现方案的问题

1. **误认（`d53348f` 已修）**：新 shell 拿到某个已死父进程的 PID 后，那个父进程留下的孤儿（CI runner 上实测有 `explorer.exe`、`sihost.exe`、`ctfmon.exe`、`vctip.exe` 等）会被当成 Pane 的子进程并被 kill。杀不掉时报 `terminal process tree did not exit after forced shutdown`；杀掉的若是 runner 进程，job 就卡死并丢日志。用户机器上同样可能误杀 `explorer.exe`。
2. **漏认**：中间进程先退出（`cmd /c start`、启动后即退出的启动器）后，孙进程的父 PID 指向死进程，链条断开，它不再算 Pane 的进程，关闭 Pane 后继续存活并占着文件。Unix 上它仍在同一会话里。
3. **竞态**：最后一次扫描之后新起的进程会漏杀；快照与 `TerminateProcess` 之间 PID 可能被复用，现用启动时间（秒级精度）缓解。
4. **开销**：每次轮询都要枚举整张进程表；`process.rs` 里已经为此做了不少优化并写了注释。

## 方案：每个 Pane 一个 Job Object

Job Object 里的进程启动的后代会自动进入同一个 Job，不受 PID 复用和中间进程退出影响。

| 需要 | API |
| --- | --- |
| 创建 | `CreateJobObjectW(null, null)`：安全属性为空，句柄不可继承，否则子进程继承句柄会让 Job 永远不关闭 |
| 限制 | `SetInformationJobObject(JobObjectExtendedLimitInformation)`，`LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE \| JOB_OBJECT_LIMIT_BREAKAWAY_OK` |
| 加入 | spawn 后立刻对 shell 句柄调 `AssignProcessToJobObject`；portable-pty 0.9 的 `Child::as_raw_handle()` 能拿到句柄 |
| 关闭 | `TerminateJobObject(job, code)`：一次原子地杀掉整棵树 |
| 成员 | `QueryInformationJobObject(JobObjectBasicProcessIdList)`，变长结构，返回 `ERROR_MORE_DATA` 时放大缓冲区重试 |
| 退出 | `JobObjectBasicAccountingInformation.ActiveProcesses == 0`，或者用 `JobObjectAssociateCompletionPortInformation` 接收 `JOB_OBJECT_MSG_ACTIVE_PROCESS_ZERO` |

语义对照 Unix：

- `KILL_ON_JOB_CLOSE`：Server 退出或崩溃时句柄关闭，整棵树结束，对应 PTY 主端关闭后会话收到 SIGHUP。
- `BREAKAWAY_OK`：程序可以用 `CREATE_BREAKAWAY_FROM_JOB` 主动脱离，对应 `setsid`、`nohup`。

先例：cargo 在 Windows 上用带 `KILL_ON_JOB_CLOSE` 的 Job Object，保证 Ctrl-C 杀掉所有子进程；Chromium 的沙箱也用 Job Object；Rust 的 `command-group`、`process-wrap` 在 Unix 用进程组，在 Windows 用 Job Object。

## 落地要点

- **依赖**：`condr-core` 已经直接依赖 `windows-sys`（0.59），只需加 feature：`Win32_System_JobObjects`，以及签名用到的 `Win32_Security`、`Win32_System_Threading`。不引入新 crate。
- **挂接点**：[runtime.rs](../../crates/condr-core/src/terminal/runtime.rs) 的 `spawn_command` 之后创建 Job 并加入 shell。`ProcessProbe` 持有 Job 句柄（Windows 专属字段），随终端运行时一起释放。
- **替换**：关闭改为 `TerminateJobObject` 加等待活动进程数归零，取代 Windows 上 KILL 加 2 秒宽限再 `child.kill()` 的流程；空闲判断和 `probe_agent` 都改用 Job 成员列表，只对成员读命令行。
- **保留**：`descendant_depth` 的启动时间校验留着。`root_agent` 仍要在成员之间判断祖先关系，成员的父 PID 也可能指向一个复用了已死成员 PID 的新成员。
- **加入 Job 前的窗口**：portable-pty 的 `CreateProcessW`（`src/win/psuedocon.rs`）没有 `CREATE_SUSPENDED`，所以从启动到加入 Job 之间有极短的窗口。pwsh 和 cmd 要几百毫秒才会起子进程，可以接受。要彻底消除，就得让 portable-pty 通过 `PROC_THREAD_ATTRIBUTE_JOB_LIST`（Windows 10 1607 起）在创建时直接放进 Job，可以向上游提 PR 或 vendor。
- **嵌套**：Windows 8 起支持嵌套 Job，Server 本身运行在别的 Job 里（CI runner、IDE、某些终端）也能加入。
- **ConPTY**：conhost/OpenConsole 由 Server 进程通过 `CreatePseudoConsole` 创建，不在 Pane 的 Job 里，不会被 `TerminateJobObject` 波及。

## 行为变化与待定问题

- 从 Pane 直接启动的 GUI 程序（`Start-Process notepad`、`code .` 的子进程等）只要没有主动脱离 Job，关闭 Pane 时会一起结束。这和 Unix 上会话成员被收掉一致；现在的 Windows 实现在这点上前后不一，取决于启动器是否还活着。需要确认这是想要的语义。
- 空闲判断：Unix 只看前台进程组，后台任务不影响空闲；Windows 用 Job 成员后，`Start-Process -NoNewWindow` 起的后台进程会让 shell 一直显示为忙，这和现在「任何后代」的行为相同。是否改成只看 shell 的直接子进程，另议。
- 通过 COM 或 Shell 关联启动、实际由已有进程（explorer、浏览器等）创建的进程不在 Job 里，关闭 Pane 不影响它们，这是期望的行为。

## 验收

- 单元测试：已有 `an_orphan_older_than_a_reused_parent_pid_is_not_a_descendant`，覆盖误认。
- Windows 集成测试（新增）：Pane 里运行 `cmd /c start /b powershell -NoProfile -Command Start-Sleep 600`，中间进程立即退出；关闭 Pane 后这个 powershell 必须已经不存在。现在的实现会漏掉它。
- Windows 集成测试（新增）：Pane 里的子进程用 `CREATE_BREAKAWAY_FROM_JOB` 启动，关闭 Pane 后它仍然存活。
- CI 的 Windows job 连续多次全量 `cargo test --workspace` 通过，不再出现 forced-shutdown 超时，也不再卡住。

## 其他平台

不做改动，理由见「各平台现状」。以后有具体需要时再评估：

- **Linux**：cgroup v2 是同一类机制，`setsid` 也逃不掉。每个 Pane 一个 cgroup，用 `cgroup.procs`、`cgroup.kill`（5.14+）、`cgroup.events` 的 `populated`；GNOME Terminal 的 VTE 就给每个终端建一个 systemd scope。但它依赖 systemd 用户会话或可写的 cgroup 子树，headless 开发机、容器、WSL 上常常没有，必须保留会话方案做兜底。只有在守护进程逃逸导致 worktree 删不掉，或者需要按 Pane 限制资源时才值得做。`pidfd`（5.3+）能消除「发信号时 PID 已被复用」的竞态，收益小。
- **macOS**：没有公开的等价机制，Apple 用来给进程分组的 coalition 是私有 API。进程组、会话加 kqueue 的 `EVFILT_PROC` 就是能用的全部，现在的做法已经是 macOS 上最好的。
