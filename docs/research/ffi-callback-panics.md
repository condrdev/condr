# FFI 回调里的 panic：嵌入 libghostty-vt 时怎么处理

调查日期：2026-10-29，基于 Condr 提交 `c45efcd`，工具链 Rust 1.99.1。libghostty-vt 对照 ghostty-org/ghostty `46e9dc9a`，与几篇 libghostty-vt 迁移调研是同一个提交。其他项目读的都是 2026-10-24 至 10-28 的默认分支，提交号写在各处链接里。除第一节的 Rust↔Zig 小实验外只读代码，没有改 Condr。

结论：回调边界上只有三种合法做法。一是吞掉：`catch_unwind` 接住 panic，给 libghostty-vt 一个"不回答"的返回值。二是暂存后重抛：接住后先存起来，等 `vt_write` 返回，再在 Rust 里 `resume_unwind`。三是 abort：什么都不写，Rust 1.81 起从 `extern "C"` 函数漏出去的 panic 就会 abort。让 panic 穿过 Zig 的栈帧是未定义行为。实测中，Zig 默认生成 unwind 表时 panic 能穿过去，被外层接住，但 Zig 的 `defer` 没有执行；关掉 unwind 表则直接 abort。这三种做法在成熟项目里都有：git2-rs、curl-rust 和 rust-openssl 的 IO 回调暂存后重抛，rusqlite 吞掉，cxx 和 libghostty-rs 明确选 abort。libghostty-vt 的几家嵌入方（herdr、gpui-cn、Paneflow、JetBrains）做法一致：回调只把数据拷进队列，`vt_write` 返回后再处理，回调里几乎没有会 panic 的代码。其中只有 Paneflow 另加了 `catch_unwind`，把 panic 记成一个事件，事后让那一个终端失败。放到 Condr 里，暂存后重抛最差。panic 会回到持有终端锁的 reader 线程，按今天的代码，这会毒化终端锁；之后有线程持着 Server 状态锁去碰这个 Pane，就会再 panic，最终整个 Server 不再响应，进程却不退出。所以原先的计划（每个回调体包 `catch_unwind`、记日志、丢掉这个事件）基本成立，但要加两个前提：回调只做拷贝、不拿锁；接住后不重抛。这样 libghostty-vt 看到的只是一次正常返回，状态一致，损失只有这一个事件，比如一次 DA1 应答或一条 OSC 7501 报告。回调保持 `extern "C"`，helper 漏掉的 panic 仍然 abort。今天 reader 线程里任何普通的 Rust panic 也会引发上面的连锁，那与 FFI 无关，另行决定。

## 一、语言规则（Rust 1.99.1）

### panic 到达 `extern "C"` 边界

