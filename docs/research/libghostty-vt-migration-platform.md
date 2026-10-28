# libghostty-vt 迁移：平台前提与 VT 核心之外的新能力

调查日期：2026-10-28，基于 Condr 提交 `afcd14a`、ghostty-org/ghostty 提交 `46e9dc9a`（2026-10-27，下称 main）。libghostty-vt 的头文件引用都指 main 的 `include/ghostty/vt/` 目录，写成 `头文件:行`。Rust 绑定看了 crates.io 上的 `libghostty-vt` / `libghostty-vt-sys` 0.2.2，以及 github.com/Uzaaft/libghostty-rs 的 `master`（`9ede35b8`，2026-10-24）。输入编码、OSC、网格、视图、选区和搜索由另外两路调研负责，本文只在需要时提到。

结论：能做，但不是换个依赖那么简单。crates.io 上的 0.2.2 落后 main 1877 个提交，没有 snapshot、search，也没有内存统计，实际只能依赖 libghostty-rs 尚未发版的 `master`；三台开发机和全部 CI 都要装 Zig 0.17，或者像 Paneflow 那样预编译静态库并附校验和。绑定的类型全是 `!Send`，Condr 要么写一个局部 `#[allow(unsafe_code)]` 的 Send 包装，要么改成每个 Pane 一个 VT 线程。VT 核心之外最值得要的是三件：scrollback 按字节限额、空闲时压缩、按终端统计内存。其次是 formatter 能把软换行接回去。终端快照可以让 Server 重启后保留屏幕历史，但要修订 ADR 0005。kitty graphics 对 agent CLI 价值低，却要改四层，建议往后放。

## 构建与绑定

### 绑定钉的是哪个 ghostty

