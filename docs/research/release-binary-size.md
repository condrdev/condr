# 发版产物体积：release profile 与语法库

日期：2026-09-28。本文是调研，不是 ADR。代码对照 HEAD `610ea77`，数据在 Windows x86_64 上测得（i9-12900HK，20 线程，32 GB 内存，rustc 1.95.0）。

## 结论

- **现在的发版产物不是体积最优的，但主要不是 profile 的问题。** 根 `Cargo.toml` 没有 `[profile.release]`，发版用的是 Cargo 默认配置：`opt-level = 3`，不做跨 crate 的 LTO，`codegen-units = 16`，只剥离调试信息。开 LTO 后，GUI 只小 2～3%，Server 小 9～14%。
- **GUI 体积有一半是 tree-sitter 语法库。** 72.7 MB 的 `condr-gui.exe` 里有 34.8 MB（48%）是 35 个语法的解析表。这是 ADR 0018 为预览 Tab 有意全部打开的。
- **不过压缩后差距小得多。** 语法表压缩率很高，压缩后只占 GUI 的 3.4 MB（约 21%）。所以去掉语法库，主要省的是安装后的磁盘占用（约 35 MB），下载体积只省 3.4 MB。这些表是只读数据，没用到的语言不会被读进内存，所以对内存影响不大（这一点是推断，未实测）。
- **建议**：
  1. 加 `[profile.release]`，设 `lto = "thin"`、`codegen-units = 1`。代价是每个平台的构建多 12% 左右，换来 Server 小 9%。fat LTO 多花 67% 的构建时间，只再多省一点点，单论体积不划算。
  2. 语法库暂不动。如果要缩 GUI，先做 syntect 的原型量一量（见下文），不要直接砍语言。
  3. `panic = "abort"`、`opt-level = "s"/"z"`、剥离符号表都不建议，理由见下文。

## 测量数据

### release profile 对比

三组都用 `cargo build --release --locked -p condr-gui -p condr-server`，分别构建到独立的 target 目录，profile 用环境变量覆盖（`CARGO_PROFILE_RELEASE_LTO`、`CARGO_PROFILE_RELEASE_CODEGEN_UNITS`），不改 `Cargo.toml`。构建耗时是这台笔记本上依次构建的结果，有波动，只用来比较相对差距。

| 方案 | 构建耗时 | `condr-gui.exe` | gzip -9 后 | `condr.exe` | gzip -9 后 |
|---|---|---|---|---|---|
| 默认（现状） | 7 分 28 秒 | 72.7 MB | 16.4 MB | 23.7 MB | 9.3 MB |
| `lto = "thin"`，`codegen-units = 1` | 8 分 22 秒（+12%） | 71.0 MB（-2.3%） | 16.0 MB | 21.6 MB（-9%） | 8.5 MB |
| `lto = "fat"`，`codegen-units = 1` | 12 分 27 秒（+67%） | 70.4 MB（-3.2%） | 16.0 MB | 20.5 MB（-13.5%） | 8.2 MB |

LTO 对 Server 更有效，因为 Server 基本全是代码；GUI 有一半是数据（语法表），LTO 对数据不起作用。运行性能没有测。LTO 通常会让程序更快，如果要改成 fat，应该先在有代表性的 agent 输出下测一下终端渲染。

作为参考，nightly `0f19751` 的发布包：Windows 安装包 18.2 MB，GUI 的 ZIP 25.9 MB，headless ZIP 9.4 MB；Linux AppImage 29.9～31.9 MB，macOS DMG 32.1～34.1 MB，headless 包 10.0～11.6 MB。

### 语法库的实际占用

用默认 profile 构建 `condr-gui`，只改 `condr-gui/Cargo.toml` 里 `gpui-kit` 的 feature 列表。构建在一个独立的 worktree 里进行，每次构建前恢复原来的 `Cargo.lock`，避免重新解析出新版本的依赖。

| 构建 | `condr-gui.exe` | gzip -9 后 |
|---|---|---|
| 35 个语法（现状） | 72.7 MB | 16.4 MB |
| 去掉 Kotlin、C#、Swift、Scala | 55.1 MB（-17.6 MB） | 15.0 MB（-1.4 MB） |
| 去掉全部语法 | 37.9 MB（-34.8 MB） | 13.0 MB（-3.4 MB） |

每个语法编译后的静态库大小，只作为相对大小的参考，不等于链接进程序后的实际大小（MB）：

