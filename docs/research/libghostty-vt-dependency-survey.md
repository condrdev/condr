# libghostty-vt 在 Rust 项目里怎么依赖和构建：调查

调查日期：2026-10-28。libghostty-vt 是 ghostty-org/ghostty 里的 VT 库。下文的仓库链接都钉到调查当天读到的提交上。查不到的地方标「未确认」。

**结论。** Rust 项目最常见的做法是构建时用 Zig 从源码编 libghostty-vt：crates.io 上 `libghostty-vt` 的 15 个反向依赖，以及 wsp、microsoft/tui-test、gpui-cn、herdr，都是这样。公开走预编译 archive 的 Rust 项目只找到 3 个：Paneflow（覆盖 4 个目标，含 Windows MSVC，基础设施最重）、ghostty-vt-rs（0 star，没有 Windows 包）、xipeng-jin/zed 的分支（只有 Linux，分支 8 月起没再动）。非 Rust 的 JetBrains 也用预编译，发的是共享库。上游 ghostty 不提供 Linux 或 Windows 的预编译库，只为通过测试的 main 提交发布一个带签名的 lib-vt 源码包。

两条路的代价：

- **Zig 从源码编**：不用搭任何基础设施。代价是每台开发机和每个 CI job 都要装上游指定的那个 Zig 小版本（2026 年里已经从 0.15 换到 0.16，又换到 0.17）。首次构建要联网，大约 50 秒。libghostty-rs 在 dev profile 下默认编 Debug 版，会让 `vt_write` 慢约 3000 倍。
- **预编译**：开发机不用装 Zig，dev 构建也能用上 ReleaseFast。代价是要自己维护三样东西：产出 archive 的 workflow、发布位置和校验和、绑定与 archive 的同步。Windows MSVC 的 archive 只能在 Windows runner 上编，因为上游从别的平台交叉编译 msvc 还不通。

## 总表