| 来源 | 钉住的 ghostty 提交 | 距 main | Zig |
| --- | --- | --- | --- |
| crates.io `libghostty-vt-sys` 0.2.2（2026-09-28 发布） | `a887df42`（2026-07-11）。依据：[build.rs@v0.2.2:7](https://github.com/Uzaaft/libghostty-rs/blob/v0.2.2/crates/libghostty-vt-sys/build.rs#L7)，已下载发布包核对，一致 | 落后 1877 个提交 | 0.15.x（README@v0.2.2:58；ghostty `build.zig.zon`@a887:6 要求 `minimum_zig_version = "0.15.2"`） |
| libghostty-rs `master` `9ede35b8` | `521cf2c5`（2026-10-21，master 的 build.rs:7） | 落后 92 个提交 | 0.17.x（README@master:54；ghostty `build.zig.zon`@46e9:7 要求 `0.17.0`） |

0.2.2 钉的版本和 main 的 C API 差别很大。以下是导出函数名的比对：a887 共 147 个，main 172 个。

- a887 没有 `snapshot.h`、`search.h`、`io.h`。main 新增的函数包括 `ghostty_snapshot_*`（12 个）、`ghostty_search_*`（8 个）、`ghostty_terminal_vt_write_until`、`ghostty_terminal_continuation_*`、`ghostty_terminal_paste`，以及 render state 的 image iterator（5 个）和 `ghostty_kitty_graphics_animation_tick`。
- `ghostty_terminal_new` 改了签名。a887 接受一个 `GhosttyTerminalOptions` 结构，scrollback 只能在构造时给一个行数 `max_scrollback`（a887 `terminal.h:180-193,1213-1215`）。main 改为 `(allocator, &terminal, cols, rows)`，限额在构造后用 `ghostty_terminal_set` 设置（`terminal.h:3231-3234`）。
- main 比 a887 多出 21 个 `GHOSTTY_TERMINAL_OPT_*`，其中有 `SCROLLBACK_MAX_BYTES/LINES`、`PROGRAM_STATUS`（OSC 7501）、`RESIZE_PULL_SCROLLBACK`、`UNKNOWN_SEQUENCE`、`RENDER_HOLD`；另多出 10 个 `GHOSTTY_TERMINAL_DATA_*`，其中有 `MEMORY_USAGE`、`VT_GROUND`、`VT_PROCESSING_ERROR`。
- 521cf2c5 到 main 之间，`terminal.h`、`snapshot.h`、`kitty_graphics.h` 里的导出函数只差一处改名：`vt_write_until_ground` 改成了 `vt_write_until`。

### 安全包装覆盖情况

| 能力 | 0.2.2 | master |
| --- | --- | --- |
| selection（含 gesture） | 有（`selection.rs`、`selection/gesture.rs`） | 有 |
| search | 无（C API 也没有） | 有（`search.rs`，963 行） |
| snapshot | 无（C API 也没有） | 有（`snapshot.rs`：encode/decode/ready/next）。解码器只包装了 continuation 两个选项，`COMPRESS_HISTORY` 和 `KITTY_IMAGE_STORAGE_LIMIT`（`snapshot.h:187-236`）没有包装 |
| kitty graphics | 有，但 PNG 解码器存在 `thread_local!` 里（`kitty/graphics.rs@v0.2.2:815-817`），只在注册它的那个线程有效 | 有，加了动画 tick；解码器改成全局 `Mutex`，`set_png_decoder` 变成 `unsafe fn`（`kitty/graphics.rs@master:1439-1454`） |
| render | 有，但没有 image iterator | 有（`render.rs` 中 image iterator 出现 33 处） |
| key / mouse encoder | 有（`key.rs`、`mouse.rs`） | 有 |
| formatter | 有（`fmt.rs`） | 有 |
| 压缩 / scrollback 限额 / 内存统计 | `compress`、`compression_activity` 有；限额只能在构造时给 | 另有 `set_scrollback_max_bytes/lines`。`MEMORY_USAGE` 两个版本都没包装，要直接调 ffi |

### 构建要求

- **会联网，而且需要 `git`。** build.rs 默认用 `git clone --filter=blob:none` 拉 github.com/ghostty-org/ghostty 并 checkout 钉住的提交（`build.rs@v0.2.2:356-397`）。之后 `zig build` 还要从 `deps.files.ghostty.org` 下载 Zig 包：lib-vt 用到 uucode、highway、wuffs，main 还有不 lazy 的 zigimg（`build.zig.zon`@46e9:16-75、`pkg/highway/build.zig.zon:9`、`pkg/wuffs/build.zig.zon:10-17`）。另外还要从 codeberg 拉 arocc（发布的源码包里有 `build.zig.zon.txt` 清单）。
- **可以离线。** 设 `GHOSTTY_SOURCE_DIR` 用本地源码（`build.rs@v0.2.2:96-99,119-130`），再设 `GHOSTTY_ZIG_SYSTEM_DIR`，build.rs 就给 zig 传 `--system`，禁止下载（`build.rs@v0.2.2:163-182`）。预取步骤见 ghostty `PACKAGING.md:71-106`。libghostty-vt-sys 没有「直接链接一个预编译 archive」的开关，最接近的是 `pkg-config` feature，它会优先找已安装的库（`build.rs@v0.2.2:101-107,262-312`）。Paneflow 的做法是自己写 -sys crate，在仓库里钉住 commit、头文件、绑定、Zig 版本和 archive 校验和，`build.rs` 从不下载源码（[Paneflow Linux 文](https://paneflow.dev/blog/libghostty-linux)）。
- **优化等级由 Cargo profile 决定。** Cargo 的 `DEBUG=true` 对应 Zig `Debug`，`opt-level` 为 s/z 对应 `ReleaseSmall`，其余是 `ReleaseFast`；可以用 `LIBGHOSTTY_VT_SYS_OPTIMIZE` 覆盖（`build.rs@v0.2.2:324-354`）。Condr 的 dev profile 是 `debug = "limited"`（`Cargo.toml:102`），所以 `cargo run -p condr-gui` 会链接 Debug 版的 VT。AGENTS.md:98 要求 dev 帧率可用，并保留 `alacritty_terminal = { opt-level = 3 }`（`Cargo.toml:105`）。替代办法是在新建的 `.cargo/config.toml` 里用 `[env]` 设 `LIBGHOSTTY_VT_SYS_OPTIMIZE = "ReleaseFast"`，仓库现在没有这个文件。
- **CPU 默认 `-Dcpu=baseline`**（`build.rs@v0.2.2:137-154`），可以直接发布。

### 各平台

| 平台 | 结论 | 出处 |
| --- | --- | --- |
| Windows x86_64 MSVC | 可行，有坑。静态库叫 `ghostty-vt-static.lib`，要用 `+verbatim` 链接，否则会链到 DLL 的导入库（`build.rs@v0.2.2:9-13,235-250`）。ghostty 构建静态库时对 MSVC 关掉 stack protector、不带 ubsan 运行时、链接 ntdll 和 kernel32（`GhosttyLibVt.zig:255-271`）。Zig 的 libc 和 MSVC CRT 是两个堆，libghostty-vt 分配的内存必须用 `ghostty_free` 释放（`allocator.h:61-65`）。0.2.2 克隆源码时没开 `core.longpaths`，Cargo 嵌套的 OUT_DIR 可能超过 MAX_PATH，master 修了。Paneflow 0.8.1 起在 Windows x64 MSVC 上静态链接这个库 | [Paneflow Windows 文](https://paneflow.dev/blog/libghostty-windows) |
| Linux aarch64（无头开发机，以及 `ubuntu-22.04-arm` 打包） | 可行。target 映射里有 `aarch64-unknown-linux-gnu`（`build.rs@v0.2.2:421`），本机构建不传 `-Dtarget`（`:184-189`）。Paneflow 发布的 Linux 包里有 x86_64 和 ARM64 两种静态 archive | Paneflow Linux 文 |
| macOS | Condr 只发 Apple Silicon 包（`build.yml:123-126`），不需要通用二进制。真要做，ghostty tip 发布的 xcframework 里已有 `macos-arm64_x86_64` 通用 archive（实测，见「体积与依赖」）。Zig 本机构建时最低系统版本取什么、和 Rust 默认值是否一致，未确认 | — |
| iOS / Android（Companion） | 不受影响，前提是 libghostty-vt 放在 `runtime` feature 下（`crates/condr-core/Cargo.toml:15-28`）。`condr-client` 和 `condr-mobile` 都以 `default-features = false` 依赖 condr-core（`crates/condr-client/Cargo.toml:15`、`crates/condr-mobile/Cargo.toml:31`），CI 的 mobile 任务也是 `--no-default-features`（`ci.yml:102`） | — |
| 在 Mac 上交叉检查 Windows/Linux（memory 里那套 stub cc/ar 的流程） | libghostty-vt-sys 的 build.rs 直接调 zig，不走 `cc`，stub 不起作用。Zig 从 macOS 交叉编到 `x86_64-windows-msvc` 时，在开 SIMD（会链接 libc）的情况下缺 MSVC CRT，大概率失败，未验证。可以绕过：设 `DOCS_RS=1`，build.rs 会直接返回，不构建也不链接（`build.rs@v0.2.2:71-76`），clippy 只做类型检查，够用。未验证，而且这个变量也会影响其他 crate 的 build.rs | — |

### CI、打包和安装脚本要改的地方

- `ci.yml` 的 test 任务（`:43-73`）：三个 OS 都要装 Zig 0.17，否则 `cargo clippy --workspace` 跑 build.rs 时就会失败。另外要在 `[env]` 里设 ReleaseFast，否则测试跑的是 Debug 版 VT。runner 要能访问 github.com、deps.files.ghostty.org、codeberg.org，或者走 `GHOSTTY_SOURCE_DIR` 加 `GHOSTTY_ZIG_SYSTEM_DIR` 的离线路径。`Swatinem/rust-cache` 只缓存 Cargo 的东西，Zig 全局缓存默认在 OUT_DIR 外面，未确认会不会被缓存住。mobile 任务不用改（`:102`）。
- `build.yml` 的 linux/windows/macos 三个任务（`:53-54`、`:97-98`、`:139-140`）：同样要装 Zig，或者改成下载预编译的静态库并校验 SHA256（Paneflow 的做法）。打包检查要加一条「二进制没有动态依赖 libghostty-vt」，目前 `check-linux-package.sh:14-15` 只跑了 `readelf -h`。`nightly.yml:43` 和 `release.yml` 都复用 build.yml，跟着改就行。
- 预编译方案要新开一个 workflow：用 Zig 为 4 个目标（linux x86_64/aarch64 gnu、windows x86_64 msvc、macos arm64）出静态库，作为带 SHA256 的 release 资产发布。Condr 再写一个很薄的 build.rs，或者自己的 -sys crate，只负责下载、校验、链接。
- `script/install-condr.sh` / `.ps1` 不用改：它们下载的是打好的包，并校验 `SHA256SUMS`（`install-condr.sh:62-75`、`install-condr.ps1:52-58`）。静态链接不会多出要随包分发的文件。
- 许可证：`script/generate-licenses` 只用 cargo-about 收集 Rust crate 的许可证。静态库里的原生代码要手工补声明：archive 成员有 `ghostty-vt-static_zcu.o`、`simdutf.o`、`highway_zcu.o`、`wuffs-v0.4.o`、`compiler_rt.o`，对应 Ghostty、simdutf、Highway、Wuffs、Zig compiler-rt 的声明。各家许可证文本未逐一核对。
- 文档：AGENTS.md:19（「Pure Rust, no FFI」）、:57-58（两台开发机都要装 Zig），以及 `Cargo.toml:105` 的 dev 优化配置。

## 线程模型

**Condr 现状。** 每个 Pane 的 `Term` 放在 `Arc<Mutex<Term<TerminalEventProxy>>>` 里（`crates/condr-core/src/terminal/runtime.rs:10,163`、`pty_io.rs:3`），会有好几个线程碰它：

1. **reader 线程** `condr-pty-reader`（`runtime.rs:239-254,268-270`）：持锁解析每块最多 16 KiB 的输出，放锁后再把应答写回 PTY（`pty_io.rs:40,73-93`）。alacritty 的事件回调只往队列里推应答，不在持锁时回写（`notices.rs:155-168`）。
2. **resizer 线程**（`runtime.rs:214-230`、`resize.rs:101-109`）。
3. **Server 的每 Pane 监视线程**（`crates/condr-server/src/server/terminal_monitor.rs:26,66`）：调 `view_source.take_frame()` / `view()`，持锁生成帧（`view_source.rs:26-28,62-64`）。
4. **每个 Client 的处理线程**（`server.rs:378`）：经由 runtime 的方法访问，包括 `submit`（`runtime.rs:280-285`）、`scroll`（`:581`）、`copy_range`（`:598`）、`recent_text`（`:520-523`，调用方 `client.rs:701`）、鼠标（`runtime/mouse.rs:6`）、`print_notice`（`:528`）。
5. **新建终端和 Bootstrap** 的路径（`terminal_state.rs:47-50`）。

**libghostty-vt C API 的约定。** 头文件没有要求某个对象只能在固定线程上用，只要求调用方自己串行化。

- 压缩：「not thread-safe with other operations on the same terminal. The caller must serialize it」（`terminal.h:3625-3627`）；snapshot encode 期间调用方要禁止其他修改（`snapshot.h:486-489`）。
- render state「allows the renderer to be safely multi-threaded (as long as a lock is held during the update call)」（`render.h:29-34`）。两阶段更新只有 begin 阶段需要持锁（`render.h:43-52,1140-1150`）。search 的 tick 可以不持锁运行，文档说这就是 Ghostty 自己在后台线程跑搜索的方式（`search.h:92-110`）。这说明「跨线程、持锁访问」正是设计里的用法。
- effect 回调在调用 `vt_write` 的线程上同步执行，不能重入 `vt_write`（`terminal.h:78-84,2090-2091`）。从 terminal 借出的句柄在下一次修改后全部失效，包括 kitty 句柄（`kitty_graphics.h:215-220`）和 grid ref（`terminal.h:3765`）。
- `ghostty_sys_set` 是进程全局的，应在启动时设好（`sys.h:21-23,252-258`）；日志回调必须能在任意线程调用（`sys.h:100-101`）。

**Rust 绑定。** 所有类型都是 `!Send` 和 `!Sync`。理由是「the C API is allowed to use thread-local state」，作者建议另开一个线程，用 channel 通信（`lib.rs@master:50-71`）。`Terminal` 内部持有裸指针（`terminal.rs@master:227-238`）。回调只约束 `'cb` 生命周期，不要求 `Send`（`terminal.rs@master:3069-3073`），回调里 panic 会让进程 abort（`terminal.rs@master:2239`）。

**后果。** `Arc<Mutex<libghostty_vt::Terminal>>` 编译不过，因为 `Mutex<T>` 要 `Sync` 就得 `T: Send`。有两种办法：

- **A. 局部 Send 包装（建议）。** 把 `Terminal`、`RenderState` 和各种 iterator 放进同一个结构体，在一个文件里写 `#[allow(unsafe_code)] unsafe impl Send`。仓库里已有先例，都带理由注释，见 `crates/condr-server/src/main.rs:712`、`noise/identity.rs:619`、`server/local.rs:248`、`crates/condr-gui/src/app/login_environment.rs:20`；工作区默认 `unsafe_code = "deny"`（`Cargo.toml:129`）。外面的锁结构保持不变。这个包装要自己守住几条规则：装进去的回调必须是 `Send + 'static`（绑定不检查）；借出的行、cell、kitty 句柄和 grid ref 不能活过锁的守卫，这与 Condr 现在只向外交付 owned `TerminalView` 的做法一致；PNG 解码器和日志在创建第一个 Terminal 之前设好。0.2.2 的 thread-local 解码器在这种模型下会悄悄失效，必须用 master。风险在于 libghostty-vt 内部有没有线程局部状态：头文件没有保证没有，未确认。Ghostty 自己在 IO 线程和渲染线程之间共用一个加锁的 terminal，说明这种用法是被支持的。
- **B. 每个 Pane 一个 VT 线程。** 线程独占 terminal，PTY 字节和查询都通过 channel 传进去。这是安全 Rust，Paneflow 也这样做：据 Devpost 上的介绍，他们把每个 Ghostty 实例限制在专用线程上，只把自包含的 Rust 快照交给 GPUI，未直接核对。代价是上面第 3 到第 5 类同步调用（`view`、`take_frame`、`submit` 读模式、鼠标、`copy_range`）都要改成请求和应答两条消息；reader 的每块输出多一次线程跳转；监视线程「放锁后再生成帧」的结构要重写。

## 新能力清单

| 能力 | libghostty-vt API | Condr 现状 | 要动的层 | 价值 |
| --- | --- | --- | --- | --- |
| scrollback 按字节限额、空闲压缩、每终端内存统计 | `OPT_SCROLLBACK_MAX_BYTES/LINES`，以页为单位裁剪，一页约 400KB（`terminal.h:2459-2502`）；`ghostty_terminal_compression_activity`、`ghostty_terminal_compress`（增量或全量），由调用方驱动，库里没有定时器（`terminal.h:45-55,262-289,3606-3644`）；`DATA_MEMORY_USAGE` 分屏报告常驻、压缩、图片字节（`terminal.h:663-768,3212`），有的平台不支持压缩（`:711-717`），macOS 归还内存较慢（`:694-697`） | 固定 10,000 行：`terminal_config()` 用了 `..Config::default()`（`runtime.rs:732-743`），alacritty 默认 `scrolling_history: 10000`（`alacritty_terminal-0.26.0/src/term/mod.rs:359`）。每个 cell 24 字节（`term/cell.rs:303`），一个 200 列的 Pane 填满约 48 MB，这是估算，没实测。没有配置项，也没有内存统计 | Server/core：在每 Pane 监视线程（`terminal_monitor.rs:26`）里按 activity token 安排空闲压缩，设字节上限。可选：把上限写进 `config.toml`（走 `update_config_values`），在 Settings 或 CLI 里显示每个 Pane 的内存，这需要动协议和 GUI | **高。** Server 常驻，同时挂着许多 agent Pane，Codex 和 Claude 又大量刷屏，可以按内存预算统一管理 |
| 终端快照（Server 重启后恢复屏幕和 scrollback） | `ghostty_snapshot_encode*`、`ghostty_snapshot_decoder_*`（`snapshot.h:484-841`）。格式是带 CRC 的记录流，READY 之前的部分足以渲染，历史页随后从新到旧排列，可以先显示再补历史（`snapshot.h:27-30,79-111`）；kitty 图片也在里面，可选（`:92-105,124-134`）；可在解码时压缩历史（`:187-218`）。**格式版本 1 仍在开发，不保证二进制兼容**（`:136-137`）。如果编码时解析器不在 ground 状态，必须提前开启 continuation 跟踪（`:490-493`） | ADR 0005 明确不保存终端内容：「A Snapshot never contains terminal cells or scrollback」，替身 Server 的终端状态是全新的，「Condr excludes terminal history」（`docs/adr/0005-…md` 第 1、5、8 段）。只靠原生 resume 恢复对话 | Server：关闭流程里 PTY 排空后（ADR 0005 已有这一步），按 Pane 写独立文件，不能塞进 SessionSnapshot，它的持久预算是 2 MiB。恢复时先解码，只保留主屏历史，丢掉 alt screen（Claude Code 的完整 UI 跑在 alt screen 上），复位鼠标和键盘等模式，再启动新 shell；具体该写哪些序列，未确认（RIS 会清掉 scrollback，见 `terminal.h:3251-3252`）。解码失败就退回现在的行为。协议、GUI、Companion 都不用动。另外要写一个新 ADR 修订 0005 | **中。** 计划内重启（升级、`server restart`）后能看到之前的构建日志和 shell 输出。agent 的 `--resume` 本来就会重画对话，这部分重叠。如果要应对崩溃，就得定期编码，而编码期间要持锁。还有一个新问题：终端内容（可能包括密钥）会落盘 |
| formatter | `ghostty_formatter_terminal_new` / `format_*`，支持 PLAIN、VT、HTML（`types.h:256-264`），可选 `unwrap` 接回软换行、`trim`、只格式化选区，可附带模式、调色板、光标等（`formatter.h:36-123`）；writer 回调不能回调 terminal（`formatter.h:147-151`） | `condr pane read` 走自己写的 `recent_text`，逐个物理行用 `\n` 拼起来，**不接软换行**（`probes.rs:226-271`，调用方 `client.rs:701`，协议说明在 `protocol/messages.rs:51`，CLI 在 `cli/pane.rs:78`） | core：换成 formatter 的 PLAIN 加 `unwrap`，行为会变，测试要跟着改。可选：协议的读取请求加一个 format 字段，CLI 加 `--format vt\|html` | **中。** 一个 agent 读另一个 Pane 的输出时，被窗口宽度折断的长行能接回来。VT 和 HTML 格式可以用来导出 |
| kitty graphics | 存储上限一设就开（`terminal.h:2330-2340`，`kitty_graphics.h:41`）；file、temp-file、shm 三种读取方式分别开关（`terminal.h:2342-2371`）；PNG 靠进程全局的解码回调（`sys.h:118-173,204-213`）；render state 给出图片列表和位置（`render.h:205-340`），像素已解码（`kitty_graphics.h:69-71`），generation 永不复用、可直接当缓存键（`:105-113`），动画由调用方 tick（`:116-150`）；`resize` 需要 cell 的像素尺寸（`terminal.h:3265-3293`） | 不支持。alacritty 忽略 APC，Condr 只测了「被 ^C 截断的图片不破坏 VT」（`tests/osc.rs:136`）。`TerminalSize` 和协议已经带 cell 像素尺寸（`view.rs:27-32`，`terminal.proto:7-12`）。GUI 预览已经能把图片解码成 `RenderImage`（`app/preview.rs:57-105`），GPUI 有 `paint_image`（`gpui-pre-0.3.9/src/window.rs:5053`） | **四层都要动。** Server：设上限，PNG 解码复用锁文件里已有的 png 0.18；默认关掉 file 和 shm 两种读取方式。协议：图片按 (id, generation) 作为资源，分块发送，单帧上限 2 MiB，可参照 BootstrapRecord 的分块；每帧只带位置信息。像素不能放进可丢弃的视觉槽（ADR 0004），需要按客户端缓存，缺了再请求。GUI：在终端元素里 `paint_image`。Companion：iOS 现在把屏幕重新编码成 VT 字节喂给 SwiftTerm（`mobile/ios/Condr/Terminal/ScreenEncoder.swift:1-6`），要么叠加 UIImage，要么转成 SwiftTerm 能识别的图片序列（SwiftTerm 是否支持 kitty，未确认）；Android 在 `TerminalCanvas.kt` 里 drawBitmap；`condr-client` 和 uniffi 要新增类型 | **低。** Claude Code 和 Codex 的 TUI 是否输出 kitty 图片，未确认。受益的是用户在 Pane 里跑的图片工具（`kitten icat`、yazi 预览、chafa 等）。建议往后放 |
| 自定义 allocator | `GhosttyAllocator` vtable，仿照 Zig 的接口；传 NULL 就用 libc malloc（`allocator.h:29-33,73-106`） | 生产代码没有自定义分配器，只有测试里用了 `#[global_allocator]`（`crates/condr-core/tests/decode_memory.rs:44`） | 不做 | **低。** 页内存看上去直接走 mmap/madvise（静态库的未定义符号里有这两个），未确认会不会经过 allocator。按终端做预算应该用上面的 `MEMORY_USAGE`。Windows 上唯一的要求是按 `allocator.h:61-65` 用 `ghostty_free` 释放，绑定已经处理 |
| 日志接到 `tracing` | `GHOSTTY_SYS_OPT_LOG`，回调必须线程安全，Release 构建不发 debug 级（`sys.h:90-116,215-233`）；绑定有 `tracing` feature | ADR 0019 规定 subscriber 在 `condr-server` 的 `logging` 模块里；那里的注释已经列出经由 `log` 输出的依赖（`logging.rs:4`） | Server：启动时设一次 | 低到中，主要用于排查 |
| 构建信息 | `ghostty_build_info`：版本串、commit、优化模式、SIMD、是否带 kitty（`build_info.h:51-142`） | ADR 0027 的 build identity 不含 VT 版本 | core：加到 `build_identity()` 和 About 页 | 低，用于支持排障 |

不属于输入编码、OSC、网格视图，但在头文件里看到的其他能力：

- `OPT_RESIZE_PULL_SCROLLBACK`：设为 false 后，resize 不会把 scrollback 拉回屏幕，文档明说是为 Windows ConPTY 准备的（`terminal.h:2660-2684`）。和网格调研有重叠，提醒对方在 Windows 上默认关掉。
- 安全相关的默认值：DECRQCRA 校验和默认关，因为程序可以借它逐格读出屏幕上其他程序的输出（`terminal.h:2714-2729`），应保持关闭。`OPT_TERMINFO_NAME` 用来回答 XTGETTCAP 的 TN 查询（`:2612-2625`）。Glyph Protocol 有开关（`:2425-2435`），但头文件里找不到读取已注册字形的 API，嵌入方画不出来，默认值也未确认，建议显式关掉，免得程序以为终端支持。
- `ghostty_terminal_vt_write_until`：用同一个解析器找到 APC、DCS、OSC 的确切边界（`terminal.h:3346-3470`）。能否替代 Condr 在 VT 之前单独扫一遍 OSC（`pty_io.rs:45`），由 OSC 那一路评估。
- Unicode 宽度：`ghostty_unicode_codepoint_width` / `grapheme_width`，纯函数、线程安全（`unicode.h:64-69,137-144`）。`condr-gui` 依赖带 runtime 的 condr-core（`crates/condr-gui/Cargo.toml:26`），本来就会链接 libghostty-vt，所以 IME 预编辑可以和 Server 用同一张宽度表。价值低。
- tmux control mode 只有一个构建标志（`build_info.h:69-74`），C API 里没有接口，用不了。WASM 构建（`wasm.h:10-19`）和 Condr 不用 webview 的方针无关。

## 体积与依赖

**静态库大小。** 以下是实测，用的是 ghostty `tip` 发布中 `46e9dc9a` 构建的 `ghostty-vt.xcframework.zip`，优化模式未确认：

| 产物 | 大小 |
| --- | --- |
| `macos-arm64_x86_64/libghostty-vt.a`（通用 archive） | 26,555,248 B |
| 其中 arm64 切片（`lipo -thin`） | 12,608,032 B；13 个成员的 `size` 合计 __TEXT 2,076,345 B、__DATA 97,472 B |
| `ios-arm64/libghostty-vt-fat.a` | 12,498,288 B |
| 另见 `ghostty-vt.wasm` / `ghostty-vt-small.wasm` | 1,189,912 B / 798,603 B |

archive 里大部分是符号和调试段。链接进程序后，每个架构最多增加约 2.1 MB 的代码和数据，死代码剥离后的实际增量未确认。`condr` 和 `condr-gui` 都会带上（GUI 依赖 condr-core 和 condr-server，`crates/condr-gui/Cargo.toml:26,28`）。作为对照，现在 `condr.exe` 是 23.7 MB，`condr-gui.exe` 是 72.7 MB（`docs/research/release-binary-size.md:23`）。alacritty_terminal 自己在二进制里占多少，未确认。

**运行时依赖。** 不带 libc++。ghostty 用 no-libcxx 模式（`HWY_NO_LIBCXX` / `SIMDUTF_NO_LIBCXX`）构建自带的 C++ 依赖，只有改用系统 simdutf 时才需要 `-lc++`（`GhosttyZig.zig:119-127`、`GhosttyLibVt.zig:503-509`）。实测 arm64 切片的未定义符号里没有任何 `__cxa_*` 或 `std::`，只有 libSystem 的函数：malloc/free、mmap/munmap/madvise、shm_open、arc4random_buf、sysctlbyname 等。静态库还打包了 Zig 的 compiler_rt（`GhosttyLibVt.zig:245`）。Windows 上额外链接 ntdll 和 kernel32（`:268-270`），两者本来就是系统库。Linux 上实际依赖哪些 libc 符号未逐一列出，未确认。

**去掉 `alacritty_terminal` 后少掉的 crate。** 用 `cargo tree -i alacritty_terminal -e normal` 确认它只被 condr-core 引入；用 `cargo tree -p alacritty_terminal -e normal,build --target all` 列出它的 55 个依赖；再用 `cargo metadata` 在全部平台上算一遍工作区的可达集（normal、build，以及工作区成员的 dev 依赖），只有下面 7 个随它一起消失：

`alacritty_terminal 0.26.0`、`vte 0.15.0`、`cursor-icon 1.2.0`、`home 0.5.12`、`miow 0.6.1`、`rustix-openpty 0.2.0`、`signal-hook 0.4.4`。

regex-automata、parking_lot、polling、rustix、unicode-width、windows-sys 等还有别的 crate 在用。新增的 Rust crate 有 4 个：`libghostty-vt`、`libghostty-vt-sys`、`int-enum`（proc macro）、`proc-macro2-diagnostics`（int-enum 关了它的默认 feature，所以不会带上 yansi）；`syn 2`、`version_check`、`png 0.18.1` 已经在 `Cargo.lock` 里。也就是 Rust crate 少 7 个、多 4 个，另外多了一个原生静态库和 Zig 构建工具链。直接引用 alacritty 的源码只有 `crates/condr-core/src/terminal/prelude.rs:35-44`、`notices.rs:2`、`view/snapshot.rs:261,335`，`crates/condr-gui/assets/licenses.md` 里的条目需要重新生成。

## 风险与未决

1. **API 不稳定。** libghostty-rs 的 README 说库还在 1.0 之前，提交进仓库的绑定跟着钉住的版本走；snapshot 格式不保证兼容（`snapshot.h:136-137`）。Paneflow 也说 C API 的签名还在变，每次升级都要一起审 archive 和绑定。而 Condr 只能依赖 libghostty-rs 尚未发版的 master，或者自己 fork 一份。
2. **崩溃波及面。** 一个 Server 承载所有 Pane。Zig 的 ReleaseFast 不做安全检查，出 bug 就是未定义行为；绑定回调里 panic 会 abort 整个进程（`terminal.rs@master:2239`）。用 alacritty 时这些都是安全 Rust。需要模糊测试，以及同输入下和 alacritty 的差分测试（Paneflow 两样都做了）。
3. **Send 包装是否成立。** 它依赖「libghostty-vt 内部没有线程局部状态」，头文件没有保证这一点，未确认。可以考虑在 Linux CI 上用 ThreadSanitizer 跑一遍多 Pane 的压力测试。
4. **工具链和网络。** 三台开发机加全部 CI 都要装 Zig 0.17 或接入预编译 archive；构建时要访问 github、deps.files.ghostty.org、codeberg。Zig 构建要多花多少时间，未确认。
5. **dev 帧率。** 不覆盖优化等级的话，dev 构建会用 Zig Debug，违反 AGENTS.md:98。
6. **从 Mac 交叉检查。** windows-msvc 目标很可能卡在 Zig 这一步，`DOCS_RS=1` 能否绕过还没验证。
7. **平台细节。** macOS 静态库的最低系统版本是否和 Rust 一致；Windows 上 0.2.2 克隆源码会碰到 MAX_PATH；compiler_rt 和 Rust 的 compiler_builtins 会不会有重复符号（Paneflow 和 libghostty-rs 都在用，风险低）。三项都未确认。
8. **新暴露面。** kitty 的 file 和 shm 读取方式会让终端读 Server 上的文件；终端快照会让终端内容落盘。两者都需要先有决定，再写 ADR，后者要修订 ADR 0005。
9. **许可证。** 静态库里的 Ghostty、simdutf、Highway、Wuffs、compiler-rt 不在 cargo-about 的覆盖范围内，要手工维护声明。