| ≥ 2 MB | 0.5～1.5 MB | < 0.5 MB |
|---|---|---|
| Kotlin 5.6、C# 5.2、Swift 3.8、Scala 3.6、C++ 3.4、TypeScript 2.9、PHP 2.2、Ruby 2.1 | Bash 1.4、Elixir 1.4、Rust 1.2、Markdown 0.9、C 0.7、Zig 0.7、Python 0.5、Java 0.5 | JavaScript、Go、YAML、CSS、HTML、TOML、JSON、Diff、Lua、Make、CMake、Proto、GraphQL、Svelte、Astro、JSDoc、EJS/ERB |

### 其他依赖

- 用到了两套加密库：`aws-lc-rs` 来自 GPUI 检查更新用的 HTTP 客户端（`gpui-pre-reqwest-client` → `rustls`），`ring` 来自 condr-server 的 `iroh`（P2P）。两处都是依赖内部的选择，Condr 这边改不了。
- `gpui-base` 声明了依赖 `syntect`（开了 `default-syntaxes`），但源码里没有用到。它会被编译，但没有被引用，链接时会被丢掉，不占最终体积。
- Condr 自己的资源（`assets/`）不到 1 MB，GPUI Kit 的资源 168 KB，可以忽略。
- `wgpu` 在 Windows 上没有编进来。

## 不建议的做法

- **`panic = "abort"`**：现在 Server 里某个线程 panic，只会结束这个线程；改成 abort 后会结束整个 Server，连同所有 Pane、agent 和 PTY。
- **`opt-level = "s"` 或 `"z"`**：终端渲染对性能要求很高（见 `AGENTS.md` 的渲染性能要求）。
- **`strip = true`（剥离符号表）**：Windows 上没有效果，因为符号本来就在单独的 PDB 里。Linux 和 macOS 上能小一些，但崩溃栈里就没有函数名了，不利于根据用户报告排查问题。Cargo 默认的 `strip = "debuginfo"` 已经去掉了调试信息，只保留符号。

## 用 syntect 代替 tree-sitter 做高亮

GPUI Kit 专门留了接口，不需要改 Kit，也不需要自己重写代码查看器：

- `gpui-base` 的 `InputHighlighter` trait（`src/input/editor/highlighting.rs`），文档写的是"与解析器无关的语法高亮接缝"。实现者负责解析和增量状态，编辑器只通过 `styles(range, resolver)` 要带样式的区间，另外还有 `update(edit, text, …)` 和 `fold_ranges`。
- 通过 `InputState::set_highlighter_factory` 装上自己的实现。Kit 的 `Editor` 原有的行号、虚拟滚动、选中、跳转到指定行都照常工作。
- 颜色经 `HighlightStyleResolver`，把"关键字""字符串""注释"这类语义名映射到 Kit 当前的主题上。所以只要把 syntect 的 scope 映射到这些名字，亮色、暗色主题都能沿用。

取舍：

| 方面 | syntect | tree-sitter（现状） |
|---|---|---|
| 体积 | 语法是压缩后的正则规则，整套通常只有几百 KB 到 1～2 MB | 语法表 34.8 MB（压缩后 3.4 MB） |
| 质量 | TextMate/Sublime 语法，`bat`、`delta` 在用，日常足够 | 基于语法树，更准确 |
| 覆盖 | 默认语法集**缺 TypeScript/TSX、TOML、Kotlin、Swift、Zig、Svelte、Elixir 等**，要另外引入 `.sublime-syntax` 文件，并逐个确认许可证与 Apache-2.0 兼容 | 现在的 35 种 |
| 性能 | 必须从文件开头逐行解析，大文件要放到后台线程，否则会卡界面 | 快，可以增量更新 |
| 折叠 | 没有结构信息，只能不支持或按缩进折叠 | 有 |

考虑到压缩后只省 3.4 MB，这件事不急。如果要做，先做原型：用 syntect 实现 `InputHighlighter`，只接到预览 Tab 和 Diff Tab，去掉 tree-sitter 的 feature，然后量三件事：
1. 实际体积（安装后和压缩后）；
2. 几万行大文件的高亮耗时，以及界面是否卡顿；
3. 常见语言（TypeScript、Rust、Python、Go、JSON、Markdown）的高亮效果对比。

有了数据再决定是全换、混用（小语法留 tree-sitter，大的用 syntect），还是维持现状。如果改了，要写新 ADR 修订 ADR 0018 的这部分决定。

## 复现

脚本和结果都在 `C:\Workspace\tmp\condr-size\`：
- `run.sh`：三组 profile 对比。
- `run2.sh`：两组语法对比，在独立的 worktree 里改 feature。
- `results.txt`、`results2.txt`：每组的耗时和体积（字节数）。

构建目录和 worktree 在测量后已经删除。`run.sh` 可以直接重跑；`run2.sh` 要先在仓库里执行 `git worktree add --detach C:/Workspace/tmp/condr-size/src HEAD`。