| 项目 | 绑定来源 | libghostty-vt 获取方式 | 钉版本方式 | 平台 | Zig 是否进开发机 | 规模 |
| --- | --- | --- | --- | --- | --- | --- |
| [libghostty-rs](#libghostty-rsuzaaft) | 自身（bindgen 生成后提交） | build.rs 用 `git clone` 拉 ghostty，再跑 `zig build`；也可走 `GHOSTTY_SOURCE_DIR` 或 `pkg-config` | build.rs 常量 `GHOSTTY_COMMIT`，flake.nix 的 input 与之对齐，Nix 检查二者一致 | macOS arm64、Linux x86_64/aarch64、Windows MSVC/GNU、iOS（只做 check） | 是，Zig 0.17.x | 399★，master 最近提交 2026-10-24，crates.io 累计 18.2 万下载 |
| [crates.io 反向依赖](#cratesio-反向依赖和-git-rev-使用者) | crates.io `libghostty-vt` ^0.2 | 同上 | Cargo.lock | 各自 | 是 | 15 个；最大的是 terminal-control（435★，7.2 万下载） |
| [wsp](#wspwsp-labswsp) | libghostty-rs，git rev | 脚本下载 codeload tarball 并校验 sha256，经 `GHOSTTY_SOURCE_DIR` 交给 Zig 0.16 | rev，加上带 commit 的目录名 | macOS（PR 里说的验证平台） | 是 | 13★ |
| [microsoft/tui-test](#cratesio-反向依赖和-git-rev-使用者) | libghostty-rs，git rev | Zig 从源码编；发布用 cargo-zigbuild | rev | Linux、macOS、Windows | 是 | 296★ |
| [gpui-cn](#gpui-cnprabirshresthagpui-cn) | 自己的 `ghostty-vt-sys`/`ghostty-vt`，改自 libghostty-rs 0.2.2，已发到 crates.io | build.rs 依次找 `GHOSTTY_SOURCE_DIR`、submodule、官方 lib-vt 源码 tarball（校验 sha256）、pkg-config；固定 ReleaseFast | `GHOSTTY.lock`（commit、url、sha256）、submodule、crate 版本里的 build metadata；有同步脚本和检查脚本 | CI 跑 macOS/Linux/Windows 测试和 iOS check；运行时只支持 macOS | 是，Zig 0.17 | 1★，2026-10-28 |
| [herdr](#herdrherdrdevherdr) | 自写 crate，提交 bindgen 生成的 `bindings.rs`，没有独立的 -sys crate | 仓库里 vendor 了 lib-vt 源码 dist（约 19.6 MB，1339 个文件），另带本地补丁；build.rs 调 Zig 0.16.0，默认 ReleaseFast | `vendor/libghostty-vt.vendor.json`，外加补丁台账 | Linux gnu/musl x86_64/aarch64、macOS x86_64/arm64、Windows x86_64 MSVC | 是 | 46240★，2026-10-26 |
| [Paneflow](#paneflowarthjeanpaneflow) | 自写 `paneflow-libghostty-sys`（只做校验，不下载）和安全包装 | **预编译静态 archive**：bump workflow 产出，发成 GitHub pre-release，带 SLSA 证明；脚本下载并按 manifest 校验 sha256 | `native/libghostty/manifest.toml`，记 commit、Zig 版本和各项 sha256；手动触发 bump workflow 开 PR | Linux x86_64/aarch64、macOS arm64、Windows x86_64 MSVC | 否 | 90★，GPL-3.0，2026-10-27 |
| [ghostty-vt-rs](#ghostty-vt-rsnklmilojevicghostty-vt-rs) | 自己的 `ghostty-vt-rs-sys`，改自 libghostty-rs | **预编译**：build.rs 用 `curl` 从自家 release 下载，按 `prebuilt.rs` 校验 sha256；其他目标退回 Zig 0.17 源码编 | build.rs 常量，加上 workflow 写入的 `prebuilt.rs` | 预编译覆盖 macOS arm64、Linux gnu/musl x86_64/aarch64、iOS、wasm32；Windows 走源码编 | 否（Windows 是） | 0★，crates.io 542 下载，2026-10-27 |
| [xipeng-jin/zed](#xipeng-jinzed-的-migrationlibghostty-分支) | 把 libghostty-rs 一次性 vendor 进来 | **预编译**：默认从独立仓库下载 tar.gz，按 pin 文件校验 sha256；保留源码编逃生口；所有路径都是 ReleaseFast | `ghostty_pin.toml`，记 commit、源码 fork、prebuilt 仓库、headers sha256、各目标 sha256 | 只有 Linux gnu/musl x86_64/aarch64 | 否 | fork 3★；分支最后提交 2026-07-22，v2 分支 2026-08-28 |
| [gpui-ghostty](#gpui-ghosttyxuanwo) | 自写的 Zig 包装（`lib.zig`）和手写头文件，不走 lib-vt 的 C API | submodule 引入 ghostty，Zig 0.14.1，ReleaseFast | submodule | Linux、macOS（CI） | 是 | 89★，实际开发停在 2026-01-08 |
| [JetBrains](#jetbrains非-rust参照) | 自写 Kotlin 绑定 | **预编译共享库**：内部 TeamCity 从 JetBrains/ghostty fork 的 `tip` 构建，传到 JetBrains 自家存储；从源码运行时按需下载 | `dependencies.properties` 里的版本号；未见 sha256 | Windows、macOS、Linux | 否 | IntelliJ 2026.3 起默认引擎 |
| [vtcode](#没有采用或已放弃的项目) | 运行时用 dlopen 加载 | 预编译共享库，加载失败就回退到旧引擎 | — | — | — | 892★；2026-06-07 已删除这套集成 |
| [Warp、tatou、zeus](#没有采用或已放弃的项目) | 未采用 | — | — | — | — | — |

## libghostty-rs（Uzaaft）

[github.com/Uzaaft/libghostty-rs](https://github.com/Uzaaft/libghostty-rs)，399★，MIT OR Apache-2.0（仓库里只有 MIT 的 LICENSE 文本，见 zed 分支 README 的说明）。读的是 master `9ede35b8`（2026-10-24）。

- **绑定**：`crates/libghostty-vt-sys/src/bindings.rs` 由自带的 `gen-bindings` 二进制（`bindgen-tool` feature）从头文件生成后提交，所以用户构建时不需要 libclang（[README:99-113](https://github.com/Uzaaft/libghostty-rs/blob/9ede35b8/README.md#L99-L113)）。安全包装在 `crates/libghostty-vt`。
- **libghostty-vt 的来源**：build.rs 依次尝试三种方式（[build.rs:100-116](https://github.com/Uzaaft/libghostty-rs/blob/9ede35b8/crates/libghostty-vt-sys/build.rs#L100-L116)）：
  1. 设了 `GHOSTTY_SOURCE_DIR` 就用这个本地源码；
  2. 开了 `pkg-config` feature 时找已安装的库；
  3. 默认在 `OUT_DIR` 里执行 `git clone --filter=blob:none --no-checkout` 再 checkout 钉住的提交（[:422-467](https://github.com/Uzaaft/libghostty-rs/blob/9ede35b8/crates/libghostty-vt-sys/build.rs#L422-L467)）。

  拿到源码后跑 `zig build -Demit-lib-vt=true -Dapp-runtime=none`（[:187-249](https://github.com/Uzaaft/libghostty-rs/blob/9ede35b8/crates/libghostty-vt-sys/build.rs#L187-L249)）。默认静态链接。设 `ZIG` 可以换 Zig 可执行文件，设 `GHOSTTY_ZIG_SYSTEM_DIR` 会给 zig 加 `--system`，禁止它下载包。
- **钉版本**：build.rs 第 7 行 `GHOSTTY_COMMIT = 521cf2c5…`。flake.nix 的 ghostty input 也钉同一个提交。Nix 的 `bindings-fresh` 检查两件事：两处的提交一致；用钉住的头文件重新生成的绑定与仓库里的相同（[flake.nix:200-240](https://github.com/Uzaaft/libghostty-rs/blob/9ede35b8/flake.nix#L200-L240)）。2026 年 3 月到 10 月，build.rs 里共有 16 次「update to <sha>」，平均两周一次；最近两次是 10-09 和 10-23。另有 `stack/ghostty-*` 分支给升级用的 stacked PR（[ci.yml:10](https://github.com/Uzaaft/libghostty-rs/blob/9ede35b8/.github/workflows/ci.yml#L10)）。crates.io 只发过 0.2.0（06-16）、0.2.1（07-18）和 0.2.2（09-28）。0.2.2 钉的 ghostty 已落后很多，见 [libghostty-vt-migration-platform.md](libghostty-vt-migration-platform.md)。
- **平台和 CI**：Linux 和 macOS 用 Nix，跑在 Namespace runner 上，Linux 有 x86_64 和 aarch64。CI 的 Nix 构建通过 `pkg-config` feature 链接 ghostty flake 构建好的库，不走 git clone（[flake.nix:133,145](https://github.com/Uzaaft/libghostty-rs/blob/9ede35b8/flake.nix#L133)）。Windows 用 `windows-latest` 加 `mlugg/setup-zig@v2`，版本 `0.17.0`，测 MSVC 和 GNU 两种 ABI，静态和动态链接都测（[windows-ci.yml:24-66](https://github.com/Uzaaft/libghostty-rs/blob/9ede35b8/.github/workflows/windows-ci.yml#L24-L66)）。交叉编译时把 Rust triple 映射成 Zig triple，再传 `-Dtarget`（[:565-582](https://github.com/Uzaaft/libghostty-rs/blob/9ede35b8/crates/libghostty-vt-sys/build.rs#L565-L582)）。
- **dev 优化等级**：`DEBUG=true` 时用 Debug，`OPT_LEVEL` 为 s 或 z 时用 ReleaseSmall，其他情况用 ReleaseFast。可以用 `LIBGHOSTTY_VT_SYS_OPTIMIZE` 覆盖（[:390-420](https://github.com/Uzaaft/libghostty-rs/blob/9ede35b8/crates/libghostty-vt-sys/build.rs#L390-L420)）。所以 dev 构建默认拿到的是 Debug 版的 libghostty-vt。
- **公开记录的坑**（都写在代码注释里）：
  - Windows 上 Zig 把静态库命名为 `ghostty-vt-static.lib`。直接写 `static=ghostty-vt` 会在 MSVC 上链到同名 DLL 的导入库，构建能过，但运行时要 DLL；所以要用 `static:+verbatim=…` 链接（[:301-316](https://github.com/Uzaaft/libghostty-rs/blob/9ede35b8/crates/libghostty-vt-sys/build.rs#L301-L316)）。
  - Cargo 嵌套的 `OUT_DIR` 加上 ghostty 的 fuzz 语料文件名，路径会超过 Windows 的 MAX_PATH，所以 clone 时要设 `core.longpaths=true`（[:444-450](https://github.com/Uzaaft/libghostty-rs/blob/9ede35b8/crates/libghostty-vt-sys/build.rs#L444-L450)）。
  - 不给 `-Dcpu` 时 Zig 会生成本机特有的指令，分发出去的二进制在旧 CPU 上会报非法指令；2026-08-14 起默认改为 `baseline`（[:195-204](https://github.com/Uzaaft/libghostty-rs/blob/9ede35b8/crates/libghostty-vt-sys/build.rs#L195-L204)）。
  - Zig 0.17 去掉了 `--global-cache-dir`，只能用环境变量 `ZIG_GLOBAL_CACHE_DIR` 改全局缓存位置（[:221-223](https://github.com/Uzaaft/libghostty-rs/blob/9ede35b8/crates/libghostty-vt-sys/build.rs#L221-L223)）。
  - 检测到 `DOCS_RS` 时跳过原生构建，靠提交的绑定生成文档（[:71-76](https://github.com/Uzaaft/libghostty-rs/blob/9ede35b8/crates/libghostty-vt-sys/build.rs#L71-L76)）。

## crates.io 反向依赖和 git rev 使用者

截至调查日，`libghostty-vt` 有 15 个反向依赖（[crates.io API](https://crates.io/api/v1/crates/libghostty-vt/reverse_dependencies)）：terminal-control、phantom-daemon、betamax、tui-test-rs、betamax-core、don、agentmux、ratatui-ghostty、quickgui、egui_tty、phux-protocol、tuiscotti-tui-test、clawsh-tui、scosh、hyprterm。`libghostty-vt-sys` 只有 `libghostty-vt` 一个反向依赖，没有项目直接依赖 sys 层。它们都继承了 libghostty-rs 的 Zig 源码编。下面是看过 CI 的几个：

- **terminal-control**（anomalyco，435★，7.2 万下载）：依赖 crates.io 0.2.1。CI 装 Zig 0.15.2（[ci.yml:16-18](https://github.com/anomalyco/terminal-control/blob/c1d4f95e4f/.github/workflows/ci.yml#L16-L18)）。0.2.1 还不能指定 CPU，他们就在 PATH 前面放一个 zig 包装脚本，强制 baseline；构建前先 `cargo clean --release -p libghostty-vt-sys`，理由是「PATH 不是依赖构建脚本跟踪的输入」。之后用 `qemu-x86_64 -cpu qemu64` 验证产物能在旧 CPU 上跑（[build-portable.sh](https://github.com/anomalyco/terminal-control/blob/c1d4f95e4f/scripts/build-portable.sh)，[ci.yml:22-28](https://github.com/anomalyco/terminal-control/blob/c1d4f95e4f/.github/workflows/ci.yml#L22-L28)）。
- **microsoft/tui-test**（296★）：用 git rev 依赖 libghostty-rs，`rev = "a28e4ad0…"`（[Cargo.toml:37](https://github.com/microsoft/tui-test/blob/f2760592ed/Cargo.toml#L37)）。CI 和发布都用 setup-zig 加 cargo-zigbuild。坑：libghostty-vt 的静态库里带着 C++ 的 `simdutf` 命名空间，另一个依赖 rio-vt 也带一份，两份会撞符号。他们在 `.cargo/config.toml` 里用 `CXXFLAGS_<target> = "-Dsimdutf=tui_test_rio_simdutf"` 给 cc-rs 编的那份改名；另外 Zig 0.16 不接受 simdutf 的 Ice Lake 目标属性，需要 `-DSIMDUTF_IMPLEMENTATION_ICELAKE=0`（[.cargo/config.toml](https://github.com/microsoft/tui-test/blob/f2760592ed/.cargo/config.toml)）。
- **betamax**（joshka，42★）：CI 设了 `LIBGHOSTTY_VT_SYS_CPU: baseline` 和 Zig 0.15.2。

## wsp（wsp-labs/wsp）

[PR #1618](https://github.com/wsp-labs/wsp/pull/1618)（2026-10-19 合并），13★，AGPL-3.0，是一个 GPUI 桌面端。

- 依赖写的是 `libghostty-vt = { git = "https://github.com/Uzaaft/libghostty-rs", rev = "d2036bae…" }`。
- 新增的 `apps/desktop-gpui/.cargo/config.toml` 在 `[env]` 里设了两项：
  - `GHOSTTY_SOURCE_DIR = { value = "vendor/ghostty-<commit>", relative = true }`。目录名带 commit，所以「commit 一变就重编，文件被改不会重编」。
  - `LIBGHOSTTY_VT_SYS_OPTIMIZE = "ReleaseFast"`，注释写「Zig 的 Debug 版带 256 KiB 线程局部数据，glibc 从每个线程 2 MiB 的栈里扣」。
- `scripts/ghostty.sh` 从 codeload.github.com 下载这个 commit 的 tarball，先校验写死的 sha256 再解压，并检查 PATH 上的 zig 是不是 0.16.0。注释说用 tarball 是因为「git clone 或 fetch 会卡上几分钟，tarball 几秒」。

## gpui-cn（prabirshrestha/gpui-cn）

[github.com/prabirshrestha/gpui-cn](https://github.com/prabirshrestha/gpui-cn)，1★，Apache-2.0。`ghostty-vt-sys`/`ghostty-vt` 已发布到 crates.io（0.7.2，2026-10-14）。

- **绑定**：改自 libghostty-rs 0.2.2，`src/bindings.rs` 提交在仓库里。CI 有一个 macOS job 用 `llvm@22` 重新生成绑定，然后 `git diff --exit-code`。注释说 bindgen 保留的文档注释取决于 libclang 版本，所以要和同步脚本用同一个 LLVM（[ci.yml:43-62](https://github.com/prabirshrestha/gpui-cn/blob/2cb138acdf/.github/workflows/ci.yml#L43-L62)）。
- **libghostty-vt 的来源**：build.rs 依次找四处：`GHOSTTY_SOURCE_DIR`、`third_party/ghostty` submodule、`GHOSTTY.lock` 里的 URL、pkg-config（[build.rs:1-14](https://github.com/prabirshrestha/gpui-cn/blob/2cb138acdf/crates/ghostty-vt-sys/build.rs#L1-L14)）。默认 URL 是上游官方的 `https://tip.files.ghostty.org/<commit>/libghostty-vt-source.tar.gz`（约 4.3 MB），下载后先校验 sha256 再解压（[GHOSTTY.lock](https://github.com/prabirshrestha/gpui-cn/blob/2cb138acdf/crates/ghostty-vt-sys/GHOSTTY.lock)，[build.rs:209-272](https://github.com/prabirshrestha/gpui-cn/blob/2cb138acdf/crates/ghostty-vt-sys/build.rs#L209-L272)）。构建固定用 `-Doptimize=ReleaseFast -Dsimd=true -Dcpu=baseline`，不跟 Cargo profile 走（[:329-367](https://github.com/prabirshrestha/gpui-cn/blob/2cb138acdf/crates/ghostty-vt-sys/build.rs#L329-L367)）。找 Zig 的顺序是 `ZIG`、PATH，最后问 `mise`（[:396-430](https://github.com/prabirshrestha/gpui-cn/blob/2cb138acdf/crates/ghostty-vt-sys/build.rs#L396-L430)）。
- **钉版本和升级**：`GHOSTTY.lock` 记 commit、version、url、sha256、root，crate 版本号也带上 ghostty 的 commit（如 `0.7.2+ghostty.1.4.0-main.83fb956`）。只能用 `scripts/sync.sh <commit>` 移动钉住的版本。这个脚本会更新 submodule、两个 lock 文件、crate 版本、绑定和 terminfo 资源，并把官方 tarball 和本地复现的 archive 对比。`scripts/check-ghostty-version.sh` 检查各处版本一致（[.agents/skills/update-ghostty/SKILL.md](https://github.com/prabirshrestha/gpui-cn/blob/2cb138acdf/.agents/skills/update-ghostty/SKILL.md)）。2026-10-28 的提交 04d977c1 升到 Ghostty 1.4.0-main 和 Zig 0.17。
- **平台**：CI 在 macOS、Ubuntu、Windows 上都跑 `cargo test`，装 Zig 0.17.0；还有一个 iOS 模拟器 check，以及一个不拉 submodule、走 tarball 下载的 job。README 写明「这个版本运行时只支持 macOS」。
- **不一致**：SKILL.md 仍写「macOS 上把 Zig 0.16 放在 PATH 最前」，README 和 CI 已是 0.17。

## herdr（herdrdev/herdr）

[github.com/herdrdev/herdr](https://github.com/herdrdev/herdr)，46240★，Apache-2.0，master `2713feb56e`。

- **绑定**：没有独立的 -sys crate。`crates/ghostty-vt` 直接带 bindgen 生成的 `bindings.rs`，由 `scripts/generate_libghostty_bindings.sh` 对 vendor 里的 `vt.h` 运行 bindgen 生成。
- **libghostty-vt 的来源**：仓库提交了 lib-vt 的源码 dist。`scripts/vendor_libghostty_vt.py` 在干净的 ghostty checkout 里跑 `zig build dist -Demit-lib-vt`，把产出的 `libghostty-vt-*.tar.gz` 解到 `vendor/libghostty-vt/`。调查时这个目录约 19.6 MB、1339 个文件。本地补丁记在 [vendor/libghostty-vt.patches.md](https://github.com/herdrdev/herdr/blob/2713feb56e/vendor/libghostty-vt.patches.md)，目前有两个：一个暴露 modifyOtherKeys 状态，一个修 Windows 交叉编译下 Wuffs 的构建。每个补丁都写明删除条件和验证命令。build.rs 对 vendor 目录调 Zig（[build.rs](https://github.com/herdrdev/herdr/blob/2713feb56e/crates/ghostty-vt/build.rs)）。
- **钉版本**：`vendor/libghostty-vt.vendor.json` 记 `source_commit 44f2a44d`。升级在 2026-04、05、06、07、09 各有一次，间隔 1.5 到 2 个月。
- **dev 优化**：`LIBGHOSTTY_VT_OPTIMIZE` 默认 `ReleaseFast`，不看 Cargo profile（[build.rs:58](https://github.com/herdrdev/herdr/blob/2713feb56e/crates/ghostty-vt/build.rs#L58)）。
- **平台和交叉编译**：release 矩阵覆盖 Linux musl x86_64/aarch64、macOS x86_64/arm64 和 Windows x86_64 MSVC，用 `vercel-labs/setup-zig` 装 Zig 0.16.0（[release.yml:134-136](https://github.com/herdrdev/herdr/blob/2713feb56e/.github/workflows/release.yml#L134-L136)）。在 Unix 上对 Windows 跑 clippy 要先做一次性准备：用 `xwin` 下载微软 SDK，生成 Zig 的 libc 配置，再通过 `LIBGHOSTTY_VT_WINDOWS_LIBC` 交给 build.rs，同时设 `LIBGHOSTTY_VT_SIMD=false`（[scripts/windows_cross.py:50-95](https://github.com/herdrdev/herdr/blob/2713feb56e/scripts/windows_cross.py#L50-L95)）。这说明 build.rs 会调 Zig 时，单做 clippy 也得把原生库编出来。
- **坑**：
  - Apple 的链接器在 `-l` 时优先选同目录下的 dylib，所以 build.rs 把 `.a` 单独拷到一个目录再加入搜索路径（[:119-130](https://github.com/herdrdev/herdr/blob/2713feb56e/crates/ghostty-vt/build.rs#L119-L130)）。
  - Windows 上链接 `ghostty-vt-static`（[:132-136](https://github.com/herdrdev/herdr/blob/2713feb56e/crates/ghostty-vt/build.rs#L132-L136)）。
  - `.cargo/config.toml` 给 msvc 目标开了 `+crt-static`。

## Paneflow（arthjean/paneflow）

[github.com/arthjean/paneflow](https://github.com/arthjean/paneflow)，90★，**GPL-3.0**：Condr 只能借思路，不能抄代码。main `03d0b01af0`。博客：[Linux](https://paneflow.dev/blog/libghostty-linux)（2026-07-18）、[Windows](https://paneflow.dev/blog/libghostty-windows)（2026-07-21）。0.17.4 起所有平台只用 Ghostty。

- **绑定**：`paneflow-libghostty-sys` 是唯一的原始 ABI 层，`bindings.rs` 预先生成后提交。安全包装 `paneflow-terminal-ghostty` 在创建终端前会核对 API 版本、枚举值、回调签名，以及结构体的大小、对齐和字段偏移（[native/libghostty/README.md:247-254](https://github.com/arthjean/paneflow/blob/03d0b01af0/native/libghostty/README.md#L247-L254)）。
- **libghostty-vt 的来源**：预编译静态 archive。archive 不进 git，是 GitHub pre-release `libghostty-vt-<sha>` 的资产，每个目标一个文件。`scripts/fetch-libghostty.sh`（Windows 版是 `.ps1`）用 curl 下载，按 manifest 里的 `archive_sha256` 校验后放进 `prebuilt/<target>/lib/`（[README:1-21](https://github.com/arthjean/paneflow/blob/03d0b01af0/native/libghostty/README.md#L1-L21)，[fetch-libghostty.sh:82-94](https://github.com/arthjean/paneflow/blob/03d0b01af0/scripts/fetch-libghostty.sh#L82-L94)）。build.rs 不下载，只在开了 `link` feature 时做三件事：读 manifest，校验 bindings、头文件、build-info 和 archive 的 sha256，输出链接指令。archive 缺失时报错，并提示运行那个脚本（[build_support/mod.rs:53-81](https://github.com/arthjean/paneflow/blob/03d0b01af0/crates/paneflow-libghostty-sys/src/build_support/mod.rs#L53-L81)）。每个跑 cargo 的 CI job 都要先执行 `.github/actions/fetch-libghostty`，而且四个目标全部拉下来，因为 sys crate 的测试在每台机器上都会校验 Linux 和 macOS 两套目录。
- **谁产出、怎么校验**：`libghostty-bump.yml` 只能手动触发（`workflow_dispatch`）。它的步骤是：
  1. 把 manifest 改到目标 commit；
  2. 重新生成绑定；
  3. 重建 4 个目标：Linux 两个分别在 `ubuntu-22.04` 和 `ubuntu-22.04-arm` 上编；macOS arm64 在 Linux 上交叉编；Windows 在 `windows-2022` 上编；
  4. Linux 和 macOS 各干净构建两次，用 `--verify-reproducible` 证明逐字节一致；
  5. 用 `actions/attest-build-provenance` 为 archive 生成 SLSA 证明；
  6. 发成 pre-release，再开 PR 改 manifest。

  PR 不会自动合并。PR 上的 CI 只能核对 archive 和 manifest 里的哈希一致，而哈希就是这个 PR 自己写进去的，所以来源证明必须在 bump run 里完成（[README:217-245](https://github.com/arthjean/paneflow/blob/03d0b01af0/native/libghostty/README.md#L217-L245)）。调查时最新的 archive 都去掉了调试信息，大小 2.5–3.2 MiB。
- **钉版本**：[manifest.toml](https://github.com/arthjean/paneflow/blob/03d0b01af0/native/libghostty/manifest.toml) 记录：
  - 源码：`source_sha b699ea79`、`zig_version 0.16.0`、`build_mode ReleaseFast`；
  - 哈希：header、bindings、notices、SBOM 和每个目标 archive 的 sha256；
  - Windows 工具链：Zig zip、源码包和可执行文件的 sha256，MSVC toolset、Windows SDK、LLVM 的版本；
  - 许可：archive 里各成员的许可清单。

  pre-release 分别发在 2026-09-02、09-03（只改构建配方）、09-14 和 10-07。README 说 bump「每周运行」，但 workflow 只有 `workflow_dispatch`，没有 `schedule`，文档与实现不一致。
- **Zig 升级**：bump workflow 会读目标 commit 的 `minimum_zig_version`。它和 manifest 里钉的 Zig 不同时直接失败，要求先手动换 Zig，因为 Windows 的 Zig URL、校验和、PE 元数据都要跟着改（[libghostty-bump.yml:152-165](https://github.com/arthjean/paneflow/blob/03d0b01af0/.github/workflows/libghostty-bump.yml#L152-L165)）。上游已要求 Zig 0.17，而 Paneflow 还停在 0.16。
- **dev 优化**：archive 一律是 ReleaseFast，不受 Cargo profile 影响。
- **公开记录的坑**（README 和 `docs/release/*`）：
  - Zig 的 `build-lib` 按 CPU 核数开线程池，`-j1` 管不到它。`PageFormatter.formatWithState` 生成的机器码因此在不同构建间差几个字节。Linux 和 macOS 用 `taskset` 把编译钉在一个 CPU 上；Windows 上 Zig 的 `getCpuCount()` 读的是 PEB 里的 `NumberOfProcessors`，不看亲和性掩码，钉不住，所以 Windows archive 不可复现，只能靠证明说明来源。
  - 调试段里的绝对路径会泄露路径长度，所以源码、Zig、缓存都放在固定路径下。
  - Zig 静态链接时把 ntdll 和 kernel32 的导入库整个打进了 archive，要先删掉。
  - Zig 0.15.2 的产物带 `/DEFAULTLIB:libcpmt.lib` 和 `MT_StaticRelease`，0.16 起不带了。
  - 7 月为了让 Windows 构建可复现，曾给上游 formatter 打过补丁，每次升级都要重新 rebase，后来放弃了。
  - 在 9 月之前，archive 是直接提交进 git 的（455be37b 改成从 pre-release 拉取）。

## ghostty-vt-rs（nklmilojevic/ghostty-vt-rs）

[github.com/nklmilojevic/ghostty-vt-rs](https://github.com/nklmilojevic/ghostty-vt-rs)，0★，MIT，crates.io 上的 `ghostty-vt-rs-sys` 0.2.0 共 542 下载。

- **绑定**：改自 libghostty-rs，`bindings.rs` 提交在仓库里。
- **libghostty-vt 的来源**：按以下顺序决定（[README](https://github.com/nklmilojevic/ghostty-vt-rs/blob/cc0a11f2/crates/ghostty-vt-rs-sys/README.md)）：
  1. `pkg-config` feature；
  2. 默认用预编译：build.rs 用系统的 `curl` 从 `https://github.com/nklmilojevic/ghostty-vt-rs/releases/download/ghostty-<commit12>/…` 下载，按 [prebuilt.rs](https://github.com/nklmilojevic/ghostty-vt-rs/blob/cc0a11f2/crates/ghostty-vt-rs-sys/prebuilt.rs) 里的 sha256 校验（[build.rs:12,209-216](https://github.com/nklmilojevic/ghostty-vt-rs/blob/cc0a11f2/crates/ghostty-vt-rs-sys/build.rs#L209-L216)）；
  3. 没有预编译包的目标，或者设了 `GHOSTTY_VT_RS_SYS_BUILD=source`，就用 Zig 0.17 从源码编。

  可用的环境变量：
  - `GHOSTTY_VT_RS_SYS_PREBUILT_DIR`：直接用本地目录里的 archive；
  - `…_PREBUILT_URL`：换镜像；
  - `…_CACHE_DIR`：跨构建保留的缓存目录，放下载的 archive、ghostty checkout 和 Zig 缓存。
- **谁产出**：`prebuilt.yml` 按目标在原生 runner 上构建：macos-26、ubuntu-24.04、ubuntu-24.04-arm，装 Zig 0.17.0。产物发成 release `ghostty-<commit12>`（每个 tar.gz 附一个 `.sha256` 文件），workflow 再开 PR 把校验和写进 `prebuilt.rs`。没有 Windows 目标，也没做证明。调查时 tar.gz 每个 2.5–4.5 MiB。
- **升级流程**（README「Moving to another Ghostty commit」）：
  1. 改 build.rs 和 Cargo.toml 里的 commit；
  2. 用源码构建一次，重新生成绑定；
  3. 合并后运行 Prebuilt workflow；
  4. 合并它开的 PR，再执行 `just release`，`scripts/check-pins.sh` 会检查钉住的版本与 archive 是否对应。
- **dev 优化**：预编译包一律是 `fast`。源码构建在 `DEBUG=true` 时用 `debug`；iOS 例外，因为 Zig 0.17.0 的标准库开着安全检查时编不了 iOS。build.rs 注释说 Zig 0.17 把优化模式改名成 `debug/safe/fast/small`，旧名也接受（未独立核实；libghostty-rs master 配 0.17 仍传 `ReleaseFast`，CI 能过）。

## xipeng-jin/zed 的 migration/libghostty 分支

[xipeng-jin/zed@migration/libghostty](https://github.com/xipeng-jin/zed/tree/migration/libghostty)，个人 fork，3★。最后提交 `e537270d`（2026-07-22，「Swap the terminal backend to ghostty on Linux」）；`migration/libghostty2` 停在 2026-08-28。没有合进 zed-industries。这里的决策文档是本次调查里写得最完整的：[build-strategy.md](https://github.com/xipeng-jin/zed/blob/e537270d/docs/ghostty-migration/build-strategy.md)、[artifact-pipeline.md](https://github.com/xipeng-jin/zed/blob/e537270d/docs/ghostty-migration/artifact-pipeline.md)、[spike-findings.md](https://github.com/xipeng-jin/zed/blob/e537270d/docs/ghostty-migration/spike-findings.md)。

- **绑定**：一次性 vendor 了 libghostty-rs `51bf4bf7`，成为 `crates/ghostty_vt_sys` 和 `crates/ghostty_vt`。之后不跟踪上游，需要时手动 diff 移植（[ghostty_vt_sys/README.md](https://github.com/xipeng-jin/zed/blob/e537270d/crates/ghostty_vt_sys/README.md)）。
- **libghostty-vt 的来源**：build.rs 按以下顺序决定：
  1. `GHOSTTY_VT_LIB_DIR`：本地 archive，给 nix 和离线构建用；
  2. `GHOSTTY_SOURCE_DIR`：本地 ghostty checkout；
  3. `GHOSTTY_VT_FROM_SOURCE=1`：按钉住的提交 git fetch 再用 Zig 编；
  4. 默认：从 `xipeng-jin/libghostty-vt-prebuilt` 的 release 下载 `libghostty-vt-<commit10>-<triple>.tar.gz`，解压前先校验 sha256。

  他们引用的先例是 Zed 已有的 `webrtc-sys` 构建时下载预编译 libwebrtc，以及 Windows 上下载 ConPTY nupkg（build-strategy §2）。
- **谁产出**：独立仓库，只能手动触发（`workflow_dispatch`），用一台 `ubuntu-latest` 靠 Zig 交叉编译出 4 个 Linux 目标。之后用 `nm` 检查 simdutf 和 hwy 的符号，并做一次 C 链接冒烟测试，然后发布 release，附 `SHA256SUMS` 和 GitHub attestation。release 只增不删、永久保留，保证老提交和 `git bisect` 还能构建（artifact-pipeline §1-5）。实际发过两次：2026-07-16 的 `ghostty-a887df42c5` 和 07-22 的 `ghostty-636ce3a46f`。源码钉在自己的 fork `xipeng-jin/ghostty`。
- **钉版本**：[ghostty_pin.toml](https://github.com/xipeng-jin/zed/blob/e537270d/crates/ghostty_vt_sys/ghostty_pin.toml) 是唯一来源。`headers_sha256` 由生成绑定的工具写入，源码构建时由 build.rs 校验，所以光升 pin 不重新生成绑定的改动过不了构建。另有一个 CI job 在每个 PR 上从源码编一次，防止逃生口失效。他们的原则是「需要上游修复时才升级，不定期」。
- **dev 优化**：所有路径都默认 ReleaseFast（build-strategy:214）。依据是他们实测的一个陷阱：Zig Debug 版的 libghostty-vt 一旦 scrollback 非空，`vt_write` 就从约 8 ns/字节退化到约 24 µs/字节，慢了约 3000 倍；`seq 1 200000` 单次调用最长 3.6 秒，最后用了 106.6 秒，期间 GPUI 前台线程完全卡死。换成 ReleaseFast 后，同样的输入只要 54 ms（[spike-findings.md:133-160](https://github.com/xipeng-jin/zed/blob/e537270d/docs/ghostty-migration/spike-findings.md#L133-L160)）。
- **实测数据**（build-strategy §3.4，32 线程机器，Zig 0.15.2）：
  - 冷构建（包括全部网络下载）49.1 秒，峰值内存 605 MB；包已缓存但编译缓存是冷的 36.1 秒；全部缓存命中 0.09 秒。
  - ReleaseFast 的 `.a` 15.3 MB，Debug 的 41.2 MB；没去调试信息。
  - Zig 本地缓存加全局缓存约 650 MB。
  - 即使只编 lib-vt，冷构建也要拉约 24 个 Zig 包。

## gpui-ghostty（Xuanwo）

[github.com/Xuanwo/gpui-ghostty](https://github.com/Xuanwo/gpui-ghostty)，89★，Apache-2.0。不用 lib-vt 的 C API，而是自己写了一个 Zig 包装 `crates/ghostty_vt_sys/zig/lib.zig`，配一个手写的 `include/ghostty_vt.h`。ghostty 源码以 submodule 形式放在 `vendor/ghostty`（`6d2dd585`）。build.rs 在 PATH 上找不到 zig 时，退到 `.context/zig/zig`（提示用 `scripts/bootstrap-zig.sh` 装 0.14.1），然后固定用 `-Doptimize=ReleaseFast` 构建（[build.rs](https://github.com/Xuanwo/gpui-ghostty/blob/94891107/crates/ghostty_vt_sys/build.rs)）。CI 在 Linux 和 macOS 上跑，用 Zig 0.14.1。2026-01-08 之后只有依赖机器人的提交。[博客文章](https://xuanwo.io/2026/01-gpui-ghostty/)讲的是用 AI 写代码的流程，没提构建细节。参考价值低。

behzade/[gpui-libghostty](https://github.com/behzade/gpui-libghostty)（8★）用的是完整的 libghostty（链接 `ghostty-internal`），不是 lib-vt。它把整个 ghostty 源码 vendor 进 crate，用 ReleaseFast 编，跟本调查的范围不同。

## JetBrains（非 Rust，参照）

IntelliJ 2026.3 起默认用 libghostty-vt（[JetBrains 博客](https://blog.jetbrains.com/platform/2026/10/jetbrains-terminal-now-uses-ghostty/)）。打包方式见 intellij-community 的 [plugins/terminal/emulator/README.md](https://github.com/JetBrains/intellij-community/blob/865067b0a7/plugins/terminal/emulator/README.md)：

- **源码**：[JetBrains/ghostty](https://github.com/JetBrains/ghostty) 是普通 fork。调查时相对上游 ahead 0、behind 197，没有自己的提交。同步办法是拉上游的 `tip` 标签（上游把它挂在每个通过测试的提交上）。
- **产出和发布**：内部 TeamCity 配置「Terminal: libghostty-vt / Publish」在 `tip` 上构建，产物上传到 JetBrains 自家存储。一个 `libghostty-vt-<version>.zip.zst` 包含所有 `<os>-<arch>` 子目录，里面是**共享库**，不是静态 archive。版本号写在 [dependencies.properties:10](https://github.com/JetBrains/intellij-community/blob/865067b0a7/build/dependencies/dependencies.properties#L10)：`libGhosttyVtVersion=0.1.0-tip+3a3047f6b.8`。
- **开发机**：从源码运行 IDE 或跑测试时，[TerminalLibGhosttyVtDownloader.kt](https://github.com/JetBrains/intellij-community/blob/865067b0a7/platform/build-scripts/downloader/src/org/jetbrains/intellij/build/dependencies/TerminalLibGhosttyVtDownloader.kt) 自动下载并缓存。`-Dij.terminal.libghostty-vt.lib.root=<dir>` 可以换成自建的库。
- **校验**：Bazel 的 `dev_launch_libghostty` 只给了 URL，没给 sha256（[dev_launch_dependencies.bzl:448-453](https://github.com/JetBrains/intellij-community/blob/865067b0a7/build/dev_launch_dependencies.bzl#L448-L453)）。看不出有哈希校验，未确认。
- **未确认**：TeamCity 配置不公开，用的 Zig 版本和优化等级都不知道。

## 上游 ghostty 发布什么

- **tip 构建**：main 上的提交通过 Test 后，`release-tip.yml` 会发布以下文件。它们托管在 `tip.files.ghostty.org/<commit>/`，都带 minisign 签名（[release-tip.yml:253-410](https://github.com/ghostty-org/ghostty/blob/6ba1441585/.github/workflows/release-tip.yml#L253-L410)）：
  - `libghostty-vt-source.tar.gz`，由 `zig build dist -Demit-lib-vt=true` 生成；
  - `ghostty-vt.xcframework.zip`，只有 Apple 平台，ReleaseFast；
  - wasm。
- **跳过的提交**：如果已经构建过，或者只改了非构建文件，就跳过。所以不是每个 main 提交都有这些文件：libghostty-rs master 钉的 `521cf2c5` 返回 404，而 2026-04-27 的 `6590196` 仍能下载。保留期未确认。
- **没有的东西**：正式版 release（`release-tag.yml`）里没有 lib-vt 产物。Linux 和 Windows 的预编译库，上游一个都不发。
- **Windows MSVC**：上游 CI 的 lib-vt 交叉编译矩阵里，`x86_64-windows-msvc` 被注释掉了，原注释是「还不行，需要办法找到 msvc 的 libc/c++ 头文件」。Windows 只在 Windows runner 上原生构建（[test.yml:609-622](https://github.com/ghostty-org/ghostty/blob/6ba1441585/.github/workflows/test.yml#L609-L622)）。
- **Zig 版本**：ghostty 要求 Zig 的大版本和小版本都与 `minimum_zig_version` 相同（zed 分支文档在 a887 上核实过）。这个要求在 2025-10 升到 0.15，2026-07-21 升到 0.16.0（`e8525c0f`），2026-10-20 升到 0.17（`aa57f1f4`）。

## 没有采用或已放弃的项目

- **Warp**（[warpdotdev/warp](https://github.com/warpdotdev/warp)，65.7k★，AGPL-3.0）：开源后仍用从 alacritty 派生的模型，加上自家 fork 的 `vte`（[Cargo.toml:348](https://github.com/warpdotdev/warp/blob/master/Cargo.toml#L348)），没换成 libghostty-vt。issue [#9250](https://github.com/warpdotdev/warp/issues/9250) 和 [#9913](https://github.com/warpdotdev/warp/issues/9913) 仍开着。Mitchell Hashimoto 在 #9250 里说 libghostty-vt 的速度和 Alacritty「在噪声范围内」，真正的区别在于现代特性。
- **vtcode**（vinhnx/vtcode，892★）：试过预编译**共享库**加运行时 dlopen 的方案。按 `dist/ghostty-vt/<triple>/` 打包，加载失败就回退到 `legacy_vt100`，开发时由脚本自动下载并构建。2026-06-07 的 [2218ca09](https://github.com/vinhnx/vtcode/commit/2218ca09) 把整套集成删了，提交说明没写原因。
- **tatou**（raqqun/tatou，0★，GPL-3.0）：[#125](https://github.com/raqqun/tatou/issues/125)（2026-10-27）的结论是暂时用 alacritty_terminal。理由：libghostty-vt 的 C API 没有稳定版本和标签；构建要 clone ghostty 并跑 Zig，每次构建、`cargo install` 和打包都离不开 Zig 和网络；句柄既不是 Send 也不是 Sync。
- **zeus**（nnayz/zeus）：GitHub API 返回 404，仓库可能已转私有。搜索引擎的快照里，issue #91 还在探索阶段，当时用的是 alacritty_terminal。依赖方式未确认。

## 对 Condr 的启示

### Condr 的前提

- **谁需要 libghostty-vt**：只有 `condr-core` 的 `runtime` feature 用到 VT（`crates/condr-core/Cargo.toml:11-16`），server 和 GUI 会用到。`condr-client` 和 `condr-mobile` 用 `default-features = false`，所以 iOS 和 Android 不受影响。
- **需要编出 libghostty-vt 的地方**：
  - 三台开发机：macOS arm64；Linux arm64 无头服务器，跑 condr-core 的测试；Windows x64 笔记本，编 GUI。
  - CI：`ci.yml` 跑 ubuntu-24.04、windows-2025、macos-15 三个系统。
  - 打包：`build.yml` 在 ubuntu-22.04 和 ubuntu-22.04-arm 上编 Linux，在 windows-2025 上编 Windows，在 macos-15 上编 macOS。
  - 再加上 nightly 和 release。
- **Mac 上的交叉检查**：用 stub 的 cc/ar/llvm-rc 跑 `cargo clippy --target x86_64-pc-windows-msvc` 和 `x86_64-unknown-linux-gnu`。stub 只是造出输出文件，所以构建脚本若真的调用 Zig 编出库来，这套办法就不够用了。
- **dev 帧率**：AGENTS.md 要求 dev 构建的帧率可用，root `Cargo.toml` 的 dev profile 设了 `debug = "limited"`。按 libghostty-rs 的映射规则，这会编出 Debug 版的 libghostty-vt，正好撞上 zed 分支实测的 3000 倍陷阱。注意 `[profile.dev.package]` 里的 `opt-level` 影响不到 Zig，因为 libghostty-rs 先判断 `DEBUG`。在 per-package profile 里设 `debug = false` 能否改掉 build script 看到的 `DEBUG`，未验证。
- **现有文件**：仓库里没有 `.cargo/config.toml`，也没有 `mise.toml`。
- **API 版本**：Condr 要用的 snapshot、search、按字节限制 scrollback、内存统计，只在 libghostty-rs master 上有，crates.io 的 0.2.2 都没有（见 [libghostty-vt-migration-platform.md](libghostty-vt-migration-platform.md)）。所以 libghostty-rs 这条路实际上只能用 git rev 依赖。
- **许可**：可以借代码的有 libghostty-rs（MIT）、gpui-cn（Apache-2.0）、herdr（Apache-2.0）、ghostty-vt-rs（MIT）。Paneflow 是 GPL-3.0，只能借思路，不能抄代码。

### 三条路对比

| | A. 自己的 -sys + 预编译 archive | B. libghostty-rs（git rev）+ Zig 从源码编 | C. 自己的 -sys + Zig 从源码编 |
| --- | --- | --- | --- |
| 开发机要装什么 | 只要 curl（Windows 10 以上自带 curl.exe 和 tar） | 三台都装 Zig 0.17.x 和 git；ghostty 每换一次 Zig 小版本，三台跟着换（用 mise 装和管理） | 三台都装 Zig 0.17.x；能用官方 tip tarball 就不需要 git |
| 首次和 `cargo clean` 后 | 下载约 3–5 MB；把缓存放到 target 外面就不用重复下载 | git clone 加约 24 个 Zig 包，冷构建约 50 秒，Zig 缓存约 650 MB；每个 profile×target 的 `OUT_DIR` 都要编一份 | 同 B；用 tarball 能少一次 git clone |
| dev 帧率 | 天然是 ReleaseFast | 必须新建 `.cargo/config.toml`，在 `[env]` 里设 `LIBGHOSTTY_VT_SYS_OPTIMIZE = "ReleaseFast"`（wsp 的做法） | 在自己的 build.rs 里默认 ReleaseFast（herdr、gpui-cn、zed 分支都是这样） |
| Mac 交叉 clippy | build.rs 只要拿到目标的 archive 文件（clippy 不链接），或者设一个变量跳过 | 设 `DOCS_RS=1` 跳过原生构建，可行但没在 Condr 上验证过，也会影响其他读这个变量的构建脚本；否则得照 herdr 的办法装 xwin 和 Zig libc | 自己加一个跳过原生构建的开关，或者照 herdr 装 xwin |
| CI 改动 | 只多一个下载步骤；另加一个产出 workflow | 每个跑 cargo 的 job 都加 setup-zig 和缓存 | 同 B |
| ghostty 版本由谁决定 | Condr | 跟着 libghostty-rs 的 rev 走，它大约两周升一次 ghostty；想用别的提交只能靠 `GHOSTTY_SOURCE_DIR`，绑定可能对不上 | Condr；还能像 herdr 那样带补丁 |
| 升级要做什么 | 跑 workflow，然后开一个 PR，改 manifest、绑定和包装层 | 改 rev，修 breaking change（libghostty-rs master 常有 `vt!:` 这类破坏性提交） | 改 lock 文件、重新生成绑定、修包装层 |
| 新增的维护面 | 产出 workflow、manifest、build.rs 的下载和校验、绑定新鲜度检查 | 最小 | 绑定新鲜度检查和包装层 |
| 网络 | 只访问 GitHub Releases | GitHub、deps.files.ghostty.org、codeberg（arocc）；wsp 记录过 git 卡上几分钟 | 同 B，用 tarball 时少访问 GitHub git |

A 和 C 说的「自己的 -sys」都不意味着从零写包装层：zed 分支的做法是把 libghostty-rs 的两个 crate 一次性 vendor 进来（MIT，保留版权声明），再换掉 build.rs。B 还有两个变体，都还没人这样用过：一是用 libghostty-rs 的 `pkg-config` feature 去链接一个预编译 archive，它自己的 Nix CI 就是这样链接 Nix 构建好的库的，但 Windows 上需要额外装 pkgconf；二是用 Cargo 的 `links` 覆盖（在 `[target.<triple>.ghostty-vt]` 里给出 `rustc-link-*`），直接跳过它的 build.rs。这两种都未验证。

### 预编译 archive 需要的基础设施

照 Paneflow、ghostty-vt-rs 和 zed 分支已经验证过的部分，配给 Condr：

1. **谁产出**：新增一个只能手动触发的 workflow（`workflow_dispatch`，输入 ghostty 的完整 commit），四个目标分别在原生 runner 上编，环境与 Condr 打包用的一致：
   - `x86_64-unknown-linux-gnu`：ubuntu-22.04；
   - `aarch64-unknown-linux-gnu`：ubuntu-22.04-arm，或者在 x86 上用 Zig 交叉编译（zed 分支就这样）；
   - `aarch64-apple-darwin`：macos-15，也可以照 Paneflow 在 Linux 上交叉编；
   - `x86_64-pc-windows-msvc`：只能用 windows-2025 原生编（上游 msvc 交叉编译不通）。

   每个 job 做这些事：
   - 用 setup-zig 装钉住的版本；
   - 拿源码，优先用这个 commit 的 tip tarball 并校验 sha256，没有就 git checkout；
   - 运行 `zig build -Demit-lib-vt=true -Doptimize=ReleaseFast -Dcpu=baseline -Dtarget=<zig triple> -Dapp-runtime=none -Demit-xcframework=false --prefix out`；
   - 去掉调试信息（Paneflow 去掉后 2.5–3.2 MiB，zed 分支没去时 15 MB）；
   - 做一次链接冒烟测试；
   - 打包，算 sha256，可以顺手做 `attest-build-provenance`。

   逐字节可复现不是必需的：Paneflow 在 Windows 上也做不到。
2. **发到哪里**：两个选择。
   - 发在 Condr 仓库的 pre-release，tag 用 `libghostty-vt-<commit10>` 这种形式。pre-release 不算 `releases/latest`（`script/install-condr.sh:62` 用的就是 latest），也不匹配 cliff 的 `tag_pattern = "^v[0-9]"`，所以不会干扰现有的 release 和 nightly。
   - 发在单独的仓库（zed 分支的做法），让 Condr 的 release 列表保持干净。

   两种都必须只增不删、永久保留，否则老提交和 `git bisect` 就编不了了。
3. **怎么校验**：在仓库里放一个 manifest，记 ghostty commit、Zig 版本、每个目标 archive 的 sha256，以及头文件的 sha256。build.rs 下载后先比对 sha256 再解压；release 里附带的 SHA256SUMS 只用来抄写，不作为信任依据（zed 分支 artifact-pipeline §5）。头文件的 sha256 用来保证 `bindings.rs` 就是从这些头文件生成的（zed 分支的 `headers_sha256`）。另加一个 CI job：重新生成绑定，然后 `git diff --exit-code`。libclang 版本要钉死，否则生成的文档注释会变（gpui-cn 的经验）。
4. **开发机怎么拿到**：推荐 build.rs 自己下载，用 curl，这是 ghostty-vt-rs 和 zed 分支的做法。缓存放在 `target` 外面，按 sha 分目录，比如 `~/.cache/condr/libghostty-vt/<sha>/`，这样 `cargo clean` 后不用重新下载。另外保留两个变量：
   - `CONDR_GHOSTTY_VT_LIB_DIR`：指向本地 archive 目录，用于离线或自己编的库；
   - `CONDR_GHOSTTY_VT_FROM_SOURCE=1`：用 Zig 从源码编。

   Paneflow 的方式是由脚本下载、build.rs 只校验。这样构建过程是封闭的，但 clone 之后和每次升级都多一步手动操作，每个 CI job 也要先跑这个脚本。Mac 交叉 clippy 只需把 Windows 和 Linux 的 archive 也下下来，不需要任何工具链。
5. **升级 ghostty 的流程**：
   1. 选一个有 tip 产物的 commit（等于通过了上游测试），读它的 `minimum_zig_version`。如果 Zig 小版本变了，先改 workflow 和 manifest 里的 Zig，Paneflow 的 bump workflow 会在这里直接拒绝；
   2. 跑产出 workflow，发布 `libghostty-vt-<commit10>`；
   3. 开一个 PR，一起改 manifest 的哈希、重新生成的 `bindings.rs` 和包装层的修复，三者绝不分开提交；
   4. 让三系统 CI 加 Linux arm64 构建都过一遍；
   5. 可选：加一个每周跑的源码构建 job，防止 `CONDR_GHOSTTY_VT_FROM_SOURCE` 这条路失效；
   6. 更新 `script/generate-licenses` 的第三方声明：ghostty（MIT），以及 archive 里带的 simdutf、highway、uucode、wuffs、Zig compiler_rt（Paneflow 的 manifest 列出了这份清单）。

### 建议

- **起步**：B 最省事，适合迁移验证阶段。依赖写成 git rev，用 mise 装 Zig 0.17，在 `.cargo/config.toml` 的 `[env]` 里设 ReleaseFast，交叉 clippy 试试 `DOCS_RS=1`。代价是三台机器加所有 CI job 都要装 Zig，而且每次 ghostty 换 Zig 小版本都得跟着换一遍。
- **长期**：A 更合适。要按 zed 分支的方式做：预编译为默认，Zig 源码编作为逃生口。包装层从 libghostty-rs vendor 过来。只做一个产出 workflow、一个 manifest 和 build.rs 里的下载校验，不必照搬 Paneflow 的可复现构建、SBOM 和 PE 元数据钉死。
- **不推荐 C**：它既有 A 的代码所有权，又保留了 B 的 Zig 负担。只有在 Condr 需要像 herdr 那样给 libghostty-vt 打补丁时才值得选。
