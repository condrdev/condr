# 锁毒化与 panic 隔离：常驻的 Rust Server 怎么做

调查日期：2026-10-29，基于 Condr 提交 `82e3e1f`，工具链 Rust 1.99.1。condr-core 和 condr-server 与 `c45efcd`（FFI 回调调研的基线）相同。std 的说法取自工具链自带的 1.99.1 文档和 `rust-lang/rust` 的 `1.99.1` 标签。其他项目是同日浅克隆的默认分支，提交号见第二节的表和各处链接。只读代码，没有运行这些项目，也没有改 Condr。文中的计数是正则粗算：排除了 `tests/` 目录和 `#[cfg(test)]` 块，没有区分 async 锁和自定义 helper。

结论：Rust 社区没有一条公认的最佳实践。libs 团队自己还在争：「Edition 2027 是否把 `std::sync::Mutex` 换成不毒化版本」的讨论至今没有结论。不过成熟项目的做法可以归成一条规则：

- 一个工作单元不和别人共享可变状态时，在它的边界上接住 panic，只丢掉这个单元。
- 共享状态可能在 panic 中途被改坏时，让整个进程退出，交给外面重启。

用 std 还是 parking_lot，和这条规则基本无关。Zed、wezterm、alacritty、rust-analyzer 和 rustc 都用 parking_lot。它们没有写明理由；2020 年问卷里，用第三方锁的首要理由是性能和不用写 `.unwrap()`。panic 策略则各自另定：

- Zed 的发布版装了 panic hook，任何 panic 都 abort；
- rust-analyzer 只接住只读请求的 panic；
- wezterm 什么都不做。

用 std 的项目大多直接 `.lock().unwrap()`，如 cargo、zellij、omicron；herdr 和 tokio 内部则用 `.ok()` 或 `into_inner` 无视毒化。整个进程 fail-fast 的有：

- Zed：hook 里 abort；
- zellij：hook 让整个会话结束；
- deno、omicron、uv：设 `panic = "abort"`。

按单元隔离的有：

- tokio：任务；
- tower-http：请求；
- rust-analyzer、ruff、ty：只读请求；
- Paneflow：会话。

样本里在 panic 之后照常使用共享状态、既不隔离也不做标记的，是 wezterm 和 alacritty 这类用 parking_lot 的终端（推断）。std 1.99.1 的 `std::sync::nonpoison` 仍然只能在 nightly 上用，Condr 用不了。

放到 Condr 里，Pane 的终端是单元自己的状态，Server 的 `RuntimeState` 是所有单元共享的状态，正好对应规则的两半，所以推荐原方案，即第四节的方案 (a)：

- 继续用 std `Mutex`；
- reader 线程接住自己的 panic，像 PTY EOF 一样结束这个 Pane；
- 锁被毒化的 Pane 当作终端已退出；
- Server 级的锁被毒化时，记日志后 `abort`，由 GUI 重连时拉起新的 Server，按 ADR 0005 恢复。

std 的毒化位在这里正好有用：它不花一行代码就告诉别的线程「这个 Pane 的终端」或「这个 Session」可能已被改坏。换成 parking_lot，就得自己把这一位重写一遍。全局 `panic = "abort"` 在正确性上最稳，但影响范围最大：一个 Pane 的 bug 就会打断所有 Agent 正在进行的回合。它还会让 libghostty-vt 回调里的 `catch_unwind` 失效，而且 Cargo 不允许只给 Server 设。证据薄弱处：没有找到哪个成熟项目恰好是「Pane 级接住、Pane 锁被毒化就当退出、共享锁被毒化就 abort」这个组合。它是从 herdr、rust-analyzer、Zed 和 zellij 的做法拼出来的。

## 一、std 今天保证什么（Rust 1.99.1）

### 毒化的语义