- Rust 1.71 稳定了 `"C-unwind"` 等带 `-unwind` 后缀的 ABI。1.81 完成了这次过渡：不带 `-unwind` 的 ABI（如 `"C"`）遇到没被接住的 unwind 时 abort，「closing the longstanding soundness problem」（[1.81 发布说明](https://blog.rust-lang.org/2024/09/05/Rust-1.81.0/)、[1.71 发布说明](https://blog.rust-lang.org/2023/07/13/Rust-1.71.0/)）。
- Reference 的表格：`panic=unwind` 时，panic 到达非 unwind ABI 的边界会 abort；外来异常（native unwind）到达这种边界则是未定义行为。转成 abort 时，边界之前的析构函数可能全部执行，也可能一个都不执行，标准没有规定是哪种（[functions.md:253-261](https://github.com/rust-lang/reference/blob/3989fe91f1a601b51684a582ad17b2503f221787/src/items/functions.md#L253-L261)）。
- `catch_unwind` 文档的说法：从不支持 unwind 的外部代码调用的 Rust 函数应当定义成 `extern "C"`，panic 时进程自动 abort；「If this is the desired behavior, it is not necessary to use catch_unwind explicitly」（[std 1.99.1](https://doc.rust-lang.org/1.99.1/std/panic/fn.catch_unwind.html)）。
- 实测（macOS arm64，rustc 1.99.1）：`extern "C"` 回调里 panic，panic hook 先后执行两次，第一次是原来的 panic，第二次是 `panic in a function that cannot unwind`。随后打印 `thread caused non-unwinding panic. aborting.`，进程以 134（SIGABRT）退出。包在 C 调用外层的 `catch_unwind` 接不住。

### `extern "C-unwind"` 允许什么

`"C-unwind"` 让 unwind 合法地穿过这道 ABI 边界，前提是另一侧的代码支持 unwind。Reference 明确列出两种未定义行为（[panic.md:118-140](https://github.com/rust-lang/reference/blob/3989fe91f1a601b51684a582ad17b2503f221787/src/panic.md#L118-L140)）：

- 外部代码经由 `"C"` 等非 unwind ABI 声明把 unwind 带进 Rust；
- 从不支持 unwind 的代码调用会 unwind 的 Rust 函数，原文举的例子是用 `-fno-exceptions` 编译的 C。

外部运行时也不能处理或重抛 Rust 的 panic 负载：「an unwind originated from a Rust runtime must either lead to termination of the process or be caught by the same runtime」。Nomicon 的说法相同，并给出「抢先 catch」的写法（[ffi.md:747-770, 849-876](https://github.com/rust-lang/nomicon/blob/b0bb569883a3e65ab19d4f1adf1b8dbc1629d0a6/src/ffi.md#L747-L876)）。设计依据见 [RFC 2945](https://github.com/rust-lang/rfcs/blob/1dc48d761daaa2a0c80135d91f30d20ebdfb97b2/text/2945-c-unwind-abi.md)。

### 能不能穿过 Zig 的栈帧

不能。Zig 没有异常机制，也没有 landing pad，`defer`/`errdefer` 只在正常返回和返回 error 时执行。libghostty-vt 的头文件只说回调「invoked synchronously during VT writes」，并且不可重入（[terminal.h:79-84](https://github.com/ghostty-org/ghostty/blob/46e9dc9a927b2e7e7cba79e6902c176127dbdf2c/include/ghostty/vt/terminal.h#L79-L84)），没有任何地方允许 unwind 穿过它。Ghostty 构建 lib-vt 时没有设置 `unwind_tables`（`src/build/GhosttyLibVt.zig` 里搜不到），用的是 Zig 的默认值。

实验：Zig 0.17.0 写一个 `export fn zig_call(cb) void { defer defers_run += 1; cb(); }`，用 `-OReleaseFast` 编成静态库；Rust 把 `zig_call` 和回调都声明成 `"C-unwind"`，回调里 panic，外层 `catch_unwind`。

| 构建 | 结果 |
| --- | --- |
| ReleaseFast，Zig 默认 | `caught=true defers_run=0`：panic 穿过 Zig 帧被接住，Zig 的 `defer` 没有执行 |
| ReleaseFast，`-fno-unwind-tables` | `fatal runtime error: failed to initiate panic, error 5, aborting`，退出码 134 |
| 回调用 `extern "C"` | 回调边界上 abort（见上一小节） |

第一行最危险：表面上"能用"，实际上 Zig 侧的清理被悄悄跳过了，换到 libghostty-vt 里就是解析器、页表或分配器停在半截。这只在 macOS arm64 上用玩具库测过，Linux 和 Windows 的默认值未测。

所以「在回调里 catch，等 C 调用返回后在 Rust 里 `resume_unwind`」是可行的：重抛只发生在 Rust 帧里，这正是 `resume_unwind` 文档写明的用途，「carry a panic across a layer of C code」（[std 1.99.1](https://doc.rust-lang.org/1.99.1/std/panic/fn.resume_unwind.html)）。可选的只有吞掉、暂存后重抛、abort 三种。

### `catch_unwind` 本身的限制

- 只在 `panic=unwind` 下有效。`panic=abort` 时 panic 直接 abort，`resume_unwind` 也是 abort。
- 自定义 panic hook 在 catch 之前执行。Condr 的 hook 会记一条带 backtrace 的 error（第四节），所以吞掉也不会丢日志。
- `Err` 里的负载在 drop 时可能再次 panic。这发生在 `extern "C"` 函数里会 abort，作为最后一道保险可以接受。
- 闭包要满足 `UnwindSafe`，回调闭包通常要用 `AssertUnwindSafe`。

## 二、成熟的 Rust FFI crate 怎么做

| 项目 | 结局 | 机制 |
| --- | --- | --- |
| git2-rs | 暂存后重抛 | 线程局部的 `LAST_ERROR`。`wrap()` 接住 panic 后存起来，返回 `None`；回调据此返回 `GIT_EUSER`，libgit2 中止这次操作。已有暂存时，`wrap()` 不再调用用户闭包。`try_call!` 在 C 调用返回错误时调 `check()`，后者 `resume_unwind`（[panic.rs:4-29](https://github.com/rust-lang/git2-rs/blob/b10831482c0c3ce7a7cd7e7bd452082a915d4cf4/src/panic.rs#L4-L29)、[call.rs:11-18](https://github.com/rust-lang/git2-rs/blob/b10831482c0c3ce7a7cd7e7bd452082a915d4cf4/src/call.rs#L11-L18)、[diff.rs:351-372](https://github.com/rust-lang/git2-rs/blob/b10831482c0c3ce7a7cd7e7bd452082a915d4cf4/src/diff.rs#L351-L372)） |
| curl-rust | 暂存后重抛 | 与 git2 同一套设计（作者相同）。`header_cb` 在 panic 后返回 `!0`，让 curl 中止传输；`perform()` 返回后无条件 `propagate()`（[panic.rs:5-34](https://github.com/alexcrichton/curl-rust/blob/2fb602bb1d3a34ded4548cbe8a7c7202b1ab4ba2/src/panic.rs#L5-L34)、[handler.rs:3200-3204](https://github.com/alexcrichton/curl-rust/blob/2fb602bb1d3a34ded4548cbe8a7c7202b1ab4ba2/src/easy/handler.rs#L3200-L3204)、[3551-3566](https://github.com/alexcrichton/curl-rust/blob/2fb602bb1d3a34ded4548cbe8a7c7202b1ab4ba2/src/easy/handler.rs#L3551-L3566)） |
| rust-openssl | 混合 | 密码回调和 BIO 读写：panic 存进 `CallbackState.panic` 或 BIO 状态，OpenSSL 返回后由 `Drop` 或 `check_panic` 重抛（[util.rs:9-69](https://github.com/rust-openssl/rust-openssl/blob/ad29ed570acf233a2239ecb07747a147d11d42e8/openssl/src/util.rs#L9-L69)、[bio.rs:85-125](https://github.com/rust-openssl/rust-openssl/blob/ad29ed570acf233a2239ecb07747a147d11d42e8/openssl/src/ssl/bio.rs#L85-L125)、[ssl/mod.rs:4006-4010](https://github.com/rust-openssl/rust-openssl/blob/ad29ed570acf233a2239ecb07747a147d11d42e8/openssl/src/ssl/mod.rs#L4006-L4010)）。证书校验等 SSL 回调不 catch，里面用 `expect`，panic 就 abort；一个回归测试的注释直说以前「aborted the process via a panic across an extern "C" boundary」（[callbacks.rs:33-60](https://github.com/rust-openssl/rust-openssl/blob/ad29ed570acf233a2239ecb07747a147d11d42e8/openssl/src/ssl/callbacks.rs#L33-L60)、[test/mod.rs:1638-1642](https://github.com/rust-openssl/rust-openssl/blob/ad29ed570acf233a2239ecb07747a147d11d42e8/openssl/src/ssl/test/mod.rs#L1638-L1642)） |
| rusqlite | 吞掉 | 用户函数 panic 时向 SQLite 报 `Error::UnwindingPanic`（[functions.rs:607-617](https://github.com/rusqlite/rusqlite/blob/2ff4963910f6bd65824358775fc2ad781bf2533b/src/functions.rs#L607-L617)）。commit、busy、WAL 钩子用 `unwrap_or_default()`，当作钩子返回了默认值（[hooks/mod.rs:393-415, 506-516](https://github.com/rusqlite/rusqlite/blob/2ff4963910f6bd65824358775fc2ad781bf2533b/src/hooks/mod.rs#L393-L516)、[busy.rs:59-70](https://github.com/rusqlite/rusqlite/blob/2ff4963910f6bd65824358775fc2ad781bf2533b/src/busy.rs#L59-L70)）。trace 回调直接 `drop(catch_unwind(..))`（[trace.rs:34](https://github.com/rusqlite/rusqlite/blob/2ff4963910f6bd65824358775fc2ad781bf2533b/src/trace.rs#L34)）。排序规则 panic 时返回 `-1`，注释是「FIXME How ?」（[collation.rs:74-89](https://github.com/rusqlite/rusqlite/blob/2ff4963910f6bd65824358775fc2ad781bf2533b/src/collation.rs#L74-L89)） |
| mlua | 转成 Lua 错误，回到 Rust 时重抛 | 接住后包成 `WrappedFailure::Panic`，以 `lua_error` 抛给 Lua；这个值回到 Rust 时 `resume_unwind`（[state/util.rs:119-182](https://github.com/mlua-rs/mlua/blob/ff96ed786295cae96f43ef1372bd22be0fbf7866/src/state/util.rs#L119-L182)、[state/raw.rs:970-979](https://github.com/mlua-rs/mlua/blob/ff96ed786295cae96f43ef1372bd22be0fbf7866/src/state/raw.rs#L970-L979)）。选项 `catch_rust_panics` 决定 Lua 的 `pcall` 能否接住（[state.rs:210-218](https://github.com/mlua-rs/mlua/blob/ff96ed786295cae96f43ef1372bd22be0fbf7866/src/state.rs#L210-L218)）。GC finalizer 里 panic 则 `process::abort()`（[userdata/util.rs:443-450](https://github.com/mlua-rs/mlua/blob/ff96ed786295cae96f43ef1372bd22be0fbf7866/src/userdata/util.rs#L443-L450)）。mlua 用 `"C-unwind"` 是因为 Lua 的错误会穿过 Rust 帧，方向与 Condr 相反 |
| PyO3 | 转成 Python 异常，回到 Rust 时重抛 | trampoline 把 panic 转成 `PanicException`（[trampoline.rs:302-339](https://github.com/PyO3/pyo3/blob/26f9ec8be637d24d0dee18d51e129cc1a99f3a7c/src/impl_/trampoline.rs#L302-L339)），Rust 取回这个异常时 `resume_unwind`（[err/mod.rs:300-321](https://github.com/PyO3/pyo3/blob/26f9ec8be637d24d0dee18d51e129cc1a99f3a7c/src/err/mod.rs#L300-L321)）。漏出去的由 `PanicTrap` 二次 panic 变成 abort（[impl_/panic.rs:6-27](https://github.com/PyO3/pyo3/blob/26f9ec8be637d24d0dee18d51e129cc1a99f3a7c/src/impl_/panic.rs#L6-L27)） |
| cxx | abort，写进文档 | `prevent_unwind` 放一个 guard，unwind 时 guard 再 panic，变成 abort（[unwind.rs:5-39](https://github.com/dtolnay/cxx/blob/5d7705d7fc78ec811ca97527a9c151bd3d9c7b95/src/unwind.rs#L5-L39)）。文档：「If a panic occurs in *any* `extern "Rust"` function ... the program calls Rust's `std::process::abort`」（[book result.md:15-17](https://github.com/dtolnay/cxx/blob/5d7705d7fc78ec811ca97527a9c151bd3d9c7b95/book/src/binding/result.md#L15-L17)） |

选择重抛的项目有两个共同点。第一，发起 C 调用的是用户代码，panic 回到它手里，由它决定怎么办。第二，C 库有办法提前结束这次操作，比如 git2 的 `GIT_EUSER` 和 curl 的 `!0`，或者 panic 能以宿主语言的错误形式一路传回来，比如 mlua 和 PyO3。libghostty-vt 两样都没有：回调要么返回 `void`，要么返回一个表示「答没答」的 `bool`，没法让 `vt_write` 提前停下，这一块输入剩下的部分照样解析。

## 三、libghostty-vt 的嵌入方怎么做

**libghostty-rs**（MIT，master `9ede35b8`）：abort，写进文档。trampoline 由宏生成，是 `unsafe extern "C" fn`，整个 crate 里没有 `catch_unwind`。trampoline 自己对「不该发生」的情况用 `expect`（[terminal.rs:2984-3021](https://github.com/Uzaaft/libghostty-rs/blob/9ede35b8ea94a55ed3608408d67e102883129f1e/crates/libghostty-vt/src/terminal.rs#L2984-L3021)）。文档几处提醒「a panic in a callback aborts the process」，建议回调里用 `try_borrow_mut` 这类不会 panic 的写法（[2236-2239](https://github.com/Uzaaft/libghostty-rs/blob/9ede35b8ea94a55ed3608408d67e102883129f1e/crates/libghostty-vt/src/terminal.rs#L2236-L2239)、[3176-3183](https://github.com/Uzaaft/libghostty-rs/blob/9ede35b8ea94a55ed3608408d67e102883129f1e/crates/libghostty-vt/src/terminal.rs#L3176-L3183)）。迁移调研引的 `terminal.rs@master:2239` 就是这里。测试里也只记录、不断言，因为断言失败会让整个测试二进制 abort（[4206-4207](https://github.com/Uzaaft/libghostty-rs/blob/9ede35b8ea94a55ed3608408d67e102883129f1e/crates/libghostty-vt/src/terminal.rs#L4206-L4207)）。上游新增的枚举值是一个现实风险：他们专门让 trampoline 跳过不认识的 tag，而不是在 `extern "C"` 里 `expect` 一下就 abort（[5988-5991](https://github.com/Uzaaft/libghostty-rs/blob/9ede35b8ea94a55ed3608408d67e102883129f1e/crates/libghostty-vt/src/terminal.rs#L5988-L5991)）。

**herdr**（Apache-2.0，`bc8c2898`）：靠回调写得不会 panic，没有兜底。回调只往 `TerminalCallbackState` 里记数、往 `Vec` 里 push，写完后由 `take_*` 取走（[ghostty-vt/src/lib.rs:500-575](https://github.com/herdrdev/herdr/blob/bc8c28988f75c538d4ca058b702f5adbea30566e/crates/ghostty-vt/src/lib.rs#L500-L575)、[1144-1170](https://github.com/herdrdev/herdr/blob/bc8c28988f75c538d4ca058b702f5adbea30566e/crates/ghostty-vt/src/lib.rs#L1144-L1170)）。唯一调用闭包的是 write_pty。herdr 的闭包往 `Arc<Mutex<Vec<Bytes>>>` 里 push，用的是 `if let Ok(..) = lock()`，锁被毒化时也不会 panic（[src/pane/terminal.rs:1158-1165](https://github.com/herdrdev/herdr/blob/bc8c28988f75c538d4ca058b702f5adbea30566e/src/pane/terminal.rs#L1158-L1165)）。只有 clipboard_write 包了 `catch_unwind`，因为无论成败都得调用 `reply`；panic 时回复 `INVALID_DATA`，即吞掉（[lib.rs:577-598](https://github.com/herdrdev/herdr/blob/bc8c28988f75c538d4ca058b702f5adbea30566e/crates/ghostty-vt/src/lib.rs#L577-L598)）。没有设 `panic = "abort"`。生产代码锁 Pane 的终端时用 `.ok()?`，或者 `try_lock` 后 `into_inner`，锁被毒化时那个 Pane 不再动作，而不是再 panic（[terminal.rs:312](https://github.com/herdrdev/herdr/blob/bc8c28988f75c538d4ca058b702f5adbea30566e/src/pane/terminal.rs#L312)、[2192-2198](https://github.com/herdrdev/herdr/blob/bc8c28988f75c538d4ca058b702f5adbea30566e/src/pane/terminal.rs#L2192-L2198)）。

**gpui-cn 的 `ghostty-vt`**（`e49d909b`）：abort。每个回调把一个 `Event` push 进 `Shared.events`，需要返回值的回调从预先设好的字段里取答案；写完后用 `events()` 取走（[terminal.rs:154-162](https://github.com/prabirshrestha/gpui-cn/blob/e49d909b2ceae040714aa53cd38f97faa4f85cd2/crates/ghostty-vt/src/terminal.rs#L154-L162)、[1021-1046](https://github.com/prabirshrestha/gpui-cn/blob/e49d909b2ceae040714aa53cd38f97faa4f85cd2/crates/ghostty-vt/src/terminal.rs#L1021-L1046)）。没有 `catch_unwind`。工作区的 release profile 是 `panic = "abort"`（[Cargo.toml:40-44](https://github.com/prabirshrestha/gpui-cn/blob/e49d909b2ceae040714aa53cd38f97faa4f85cd2/Cargo.toml#L40-L44)）。

**Paneflow**（GPL-3.0，`f242a90a`；只描述做法，不抄代码）：接住 panic，让那一个终端失败。所有回调体都经过同一个 helper：helper 包一层 `catch_unwind`，panic 时往事件队列放一个去重的 `CallbackPanicked`。需要返回值的回调在 panic 时报「没填」。回调只做有上限的拷贝：单次回调 64 KiB，积压的 PTY 回写 1 MiB。写完后取事件时，GUI 把 `CallbackPanicked` 当作错误，把这个终端的运行时标记为失败（`RuntimeFailed`）；每个终端的运行时线程外面也包了 `catch_unwind`。常驻的 host 进程收到这个事件只记 debug 日志并继续；它把每个会话的服务循环包在 `catch_unwind` 里，panic 时丢掉引擎、保住子进程。release profile 是 unwind，只有几个 hook 小程序用 `panic = "abort"`。有一个强制 panic 的测试，还有一个回调 p99 低于 1 ms 的测试。出处：[callbacks.rs:579-593](https://github.com/arthjean/paneflow/blob/f242a90a605122e3602adfdc23ed26163b8c5560/crates/paneflow-terminal-ghostty/src/callbacks.rs#L579-L593)、[callback_ffi.rs:18-38, 353-374](https://github.com/arthjean/paneflow/blob/f242a90a605122e3602adfdc23ed26163b8c5560/crates/paneflow-terminal-ghostty/src/callback_ffi.rs#L18-L374)、[events.rs:352-354](https://github.com/arthjean/paneflow/blob/f242a90a605122e3602adfdc23ed26163b8c5560/src-app/src/terminal/ghostty_session/events.rs#L352-L354)、[pty_runtime.rs:721-729](https://github.com/arthjean/paneflow/blob/f242a90a605122e3602adfdc23ed26163b8c5560/src-app/src/terminal/ghostty_session/pty_runtime.rs#L721-L729)、[host runtime.rs:874-884](https://github.com/arthjean/paneflow/blob/f242a90a605122e3602adfdc23ed26163b8c5560/crates/paneflow-host/src/runtime.rs#L874-L884)、[1372-1376](https://github.com/arthjean/paneflow/blob/f242a90a605122e3602adfdc23ed26163b8c5560/crates/paneflow-host/src/runtime.rs#L1372-L1376)、[Cargo.toml:132-162](https://github.com/arthjean/paneflow/blob/f242a90a605122e3602adfdc23ed26163b8c5560/Cargo.toml#L132-L162)。

**JetBrains**（intellij-community `e07c24dd`，Kotlin + Java FFM）：靠回调写得不抛异常，没有兜底。upcall stub 直接用 `Linker.upcallStub` 绑定，没有 try/catch，也没有用 `catchException`（[LibGhosttyVt.kt:347-352](https://github.com/JetBrains/intellij-community/blob/e07c24dd2504f70c01efd1344bd513c45b49a8c3/plugins/terminal/emulator/src/com/intellij/terminal/emulator/impl/ghostty/bindings/LibGhosttyVt.kt#L347-L352)）。`onWritePty` 直接调 listener（[GhosttyTerminalEmulator.kt:918-926](https://github.com/JetBrains/intellij-community/blob/e07c24dd2504f70c01efd1344bd513c45b49a8c3/plugins/terminal/emulator/src/com/intellij/terminal/emulator/impl/ghostty/GhosttyTerminalEmulator.kt#L918-L926)），注释说回调「Deliberately total」（[935-941](https://github.com/JetBrains/intellij-community/blob/e07c24dd2504f70c01efd1344bd513c45b49a8c3/plugins/terminal/emulator/src/com/intellij/terminal/emulator/impl/ghostty/GhosttyTerminalEmulator.kt#L935-L941)）。注释的理由是异常会 unwind 进 ghostty 的栈帧，Java 文档的说法则是：upcall 抛出异常，「the JVM will terminate abruptly」（[Linker, Java 25](https://docs.oracle.com/en/java/javase/25/docs/api/java.base/java/lang/foreign/Linker.html)）。所以结局等于整个 IDE abort。

共同点：五家都让回调只做拷贝，数据进队列，`vt_write` 返回后再处理。四家接受「万一 panic 就 abort」，只有 Paneflow 兜了底，兜底的结局是让那一个终端失败。没有一家暂存后重抛。

## 四、Condr 现状：今天一个 panic 会怎样

- **panic 策略是 unwind。** 根 [`Cargo.toml`](../../Cargo.toml) 只有 `[profile.dev]` 和 `[profile.dev.package]`，都没有 `panic` 设置，各 crate 的清单和 `.cargo/` 里也没有，所以 `catch_unwind` 有效。正在写的 `condr-ghostty-vt` 里，bindgen 生成的回调类型就是 `unsafe extern "C" fn`。
- **panic hook**：用 `tracing::error!` 记下 panic 信息和强制捕获的 backtrace，再交给原来的 hook 打到 stderr（[logging.rs:187-194](../../crates/condr-server/src/logging.rs#L187-L194)）。后台 Server 的 stderr 是一个 condr-server 日志文件（[server/local.rs:181-185](../../crates/condr-server/src/server/local.rs#L181-L185)），默认 hook 同步写进去。tracing 的文件层是 non-blocking，由后台线程写盘；abort 时 `WorkerGuard` 不会 drop，队列里最后几行可能丢失（推断，未实测）。
- **reader 线程在终端锁里解析。** 现在 `OscScanner` 的闭包先锁 `pending_replies` 和 `notices`，都用 `expect`；之后锁住终端，调用 `parser.advance`（[pty_io.rs:45-74](../../crates/condr-core/src/terminal/pty_io.rs#L45-L74)）。换成 libghostty-vt 后，这些闭包会变成回调，在 `vt_write` 里、在终端锁里执行。
- **core 和 server 里都没有 `catch_unwind`。** reader 线程持着终端锁时 panic：线程结束，终端锁被毒化，循环之后的 `TerminalUpdate::Exited` 也不会发出（[pty_io.rs:108](../../crates/condr-core/src/terminal/pty_io.rs#L108)），Pane 看起来像卡住了。直到关闭时 `join` 才把它变成「terminal reader panicked」错误（[prelude.rs:94-100](../../crates/condr-core/src/terminal/prelude.rs#L94-L100)）。
- **连锁反应**（推断，读代码得出，未实测）：用户在这个"卡住"的 Pane 里按一个键，Client 线程会先拿 Server 状态锁（[client.rs:491](../../crates/condr-server/src/server/client.rs#L491)），再调 `execute`（[client.rs:570](../../crates/condr-server/src/server/client.rs#L570)）。`execute` 用 `expect("terminal state lock poisoned")` 锁终端（[runtime.rs:342-346](../../crates/condr-core/src/terminal/runtime.rs#L342-L346)），于是在持有 Server 状态锁时 panic，Server 状态锁也被毒化。`print_notice`（[agents.rs:222-223](../../crates/condr-server/src/server/agents.rs#L222-L223)）和焦点清除（[terminal_state.rs:25-31](../../crates/condr-server/src/server/terminal_state.rs#L25-L31)）也是同样的路径。此后每处 `state.lock().expect("server state lock poisoned")` 都会 panic，包括新连接握手的 `send_welcome`（[bootstrap.rs:117-127](../../crates/condr-server/src/server/bootstrap.rs#L117-L127)）和各 Pane 的 monitor 线程。结果是 Server 不再响应任何 Client，进程却不退出，PTY 和 Agent 在没人能看的地方继续跑。

也就是说，Condr 今天没有 Pane 级的隔离。一个 Pane 里的 bug 已经会拖垮整个 Server，只是来得慢，而且没有声响。拿这个做参照，回调边界的三种做法分别改变了：

- **abort**：影响范围和今天一样，但立即发生、看得见。进程退出，PTY 关闭，shell 和 Agent 收到 SIGHUP。下次启动时按 ADR 0005 恢复结构，再用会话 ID 续上 Agent；正在进行的回合和 scrollback 都丢了。
- **吞掉**：影响范围是一个事件。
- **暂存后重抛**：重抛发生在 reader 线程、终端锁里，正好走进上面的连锁。只有 reader 线程先有了自己的隔离，这种做法才有意义。

## 五、三种做法放到 Condr 里比较

| 做法 | 这个 Pane | 其他 Pane | libghostty-vt 的状态 | Condr 自己的状态 | 代码量 | 谁这么做 |
| --- | --- | --- | --- | --- | --- | --- |
| abort（什么都不写） | 结束 | 全部结束；重启后续上 Agent，当前回合和 scrollback 丢失 | 无所谓 | 无所谓 | 0 | libghostty-rs、cxx、herdr、gpui-cn、JetBrains |
| 吞掉 | 继续运行，丢一个事件 | 不受影响 | 一致：Zig 只看到一次正常返回 | 一致，前提是回调只做一次 push、不拿锁 | 一个十来行的 helper | rusqlite、herdr 的 clipboard_write |
| 暂存后重抛 | reader 线程 panic，终端锁被毒化 | 按今天的代码连锁到整个 Server，进程不退出 | 一致 | 终端锁被毒化 | helper，加上每次 `vt_write` 后检查 | git2、curl、openssl IO、mlua、PyO3 |
| 事后让这个 Pane 失败 | 结束，Agent 被杀 | 不受影响 | 一致 | 一致 | helper，加一条 Pane 失败路径（可复用 PTY EOF 的 `Exited`） | Paneflow |

题目里担心「吞掉后终端可能停在解析中途的不一致状态」，这其实不会发生。panic 被接在 Rust 这一侧，libghostty-vt 只看到回调返回了「不回答」，会照常解析完这块输入。会不一致的只有 Condr 在回调里改了一半的状态，所以回调必须足够简单。

## 六、建议

1. **回调只做拷贝。** 每个终端的 userdata 里放一个事件 `Vec`。终端本身已经在终端锁后面，不需要再加锁。PTY 回写、OSC 7501、OSC 777、标题、cwd、响铃、剪贴板写入和 prompt 标记都只 push 一个拥有所有权的值。DA1、尺寸、XTVERSION 这类要返回值的回调，从 userdata 里的普通值取答案；尺寸在 resize 时（已经持有终端锁）写进去，不要在回调里去锁 `size: Arc<Mutex<TerminalSize>>` 再 `expect`。Agent 通知、cwd 和 PTY 回写都在 `vt_write` 返回后按顺序处理；今天的 `pending_replies` 本来就是解析完再 flush 的。头文件也要求回调「not block for too long」。这是 herdr、gpui-cn 和 Paneflow 的共同做法。
2. **一个 helper 包住所有回调体。** 用 `catch_unwind(AssertUnwindSafe(..))`；panic 时给 userdata 里的计数器加一，返回这个回调的默认值：write_pty 丢弃字节，DA1、尺寸、XTVERSION 返回「不回答」，其余什么也不做。不重抛。日志 hook 已经记了带 backtrace 的 error；`vt_write` 返回后如果计数器不为零，再记一条带 Pane ID 的 warn，因为 hook 不知道是哪个 Pane。代价举例：DA1 没答，Claude Code 和 Pi 的 OSC 7501 探测要等到它们自己超时；一条 OSC 7501 报告丢了，Agent 状态要等下一条报告才更新。
   ```rust
   unsafe fn guarded<T>(state: *mut CallbackState, fallback: T, f: impl FnOnce(&mut CallbackState) -> T) -> T {
       let Some(state) = (unsafe { state.as_mut() }) else { return fallback };
       match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(&mut *state))) {
           Ok(value) => value,
           Err(_) => { state.panics += 1; fallback }
       }
   }
   ```
3. **trampoline 保持 `extern "C"`**（bindgen 默认生成的就是），不要改成 `"C-unwind"`。helper 漏掉的 panic，比如负载 drop 时再 panic，仍然确定地 abort。
4. **不要在 profile 里加 `panic = "abort"`，也不要让 panic hook 自己 abort。** hook 在 catch 之前执行，它一 abort，第 2 条就白写了。
5. **一个测试。** 照 Paneflow 的做法，`#[cfg(test)]` 下让下一次回调强制 panic，在 `vt_write` 期间触发。断言 `vt_write` 正常返回、计数器为 1，之后的 DA1 查询仍然有应答。

不选 Paneflow 的「让这个 Pane 失败」：为了一次丢失的响铃或标题杀掉用户的 Agent，代价太大；回调只做拷贝时，也没有留下需要保护的半截状态。如果同一个 bug 在每块输入上都触发，计数器会一直涨，日志会刷屏。那时再考虑升级成让 Pane 失败；现在不做。

**另一件事，不属于回调边界。** `vt_write` 返回后 reader 线程里的普通 Rust 代码（逐行比对 render state、光标稳定、处理通知）同样可能 panic，并引发第四节的连锁。可以有两个方向。(a) Pane 级隔离：reader 循环自己接住 panic，像 PTY EOF 一样发 `Exited`；Server 遇到被毒化的终端锁时把它当作终端已退出，而不是 `expect`。这接近 Paneflow host 的做法，也接近 herdr 的 `.ok()?`。(b) 让 Server 一 panic 就退出。但这和第 4 条冲突，回调里的 catch 也会跟着失效。这是独立的决定，建议另开 issue。

## 未确认

- Zig 默认 unwind 表的行为只在 macOS arm64 上用玩具库测过，没有在 Linux x86_64、aarch64 或 Windows MSVC 上测，也没有拿真正的 libghostty-vt 静态库测。结论不受影响：在 Rust 的规则下，unwind 穿过 Zig 帧本来就是未定义行为。
- abort 时 tracing non-blocking 文件层是否真的丢掉 panic 那条记录：推断，未实测。stderr 那条是同步写的。
- 第四节「毒化的终端锁连锁到整个 Server」是读代码得出的，没有实际让 reader 线程 panic 来验证。
- JetBrains 的 `TerminalListener.onRespondToHost` 怎么实现，以及别处有没有对 upcall 的兜底：只看了绑定和这一个文件。
- libghostty-vt 的 Zig 代码在回调期间持有哪些需要 `defer` 释放的东西，没有逐一审查。不让 unwind 穿过 Zig 帧，这个问题就不存在。