`Mutex` 文档（[poison/mutex.rs:20-54](https://github.com/rust-lang/rust/blob/1.99.1/library/std/src/sync/poison/mutex.rs#L20-L54)）有四点：

- **默认用法。** 「Most usage of a mutex will simply `unwrap()` these results, propagating panics among threads to ensure that a possibly invalid invariant is not witnessed.」
- **只是建议性的。** 「Poisoning is only advisory」：`PoisonError::into_inner` 照样交出 guard。
- **检测不完整。** 「the panic detection is not ideal」，列出的例外有：在 `Drop` 或 panic hook 里第二次 panic、跨 panic 上下文存取 guard、外来异常（「Foreign exceptions do not currently trigger poisoning」）。
- **unsafe 代码不能依赖它。** 「`unsafe` code cannot rely on poisoning for soundness」。这一句是 [rust#143471](https://github.com/rust-lang/rust/issues/143471) 讨论后补进去的。

与毒化相关的 API：

- `PoisonError::{into_inner, get_ref, get_mut}` 自 1.2.0 起稳定。
- `Mutex::is_poisoned` 自 1.2.0 起稳定，文档提醒「You should not trust a `false` value for program correctness」。
- `Mutex::clear_poison` 自 1.77.0 起稳定（[mutex.rs:608-610](https://github.com/rust-lang/rust/blob/1.99.1/library/std/src/sync/poison/mutex.rs#L608-L610)），用途是检查或修好数据后摘掉标记。
- `RwLock` 只在写锁持有期间 panic 才会被毒化，读锁持有期间 panic 不会。

`UnwindSafe` 的文档写明，`&Mutex<T>` 之所以算 unwind safe，正是因为它默认毒化：「they already provide their own "speed bumps"」。`catch_unwind` 的文档：「It is **not** recommended to use this function for a general try/catch mechanism」（[panic.rs:303](https://github.com/rust-lang/rust/blob/1.99.1/library/std/src/panic.rs#L303)），而且自定义 panic hook 在接住之前就执行。

Rust 1.62 起，Linux 上的 std 锁改用 futex 实现（[1.62 发布说明](https://blog.rust-lang.org/2022/06/30/Rust-1.62.0/)）。当年换 parking_lot 的性能理由，在 Linux 上已不成立。

### `nonpoison` 与改默认值的计划

- **1.99.1 里仍未稳定。** `pub mod nonpoison` 挂在 `#[unstable(feature = "sync_nonpoison", issue = "134645")]` 下，`pub mod poison` 挂在 `sync_poison_mod`（#134646）下（[sync/mod.rs:227-230](https://github.com/rust-lang/rust/blob/1.99.1/library/std/src/sync/mod.rs#L227-L230)）。本地 1.99.1 文档里这两个模块都标着「nightly-only experimental API」。`nonpoison::Mutex` 还要另开 `nonpoison_mutex` 特性，`Condvar` 和 `RwLock` 也各有各的特性（[rust#134645](https://github.com/rust-lang/rust/issues/134645)）。
- **libs-api 的决定**（2024-07-23，[libs-team#169](https://github.com/rust-lang/libs-team/issues/169#issuecomment-2245693473)）：加 `std::sync::nonpoison::*`；把现有类型移进 `std::sync::poison` 并在 `std::sync` 重新导出；「in a future edition, we can change which type is exported as `std::sync::Mutex`」。
- **改默认值仍是一个开着的讨论**：[rust#149359](https://github.com/rust-lang/rust/issues/149359)「Edition 2027: Consider replacing `std::sync::Mutex`…」，标签 `maybe-future-edition`。截至 2026-10-25 的最后一条评论，没有看到 libs-api 的决定。各方立场见第三节。std 内部改用 `nonpoison::Mutex` 的 [PR #148689](https://github.com/rust-lang/rust/pull/148689) 也还开着。

## 二、成熟项目怎么做

| 项目（提交） | 形态 | 锁 | `lock()` 的结果怎么处理 | release `panic` | 一个单元 panic 之后 |
| --- | --- | --- | --- | --- | --- |
| Zed `7f9a39a9` | 编辑器，另有常驻的 remote server | parking_lot 为主（非测试代码约 125 处 `parking_lot::`，std 约 24 处）；终端是 alacritty 的 `FairMutex`（[alacritty.rs:52](https://github.com/zed-industries/zed/blob/7f9a39a9c08373dcc62599906f8dc335d358d7c4/crates/terminal/src/alacritty.rs#L52)） | parking_lot 不返回 `Result` | 未设，即 unwind（[Cargo.toml:1149-1152](https://github.com/zed-industries/zed/blob/7f9a39a9c08373dcc62599906f8dc335d358d7c4/Cargo.toml#L1149-L1152)） | 整个进程 abort，见下文 |
| zellij `1f3df933` | 每个会话一个 server 进程（[server_launch.rs:70-74](https://github.com/zellij-org/zellij/blob/1f3df933978e00c246553070310175c392fcfe4f/zellij-client/src/server_launch.rs#L70-L74)） | std | 约 430 处 `.unwrap()` | 未设 | 整个会话结束，见下文 |
| wezterm `e68a8ec2` | GUI，另有 mux server | parking_lot：`Mux` 的各张表是 `RwLock`（[lib.rs:102-113](https://github.com/wezterm/wezterm/blob/e68a8ec27727fbbda15e07f0a0737cd5e16481f6/mux/src/lib.rs#L102-L113)），Pane 的 `Terminal` 是 `Mutex`（[localpane.rs:14, 126](https://github.com/wezterm/wezterm/blob/e68a8ec27727fbbda15e07f0a0737cd5e16481f6/mux/src/localpane.rs#L14-L126)） | 不返回 `Result` | 未设 | 没人处理，见下文 |
| alacritty `bbb2cbf9` | 终端，一个进程多个窗口 | parking_lot 上的 `FairMutex<Term>`（[sync.rs:1-16](https://github.com/alacritty/alacritty/blob/bbb2cbf9470a6707b1205fbddbdb25109b14e2fd/alacritty_terminal/src/sync.rs#L1-L16)） | 不返回 `Result` | 未设 | 没有 `catch_unwind` |
| herdr `bc8c2898` | Server + Client，与 Condr 最像 | std | 生产代码禁止 `unwrap()`；约 35 处 `.ok()`，另有 `if let Ok` 和 `let Ok(..) else`；约 50 处 `into_inner` | 未设 | 只有那个 Pane 停下，见下文 |
| rust-analyzer `889bb47e` | LSP server | 全局状态用 parking_lot（[global_state.rs:22-25](https://github.com/rust-lang/rust-analyzer/blob/889bb47e853003f62b0a593b955ab9afc97e7885/crates/rust-analyzer/src/global_state.rs#L22-L25)） | 少数 std 锁用 `into_inner`（[proc-macro-srv lib.rs:212](https://github.com/rust-lang/rust-analyzer/blob/889bb47e853003f62b0a593b955ab9afc97e7885/crates/proc-macro-srv/src/lib.rs#L212)） | 未设 | 只读请求在快照上运行并被 `catch_unwind` 包住；改状态的请求不包。见下文 |
| ruff / ty `7cb935d9` | CLI，另有 LSP server | 以 std 为主 | `.unwrap()` | 未设 | 同 rust-analyzer：请求在快照上运行，并被 `catch_unwind` 包住（[ruff_server api.rs:193-194](https://github.com/astral-sh/ruff/blob/7cb935d94e5ccdaefd4efee86ba454e8673a3fa6/crates/ruff_server/src/server/api.rs#L193-L194)、[ty_server api.rs:291](https://github.com/astral-sh/ruff/blob/7cb935d94e5ccdaefd4efee86ba454e8673a3fa6/crates/ty_server/src/server/api.rs#L291)）。`ruff check` 给每个文件包一层，panic 变成这个文件的一条诊断（[check.rs:193-215](https://github.com/astral-sh/ruff/blob/7cb935d94e5ccdaefd4efee86ba454e8673a3fa6/crates/ruff/src/commands/check.rs#L193-L215)） |
| tokio `63fd6949` | async 运行时 | 内部 `Mutex` 包住 std 并「removes the poisoning aspects」（[loom/std/mutex.rs:3-26](https://github.com/tokio-rs/tokio/blob/63fd6949665ffb53ea1091f45f1745242d377e84/tokio/src/loom/std/mutex.rs#L3-L26)），可选 parking_lot；`tokio::sync::Mutex` 不毒化，文档提醒「this might leave the data … in an inconsistent state」（[sync/mutex.rs:122-125](https://github.com/tokio-rs/tokio/blob/63fd6949665ffb53ea1091f45f1745242d377e84/tokio/src/sync/mutex.rs#L122-L125)） | `into_inner` | 由用户决定 | 任务边界有 `catch_unwind`，panic 变成 `JoinError`（[harness.rs:521-545](https://github.com/tokio-rs/tokio/blob/63fd6949665ffb53ea1091f45f1745242d377e84/tokio/src/runtime/task/harness.rs#L521-L545)）。默认 `UnhandledPanic::Ignore`，其他任务照常运行；`ShutdownRuntime` 仍要开 unstable（[builder.rs:179-244](https://github.com/tokio-rs/tokio/blob/63fd6949665ffb53ea1091f45f1745242d377e84/tokio/src/runtime/builder.rs#L179-L244)） |
| tower-http `8569ec8b` | HTTP 中间件 | — | — | 由用户决定 | `CatchPanic` 把一个请求的 panic 变成 500（[catch_panic.rs:202-215](https://github.com/tower-rs/tower-http/blob/8569ec8b99f661eb0a4406a2e032504188d9f3fd/tower-http/src/catch_panic.rs#L202-L215)）。模块文档开头说「using panics for error handling is _not_ recommended」（[1-4](https://github.com/tower-rs/tower-http/blob/8569ec8b99f661eb0a4406a2e032504188d9f3fd/tower-http/src/catch_panic.rs#L1-L4)） |
| deno `e7b39ebd` | JS 运行时 CLI | parking_lot 与 std 混用 | `.unwrap()` | `abort`（[Cargo.toml:477-484](https://github.com/denoland/deno/blob/e7b39ebd55c85aaf6a29290c1f5a99e2e21e09dd/Cargo.toml#L477-L484)） | hook 的注释：「Tokio does not exit the process when a task panics, so we define a custom panic hook to implement this behaviour」（[cli/lib.rs:580-587](https://github.com/denoland/deno/blob/e7b39ebd55c85aaf6a29290c1f5a99e2e21e09dd/cli/lib.rs#L580-L587)） |
| omicron `271efb28` | Oxide 控制面，常驻服务 | std | 约 230 处 `.unwrap()` | release 用 `abort`；dev 为了统一构建缓存改回 unwind（[Cargo.toml:962-966, 1002-1003](https://github.com/oxidecomputer/omicron/blob/271efb284d43402110f79d0d6d842a351889ad34/Cargo.toml#L962-L1003)） | 进程退出，由外部重启（没读部署配置，未确认） |
| cargo `06413ecf` | CLI | std | 约 53 处 `.unwrap()` | 未设 | 没有 `catch_unwind` |
| uv `85b7a374` | CLI | std | `.unwrap()` | `abort`，注释「This will still show a panic message, we only skip the unwind.」（[Cargo.toml:486-491](https://github.com/astral-sh/uv/blob/85b7a374a2f6bda197915f89d32f746143beb315/Cargo.toml#L486-L491)） | 进程退出 |
| rustc `ce37c4ff` | 编译器 | 自己的 `Lock`，底层是 parking_lot 的 `RawMutex`（[lock.rs:17-18](https://github.com/rust-lang/rust/blob/ce37c4ff837c5c5dc45cddf9e1e3704e42ca67ea/compiler/rustc_data_structures/src/sync/lock.rs#L17-L18)） | 不返回 `Result` | — | ICE 报告后退出 |

和 Condr 最接近的四家：

**Zed。** 崩溃上报装上后，panic hook 依次记日志、发 minidump，最后 `std::process::abort()`（[crashes.rs:94-105](https://github.com/zed-industries/zed/blob/7f9a39a9c08373dcc62599906f8dc335d358d7c4/crates/crashes/src/crashes.rs#L94-L105)、[572-599](https://github.com/zed-industries/zed/blob/7f9a39a9c08373dcc62599906f8dc335d358d7c4/crates/crashes/src/crashes.rs#L572-L599)）。除 Dev 外的发布渠道，只要编译时设了上报地址就会装（[telemetry.rs:98-103](https://github.com/zed-industries/zed/blob/7f9a39a9c08373dcc62599906f8dc335d358d7c4/crates/client/src/telemetry.rs#L98-L103)）。编辑器和 remote server 都装（[main.rs:386-421](https://github.com/zed-industries/zed/blob/7f9a39a9c08373dcc62599906f8dc335d358d7c4/crates/zed/src/main.rs#L386-L421)、[remote_server/server.rs:679-705](https://github.com/zed-industries/zed/blob/7f9a39a9c08373dcc62599906f8dc335d358d7c4/crates/remote_server/src/server.rs#L679-L705)）。hook 在任何 `catch_unwind` 之前执行，所以 GPUI 里那些 `catch_unwind` 在这样的构建里接不到东西。remote server 唯一的接住点包着整个 `app.run`，只为了记一句日志再退出：「We do not reuse any of the state after unwinding」（[server.rs:839-842](https://github.com/zed-industries/zed/blob/7f9a39a9c08373dcc62599906f8dc335d358d7c4/crates/remote_server/src/server.rs#L839-L842)）。也就是说，Condr 所用 UI 栈的上游，选的是 unwind 构建，再由 hook 统一 abort。

**zellij。** server 的 panic hook 把非 main 线程的 panic 转成 `ServerInstruction::Error` 发给主循环（[lib.rs:3813-3821](https://github.com/zellij-org/zellij/blob/1f3df933978e00c246553070310175c392fcfe4f/zellij-server/src/lib.rs#L3813-L3821)、[errors.rs:939-997](https://github.com/zellij-org/zellij/blob/1f3df933978e00c246553070310175c392fcfe4f/zellij-utils/src/errors.rs#L939-L997)）。主循环收到后，让所有 Client 带着错误退出，然后 `break`（[lib.rs:4660-4671](https://github.com/zellij-org/zellij/blob/1f3df933978e00c246553070310175c392fcfe4f/zellij-server/src/lib.rs#L4660-L4671)）。每个会话是一个进程，所以影响范围是一个会话。

**wezterm。** 每个 Pane 有一个 reader 线程和一个 parser 线程（[lib.rs:283-318](https://github.com/wezterm/wezterm/blob/e68a8ec27727fbbda15e07f0a0737cd5e16481f6/mux/src/lib.rs#L283-L318)），都没有 `catch_unwind`。hook 只记日志，再交给默认 hook（[env-bootstrap lib.rs:166-185](https://github.com/wezterm/wezterm/blob/e68a8ec27727fbbda15e07f0a0737cd5e16481f6/env-bootstrap/src/lib.rs#L166-L185)）。推断：parser 线程 panic 后，parking_lot 照常释放锁，这个 Pane 不再更新；mux 继续运行，既不退出，也不把 Pane 标成失败。

**herdr**（与 FFI 调研同一提交）：

- 规则是「no `unwrap()` in production code」（[AGENTS.md:253](https://github.com/herdrdev/herdr/blob/bc8c28988f75c538d4ca058b702f5adbea30566e/AGENTS.md#L253)）。
- 锁 Pane 终端时用 `.ok()?` 或 `let Ok(..) else`，锁被毒化时那个 Pane 的操作返回空、不再动作（[pane/terminal.rs:312](https://github.com/herdrdev/herdr/blob/bc8c28988f75c538d4ca058b702f5adbea30566e/src/pane/terminal.rs#L312)、[1214](https://github.com/herdrdev/herdr/blob/bc8c28988f75c538d4ca058b702f5adbea30566e/src/pane/terminal.rs#L1214)）。
- 小锁直接 `into_inner`（[pane.rs:3597-3600](https://github.com/herdrdev/herdr/blob/bc8c28988f75c538d4ca058b702f5adbea30566e/src/pane.rs#L3597-L3600)）。
- 关键差别是没有全局状态锁：`App` 由一个任务独占，经 mpsc 收事件（[app/mod.rs:105-111](https://github.com/herdrdev/herdr/blob/bc8c28988f75c538d4ca058b702f5adbea30566e/src/app/mod.rs#L105-L111)）。Pane 的后台工作跑在 tokio 任务里（如 [pane.rs:1353](https://github.com/herdrdev/herdr/blob/bc8c28988f75c538d4ca058b702f5adbea30566e/src/pane.rs#L1353)），tokio 默认会接住这些 panic。
- 所以 herdr 不会出现 Condr 这样的连锁：一个 Pane 出事，只有那个 Pane 停下。代价是 Pane 静默失效，没有人把它标成已退出（推断）。

**rust-analyzer** 的注释最直接（[dispatch.rs:34-44](https://github.com/rust-lang/rust-analyzer/blob/889bb47e853003f62b0a593b955ab9afc97e7885/crates/rust-analyzer/src/handlers/dispatch.rs#L34-L44)）：「Read-only requests are wrapped into `catch_unwind` -- they don't modify the state, so it's OK to recover from their failures」。改全局状态的请求在主循环上运行，不包，注释是「please, don't make bugs :-)」。

FFI 调研里的 Paneflow 也是同一思路：常驻 host 给每个会话的服务循环包 `catch_unwind`，panic 时丢掉引擎、保住子进程。

归纳：

1. **锁的类型和 panic 策略无关。** parking_lot 的使用者里，有的整进程 abort（Zed），有的按请求隔离（rust-analyzer），也有什么都不做的（wezterm）。std 的使用者也一样分散。
2. **两种自洽的策略。** 一是在「不共享可变状态」的单元边界接住：快照上的请求、tokio 任务、HTTP 请求、独立的会话或进程。二是整个进程 fail-fast，让外面重启：Zed、zellij、deno、omicron、uv。会改共享状态的代码，两种策略下都不接住 panic。
3. **样本里没有谁用 `into_inner` 或 `clear_poison` 去「修好」重要的共享状态再继续用。** 用 `into_inner` 的地方，锁里要么是计数、缓冲区这类无所谓的数据，要么像 tokio 内部那样自己维护不变量。

## 三、被广泛引用的说法

- **std 文档**：默认 `unwrap()`，让 panic 在线程间传播；毒化只是建议，检测也不完整（第一节）。
- **Rust Book** ch16-03：「The call to `lock` would fail if another thread holding the lock panicked. In that case, no one would ever be able to get the lock, so we've chosen to `unwrap` and have this thread panic」（本地 1.99.1 文档）。
- **Nomicon**《Poisoning》：毒化不是为了内存安全，而是防止「blindly using the data that comes out of a Mutex that has witnessed a panic」，「the program should not proceed normally」（本地 1.99.1 文档）。
- **Google Rust Style Guide**：「Never reach inside a poisoned mutex to get at its insides, no matter how tempting the use case may seem」（[#handling-poison](https://google.github.io/styleguide/rustguide.html#handling-poison)）。第一方代码禁止 `catch_unwind`，只允许作为「defense in depth」，前提是「never _relied_ upon for any properties」（[#dont-call-paniccatch_unwind](https://google.github.io/styleguide/rustguide.html#dont-call-paniccatch_unwind)）。
- **Rust API Guidelines**：没有关于毒化或 `catch_unwind` 的条目（`api-guidelines` `97a0969c` 全文检索）。
- **libs 团队 2020 年的问卷**（[博文](https://blog.rust-lang.org/2020/12/11/lock-poisoning-survey/)，[原始数据](https://github.com/rust-lang/libs-team/tree/21395015de40a042dedb97705b62d7f8ccdb21a1/data/2020-12-11-poisoning-survey)）。博文说毒化「isn't actually a tool for [keeping invariants] in the way you might think」，它只是把 panic 传到别的线程。863 份回答里：
  - 828 人答了「怎么拿锁」，698 人写 `.lock().unwrap()`，52 人用 `into_inner`；
  - 652 人答了「毒化时想要什么」，411 人选「I want my program to terminate」；
  - 793 人答了「是否受益于默认毒化」，538 人答「否」；
  - 用第三方锁的首要理由是性能（339/541），那是 futex 实现之前。
- **Niko Matsakis**《[Unwind considered harmful?](https://smallcultfollowing.com/babysteps/blog/2024/05/02/unwind-considered-harmful/)》（2024-05-02）：
  - 毒化是「an ergonomic annoyance and an imperfect solution」；
  - 事件驱动的服务靠 unwind 恢复，有共享状态时「a bit risky」，建议改用进程级恢复；
  - 实际的网络服务要么 `panic=abort`，要么并没有真正靠 unwind 来恢复。
- **BurntSushi**《[Using unwrap() in Rust is Okay](https://burntsushi.net/unwrap/)》（2022-08-08）：panic 表示 bug，「correct Rust programs don't panic」；`mutex.lock().expect("an unpoisoned mutex")` 这类 `expect` 不比 `unwrap` 多传达什么。
- **Rain（Oxide）**《[In defense of lock poisoning in Rust](https://sunshowers.io/posts/on-poisoning/)》（2025-12-02）：
  - 「What is important is detection, not recovery」；
  - Oxide 的大部分产品 abort on panic，这样「guarantees that in-memory state is torn down」；
  - 但可复用的库不能要求使用者 `panic = "abort"`，所以默认毒化有价值。

[rust#149359](https://github.com/rust-lang/rust/issues/149359) 和 [#134645](https://github.com/rust-lang/rust/issues/134645) 里两边的论点：

- **支持默认毒化。**
  - bjorn3：没有毒化，「you are way too likely to keep limping around without making any forward progress ever again when an essential background thread panicked」。他主张让 `lock()` 遇到毒化直接 panic，另设一个返回 `Result` 的方法（[3583364001](https://github.com/rust-lang/rust/issues/149359#issuecomment-3583364001)）。
  - davepacheco：两种默认都可能留下一个「broken-but-alive」的程序，但「explicitly producing a clear error is a much, _much_ safer way for the program to fail than to read invalid state and drive on」（[3615473394](https://github.com/rust-lang/rust/issues/149359#issuecomment-3615473394)）。
  - bjorn3 又引了 Google 的规则（[6399387451](https://github.com/rust-lang/rust/issues/149359#issuecomment-6399387451)）。
- **支持不毒化，或把毒化从锁里拆出去。**
  - Amanieu（libs-api 成员，parking_lot 作者）：毒化管的是「code that doesn't clean up properly after itself when a panic happens」，和锁是两回事。他提议用 `Poison<T>` 单独表达，现在的 `Mutex<T>` 就等于 `Mutex<Poison<T>>`；unsafe 代码要自己保证 unwind 安全（[3590942770](https://github.com/rust-lang/rust/issues/134645#issuecomment-3590942770)）。parking_lot 的文档就写着「No poisoning, the lock is released normally on panic」（[mutex.rs:24](https://github.com/Amanieu/parking_lot/blob/85827ffa2ffb0b60d416e2e2a091f36b0e8e5a91/src/mutex.rs#L24)）。
  - oconnor663 描述的场景正是 Condr 的这个 bug：每个连接一个线程，一个请求持着要紧的锁 panic，毒化让「~all requests after that」都 panic，直到看门狗发现并重启（[3587625097](https://github.com/rust-lang/rust/issues/149359#issuecomment-3587625097)）。
- **两边都同意的。**
  - 毒化是检测，不是恢复；
  - 不要读被毒化的数据；
  - `catch_unwind` 只放在粗粒度的边界上；
  - 「活着但不干活」是最差的结局。

## 四、放到 Condr

### 现状

生产代码里共有 153 处 `expect("… poisoned")`，按锁分：

| 锁 | 所在 | 处数 |
| --- | --- | --- |
| Server 状态锁 `Arc<Mutex<RuntimeState>>`（[server.rs:112](../../crates/condr-server/src/server.rs#L112)） | condr-server | 67 |
| 其他 Server 级的锁：Snapshot worker 7、lifecycle 6、layout 4、Git activity 2、settings write 1 | condr-server | 20 |
| Pane 级的锁：终端 21、notices 11、size 6、mouse press 5、reply queue 5、resize 4、PTY master 3、cwd 3、cursor settle 3、child 1、damage baseline 1 | condr-core `terminal/` | 63 |
| 进程表快照 1、Companion 的 name 2 | condr-core、condr-mobile | 3 |

另外还有 18 处 `.lock().unwrap()`，主要在 `ssh/stream.rs`；约 14 处已经用 `into_inner`，在 condr-client、GUI、`client_writer.rs`、`input_queue.rs` 和 `files/write.rs`。core 和 server 里提到 `Mutex` 或 `Condvar` 的文件有 26 个，Condvar 等待 9 处。连锁的过程见 [FFI 回调调研第四节](ffi-callback-panics.md)。core 和 server 的代码从那次调研到现在没有变。

### 四种做法

| | 正确性：panic 后有没有人读到改了一半的状态 | 影响范围 | 改动量 | 依赖 | 和 libghostty-vt 回调里的 `catch_unwind` |
| --- | --- | --- | --- | --- | --- |
| (a) std，加隔离和 fail-fast 两个 helper | 没有。被毒化的 Pane 锁不再读，Pane 按退出处理；被毒化的 Server 级锁让下一个拿锁的线程 abort，之后不会再有变更把坏掉的 Session 写进 Snapshot（推断） | reader panic：一个 Pane。持 Server 状态锁时 panic：整个 Server，GUI 重连时拉起新的。Client 线程不持锁时 panic：只断这一个连接 | 87 处换成 `lock_or_abort`，是机械替换；Pane 级 63 处里，从 reader 线程以外调用的要改成「被毒化就当已退出」，需要逐处判断；reader 循环包一层；加一个测试 | 无 | 两层叠加：回调里的 helper 接住，终端锁就不会被毒化；漏掉的由 reader 那层接住。两层都要求 unwind，正是现在的设置 |
| (b) parking_lot，加隔离 | 有。reader 持终端锁 panic 后，别的线程照样拿到改了一半的 `Terminal`。Client 线程持 `RuntimeState` panic 后，别的 Client 继续改这个 Session，下一次变更就把它写进 Snapshot，重启后还在（推断）。要堵住，就得在每个持锁的地方包 `catch_unwind`，或在 guard 的 `Drop` 里查 `thread::panicking()`，这等于重写 std 的毒化 | 不自制标记时最小，像 wezterm，代价是在可能坏掉的状态上继续跑 | 最大：26 个文件换类型；删掉 153 处 `expect` 和 18 处 `unwrap`；Condvar 的 9 处等待改成 parking_lot 的 `wait(&mut guard)`；再加上自制的毒化 | core 和 server 不新增 crate：gix 已经带进 parking_lot 0.12.5，alacritty_terminal 迁走后它仍在；condr-client 和 condr-mobile 今天不依赖它 | 回调里的 helper 照常工作；reader 那层接住后，要自己把 Pane 标成失败 |
| (c) std `nonpoison` | 同 (b) | 同 (b) | 同 (b) | 1.99.1 上不稳定，`rust-toolchain.toml` 固定在 stable，**用不了** | 同 (b) |
| (d) `panic = "abort"` | 最好：panic 之后没有任何代码继续运行 | 最大。任何线程的任何 panic，比如某个 Pane 的 reader、某个 Client 的编码 bug、Git watcher，都会结束整个 Server，杀掉所有 PTY 里的 shell 和 Agent 正在进行的回合。本地由 GUI 或 CLI 的 `ensure_local_server` 拉起（[startup.rs:193-215](../../crates/condr-gui/src/app/startup.rs#L193-L215)），经 SSH 的远程由 `server bridge` 拉起；只开 TCP 或 Peer-to-peer 的远程 Server 没有人拉，Condr 没有配置服务管理器的重启 | 1 行。`expect` 可以保留：abort 发生在 unwind 之前，锁永远不会被毒化 | 无。但 profile 对整个 workspace 生效，「Overrides cannot specify the `panic` … settings」，GUI 也会一起 abort；而且「Tests … ignore the `panic` setting」，测试里看到的仍是 unwind（[profiles.md:192-211, 638](https://github.com/rust-lang/cargo/blob/06413ecf8cd3ec5b10b93ac2b85f6fad34f8597c/doc/book/src/reference/profiles.md#L192-L211)） | 回调里的 `catch_unwind` 成了死代码，FFI 调研第六节第 2 条白写；回调里一 panic 就 abort |

Zed 那种「unwind 构建，hook 里 abort」与 (d) 的影响范围相同。它的好处是可以按渠道开关，代价是同样让所有 `catch_unwind` 失效，因为 hook 先执行。

关于 (a) 的检测缺口：std 列出的几种漏检情形是 `Drop` 或 hook 里的二次 panic，以及外来异常。它们与 Condr 无关：回调保持 `extern "C"`，Zig 不会把异常抛进 Rust。

### 建议：(a)

1. **Server 级：毒化即 abort。** 加一个 `fn lock_or_abort<T>(mutex: &Mutex<T>, name: &str) -> MutexGuard<'_, T>`：拿到 `Err` 时先 `tracing::error!`，再 `std::process::abort()`。用它替换 67 处 Server 状态锁和 20 处其他 Server 级的锁。不要 `into_inner`，也不要 `clear_poison`（Google 的规则、Nomicon）：`RuntimeState` 是会被持久化的权威 Session。原始 panic 已由现有的 hook 同步写进 stderr 日志；tracing 的 non-blocking 层在 abort 时可能丢掉最后几行，debounce 中还没写盘的 Snapshot 也会丢（推断）。
2. **Pane 级：毒化即退出。**
   - reader 循环外包一层 `catch_unwind(AssertUnwindSafe(..))`。panic 时记一条带 Pane ID 的 warn，然后照常走到 `updates.send(TerminalUpdate::Exited)`（[pty_io.rs:108](../../crates/condr-core/src/terminal/pty_io.rs#L108)）。monitor 收到 `Exited` 后会 `runtime.close()` 停掉进程树，再读一次最终视图（[terminal_monitor.rs:74-110](../../crates/condr-server/src/server/terminal_monitor.rs#L74-L110)）。这次读也要能应对被毒化的终端，退回上一帧或空视图。
   - reader 线程以外锁 Pane 的地方，一律改成 `.lock().ok()`，`None` 就当终端已退出，与 herdr 的 `.ok()?` 相同。这里不读被毒化的数据，只是不再用它，所以不违背 Google 的规则。
   - libghostty-vt 迁移本来就要重写 `pty_io` 和视图路径（ADR 0051），这一步最好放在迁移里一起做。
3. **保持 unwind，保留回调 helper。** 也就是 FFI 调研第六节的做法，不加 `panic = "abort"`，也不让 hook abort。
4. **加一个测试。** 让一个 Pane 的 reader 强制 panic，断言：这个 Pane 收到 `Exited`；另一个 Pane 照常回显；新的 Client 仍能完成握手。abort helper 只有三行，不另写测试。
5. **不做 parking_lot 和 `nonpoison`。** Edition 2027 有了决定再回头看。届时若换成不毒化的锁，第 1、2 条依赖的那一位就得自己维护。

**这个方案没有覆盖的缺口。** 线程不持任何锁时 panic，比如 Snapshot worker、某个 Pane 的 monitor、Git watcher 中途退出，今天和方案 (a) 下都会静默消失，就是 bjorn3 说的「keep limping around」。deno 和 zellij 的做法是让这种 panic 结束整个进程。Condr 是否对「必须一直活着」的线程也这样处理，是另一个决定，建议另开 issue。

## 未确认

- 各项目里 Pane 线程或任务 panic 之后的端到端结局（wezterm、alacritty、herdr、zellij）只读了代码，没有实际运行。
- 第二节的计数是正则粗算。parking_lot、async 锁和自定义 helper 的 `.lock()` 都算作「不返回 `Result`」，没有逐个区分。
- omicron 由什么重启，没有读它的部署配置。Zed 的官方构建是否都设了上报地址，从而都会 abort：推断如此，没有核实。
- rust#149359 的走向：只读到 2026-10-25 的评论，之后可能已有 libs-api 的决定。
- Condr 里有两条是读代码得出的，没有实测：一是「parking_lot 下后续变更会把被改坏的 Session 写进 Snapshot」；二是「`Exited` 路径会读到被毒化的终端」。
- 方案 (a) 对 Pane 级 63 处锁的拆分（哪些只在 reader 线程里用、哪些会被别的线程用到）还没有逐处核对。
