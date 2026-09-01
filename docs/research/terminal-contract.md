# `alacritty_terminal` + `portable-pty` 的最小终端契约

上下文：[研究：alacritty_terminal 相对 libghostty-vt 的最小终端契约](https://github.com/condrdev/condr/issues/7)

本轮交互复核基线：Herdr `5158adab10b6dcfea9370782043392f80fa0643c`；Zed `f66ed399cdde86092af8af3dc7b418abf45f37f8`。Zed `terminal`/`terminal_view` 为 GPL-3.0，仅参考外部行为与测试意图，未复制其实现。

## 结论

Condr 不需要为每种 agent CLI 编写启动或终端适配器。窗口启动默认 shell；用户在其中运行任意 agent CLI。MVP 的边界是做成一个行为自洽的 `xterm-256color` 终端：PTY 输出进入同一个 VT 状态机，VT 产生的回复和用户输入有序写回同一个 PTY，GUI 从该状态机渲染、滚动和复制。

`alacritty_terminal` 已覆盖这条路径里最重的 ANSI/VT 状态机。官方支持表包括常用 ESC、CSI 光标/擦除/插删/滚动、SGR、设备状态回复、备用屏幕、应用光标、焦点、鼠标、括号粘贴、同步更新、OSC 标题/颜色/超链接/剪贴板，以及 CSI-u 键盘模式；它不是完整的“所有历史终端协议”实现，也无需为了 agent CLI 补成完整兼容层。[Alacritty escape support](https://github.com/alacritty/alacritty/blob/ede2ac144da4dec4c075bfa803aacf3b3739bce6/extra/man/alacritty-escapes.7.scd#L18-L265)

真正需要 Condr 自己保证的是外围契约。`alacritty_terminal` 的 `Event` 明确把 PTY 回复、尺寸/颜色/剪贴板请求和 redraw wakeup 交给宿主；Alacritty 自己的按键编码则位于 GUI crate，而不在 `alacritty_terminal` crate 内。因此不能把“解析能工作”等同于“终端已经完成”。[terminal events](https://github.com/alacritty/alacritty/blob/ede2ac144da4dec4c075bfa803aacf3b3739bce6/alacritty_terminal/src/event.rs#L9-L58) [frontend keyboard encoder](https://github.com/alacritty/alacritty/blob/ede2ac144da4dec4c075bfa803aacf3b3739bce6/alacritty/src/input/keyboard.rs#L20-L172)

## MVP 必须满足的契约

| 面向 | 最小行为 | 验收条件 |
| --- | --- | --- |
| PTY 生命周期 | 用 `portable_pty::native_pty_system()` 创建初始尺寸的 PTY，spawn 默认 shell，立即丢弃宿主不用的 slave；只取得一个 reader 和一个 writer，串行化所有用户输入与终端回复；EOF/子进程退出能结束 reader 并上报 pane 已退出。`portable-pty` 的主端 API 正好只承诺 clone reader、take-once writer 和 resize。[portable-pty API](https://github.com/wezterm/wezterm/blob/78cd82dbba7315814bfbff40e246b8bed4b702e7/pty/src/lib.rs#L62-L104) | shell 提示符出现；输入 `printf`/`exit` 可往返；Linux PTY 与 Windows ConPTY 都不死锁、不丢尾部输出。 |
| 终端身份 | 不继承外层终端的 `TERM`；固定声明实际实现能承担的 `TERM=xterm-256color`，真彩色通过 `COLORTERM=truecolor` 声明。herdr 也固定这两个值，理由是泄漏外层终端身份会让本地或 SSH 端选择错误 terminfo。[herdr terminal env](https://github.com/herdrdev/herdr/blob/v0.8.2/src/pane.rs#L55-L81) | shell 内两变量稳定；`tput colors` 与颜色/光标控制实际显示一致。 |
| 输出解析 | 每个 PTY byte chunk 按原顺序喂给一个长期存在的 `vte::ansi::Processor`/`Term`，不可按 UTF-8 字符串切块或丢弃不完整序列。必须渲染普通/宽字符、组合字符、wrap、光标、擦除、插删、滚动区、SGR 16/256/truecolor、主屏/备用屏和 cursor visibility。Alacritty 的公开支持表与 `Term::renderable_content()` 已提供这些状态。[escape support](https://github.com/alacritty/alacritty/blob/ede2ac144da4dec4c075bfa803aacf3b3739bce6/extra/man/alacritty-escapes.7.scd#L66-L256) [renderable content](https://github.com/alacritty/alacritty/blob/ede2ac144da4dec4c075bfa803aacf3b3739bce6/alacritty_terminal/src/term/mod.rs#L635-L642) | 一组跨 chunk fixture 能正确显示 ANSI 色、CJK/emoji/组合符、边界 wrap、清屏、滚动区和 `?1049h/l` 往返。 |
| 终端回复 | `Event::PtyWrite` 必须写回 PTY；`TextAreaSizeRequest` 用当前 rows/cols/cell pixels 格式化后写回；`ColorRequest` 从 VT 动态颜色或终端固定 palette 解析。一次 parse 产生的回复按产生顺序批量进入与用户输入共用的 writer，不能并发交错。herdr 在处理 PTY bytes 后收集 terminal responses，再由 I/O actor 排序写回；这是参考行为，不需要复制其 libghostty 专用补丁。[herdr response path](https://github.com/herdrdev/herdr/blob/v0.8.2/src/pane/terminal.rs#L1288-L1308) [Alacritty event contract](https://github.com/alacritty/alacritty/blob/ede2ac144da4dec4c075bfa803aacf3b3739bce6/alacritty_terminal/src/event.rs#L24-L49) | DA/DSR/光标位置、text-area-size 和 palette/default-color 查询能收到响应；同一 parse 中的多种回复保持 FIFO。 |
| Resize | 同一次操作中更新 `Term::resize` 与 `MasterPty::resize(PtySize)`，使用一致的 rows/cols 和可得的 pixel 尺寸；忽略未变化尺寸。`portable-pty` 说明 resize 会更新内核窗口尺寸并通知子进程；Unix 映射到 `TIOCSWINSZ`，ConPTY 映射到 `ResizePseudoConsole`。[portable-pty resize contract](https://github.com/wezterm/wezterm/blob/78cd82dbba7315814bfbff40e246b8bed4b702e7/pty/src/lib.rs#L89-L104) [Unix implementation](https://github.com/wezterm/wezterm/blob/78cd82dbba7315814bfbff40e246b8bed4b702e7/pty/src/unix.rs#L179-L225) [ConPTY implementation](https://github.com/wezterm/wezterm/blob/78cd82dbba7315814bfbff40e246b8bed4b702e7/pty/src/win/conpty.rs#L53-L92) | pane 从 80x24 改到 100x40 后，`stty size`/等价 Windows probe 与 `Term` 均为 40x100；全屏 TUI 重排且无旧行残留。 |
| 键盘 | GPUI 文本/IME commit 直接写 UTF-8；按当前 `TermMode` 编码 Enter、Tab/BackTab、Backspace、Delete、Esc、方向、Home/End、PageUp/PageDown、Insert、F1-F12、Ctrl 字符和 Alt 前缀；`APP_CURSOR` 必须改变方向键编码。MVP 保持 `Config::kitty_keyboard=false`（也是 Alacritty 默认值），让应用走 legacy/xterm 路径；不要协商 CSI-u 后仍发送 legacy。`TermMode` 暴露动态模式，但 Alacritty 的编码器在前端。[TermMode](https://github.com/alacritty/alacritty/blob/ede2ac144da4dec4c075bfa803aacf3b3739bce6/alacritty_terminal/src/term/mod.rs#L53-L110) [default config](https://github.com/alacritty/alacritty/blob/ede2ac144da4dec4c075bfa803aacf3b3739bce6/alacritty_terminal/src/term/mod.rs#L340-L365) [Alacritty mode-aware encoding](https://github.com/alacritty/alacritty/blob/ede2ac144da4dec4c075bfa803aacf3b3739bce6/alacritty/src/input/keyboard.rs#L291-L362) [herdr negotiated encoding](https://github.com/herdrdev/herdr/blob/v0.8.2/src/input/encode.rs#L12-L74) | `cat -v`/byte-capture matrix 覆盖普通文本、IME、Ctrl-C/D/Z、Alt、普通/应用方向键和修饰特殊键；应用探测 CSI-u 时不收到支持声明。 |
| 鼠标与焦点 | GUI 发送语义化 button/motion/wheel/focus 事件，Server 根据权威 `TermMode` 编码 1000/1002/1003 与 legacy/1005/1006；Shift 是本地选择/scrollback 的 escape hatch。应用鼠标与 alternate-scroll 直接写 PTY，不得触发 host `scroll_to_bottom`；历史 scrollback row 不得冒充 live screen 坐标。窗口、Pane、控制权和连接生命周期必须成对发送 focus/release，且同 cell motion 去重，避免持续输出时积压输入。 | 真实 PTY 能断言左右中键、drag/motion、wheel、alternate-scroll 和 focus bytes；Shift 保留本地选择/滚动；ReleaseControl/EOF 会补发最后的 button-up 和 FocusOut。 |
| 粘贴 | 若 `BRACKETED_PASTE` 开启，用 `ESC[200~` 与 `ESC[201~` 包裹文本；否则把平台换行规范成 Enter 的 `\r`。粘贴内容不能伪造 bracketed-paste 结束序列；Alacritty 的实现会过滤 ESC 与 ETX。[Alacritty paste behavior](https://github.com/alacritty/alacritty/blob/ede2ac144da4dec4c075bfa803aacf3b3739bce6/alacritty/src/event.rs#L1364-L1409) | 多行粘贴在 shell/agent 输入框中只作为一次 paste，不逐行提交；关闭模式后行为退化可预测。 |
| Scrollback | 为主屏配置有限但非零 history；滚轮/PageUp 调 `Term::scroll_display`，能到 Top/Bottom，用户离底部时新输出保持当前 viewport，输入时回到底部。备用屏本身不产生 host history。Alacritty 的 grid 原生维护 `display_offset`，且在新输出时保持离底部视图稳定。[grid scrollback invariant](https://github.com/alacritty/alacritty/blob/ede2ac144da4dec4c075bfa803aacf3b3739bce6/alacritty_terminal/src/grid/mod.rs#L125-L176) [scroll while output arrives](https://github.com/alacritty/alacritty/blob/ede2ac144da4dec4c075bfa803aacf3b3739bce6/alacritty_terminal/src/grid/mod.rs#L249-L275) | 产生超过一屏输出，向上滚动后继续输出，viewport 不跳；回到底部可看最新内容；备用屏进出不污染主屏 history。 |
| 选择/复制 | GUI 用 cell metrics 在本地维护并绘制 simple/word/line selection，拖动不能逐 move 往返 Server，也不能因 spinner 等普通 terminal revision 刷新而消失。范围端点保留在 viewport cells；离底部时新输出改变权威 `display_offset`，GUI 同步该 offset，复制仍指向同一批可见 cells。输入、粘贴、显式滚动或切换 Pane 时清理选择。复制时才把范围发给 Server，临时映射到 Alacritty grid point/side 并调用 `selection_to_string()`；不要自己拼 cell 文本，也不要修改持久 VT 选择状态或发布 terminal view。该 API 已处理软换行、宽字符尾 cell、组合字符和行尾换行。[selection model](https://github.com/alacritty/alacritty/blob/ede2ac144da4dec4c075bfa803aacf3b3739bce6/alacritty_terminal/src/selection.rs#L90-L135) [text extraction](https://github.com/alacritty/alacritty/blob/ede2ac144da4dec4c075bfa803aacf3b3739bce6/alacritty_terminal/src/term/mod.rs#L528-L632) | 持续输出时拖动高亮仍留在 GUI 本地且不重复 shaping；跨软换行、CJK 宽字符、组合符、scrollback/viewport 边界拖选后，系统剪贴板内容与屏幕语义一致。 |
| 输出刷新 | PTY 读线程在 VT 锁内更新 revision 后发送轻量 wakeup；Server 合并连续 wakeup，并从 `alacritty_terminal` damage 一次生成 full/cell-run delta，通过独立于可靠 Session events 的 per-client 视觉流发送。每个 client writer 只有一个视觉槽；槽满时不推进 baseline，drain 后从最新 VT 状态重生成。GUI 的可靠消息保持 FIFO，视觉入口只保留一个 token 并合成同 Pane 的连续语义 delta；Bootstrap 代际屏障丢弃旧 token。GUI 应用帧后事件驱动 repaint，revision gap 通过 Bootstrap 重同步。先不要抑制 synchronized-update 中间帧；以后若采用 Alacritty 的抑制优化，必须同时实现 timeout/`stop_sync`，防止应用漏发结束序列后永久不刷新。[Alacritty read/wakeup](https://github.com/alacritty/alacritty/blob/ede2ac144da4dec4c075bfa803aacf3b3739bce6/alacritty_terminal/src/event_loop.rs#L120-L170) [sync timeout](https://github.com/alacritty/alacritty/blob/ede2ac144da4dec4c075bfa803aacf3b3739bce6/alacritty_terminal/src/event_loop.rs#L225-L250) [herdr render stream](https://github.com/herdrdev/herdr/blob/9eb521456ac0d19d3ab3d9d7cea3cca10baa8a4c/src/server/render_stream.rs) | 高频进度输出不积压旧帧且最终状态可见；慢 Client 不阻塞可靠控制消息；resize/scroll/cursor 和 GUI 本地 selection 都能单独触发 repaint。 |
| 可读快照 | core 能从当前 grid 提取 visible plain text，保留 Unicode、去除样式控制码；这是后续 agent 状态检测的输入，不是 agent CLI 启动前提。herdr 同样把 terminal state 与 visible/detection text 暴露在 pane terminal 边界。[herdr terminal boundary](https://github.com/herdrdev/herdr/blob/v0.8.2/src/pane/terminal.rs#L391-L479) | fixture 的底部若干行与渲染可见文本一致；读快照不改变 viewport 或应用状态。 |

## 可以留到完整终端体验的行为

以下项目不阻塞“用户在 shell 中启动并操作 agent CLI”，因此不应先于上述契约：

- Pixel mouse（1016）及 Kitty 等扩展鼠标协议。当前 cell-based 1000/1002/1003 与 legacy/1005/1006 已覆盖文本 agent/TUI；只有真实目标应用要求 pixel 坐标时再扩展同一语义事件。[mouse reports](https://github.com/alacritty/alacritty/blob/ede2ac144da4dec4c075bfa803aacf3b3739bce6/alacritty/src/input/mod.rs#L541-L615)
- CSI-u/Kitty keyboard protocol 与 key press/repeat/release reporting。保持 `Config::kitty_keyboard=false` 时应用会使用 legacy/xterm 输入；只有真实目标 CLI 需要 enhanced keys 时再启用，并一次性补齐 mode-aware encoder。
- Bell、动态窗口标题、cursor blink 调度与 blur cursor 样式。Focus in/out 已按模式和连接生命周期实现。[focus reporting](https://github.com/alacritty/alacritty/blob/ede2ac144da4dec4c075bfa803aacf3b3739bce6/alacritty/src/input/mod.rs#L830-L836)
- OSC 8 hyperlink UI 与 OSC 52 clipboard read/write。颜色查询已实现；OSC 52 在 headless Server 上显式禁用，直到定义 local/remote client、控制权与 clipboard payload 的授权策略。[OSC 52 policy](https://github.com/alacritty/alacritty/blob/ede2ac144da4dec4c075bfa803aacf3b3739bce6/alacritty_terminal/src/term/mod.rs#L358-L385)
- Shift-click 扩展、块选、copy-on-select、自动拖选滚动、搜索、键盘 copy mode、可拖 scrollbar。simple/word/line selection + copy 已覆盖基本复制。
- Kitty graphics、Sixel、图片粘贴、ligature shaping、复杂字体 fallback 和字体感知的长 IME preedit wrapping。这些是完整 GUI 终端体验或特定应用能力，不是文本 agent CLI 的必要条件。
- 会话保活、远程 attach、scrollback 持久化、恢复时重播 keyboard/mouse mode。它们属于后续 server/client 和持久化阶段，不应进入第一版 terminal element。
- 帧率自适应和直接消费 `Term::damage()`。当前跨进程视觉流已按 per-client baseline 生成 cell-run delta；是否进一步复用 Alacritty 内部 damage 需要单独验证 reset 生命周期和多 Client 语义。

## 必须先验证的集成风险

1. **键盘不是库内赠品。** `alacritty_terminal` 维护 mode，但 Alacritty GUI 才把窗口事件编码成 bytes。Condr 需要一个小而明确的 GPUI-key 到 PTY-byte 层，并用 byte matrix 锁住 legacy、application cursor 与 bracketed paste；启用 CSI-u 时再扩展同一 matrix。
2. **终端回复必须共享 writer 顺序。** 若解析线程直接写回复、GUI 线程另写用户输入，查询回复可能与输入交错。一个 pane 一个 writer actor/队列即可；无需为每类事件建通道。
3. **resize 是双写。** 只 resize grid 会让子进程继续按旧尺寸绘制；只 resize PTY 会让 renderer 按旧 grid 截断。herdr 的顺序是先更新 terminal，再把同一尺寸交给 I/O actor。[herdr resize path](https://github.com/herdrdev/herdr/blob/v0.8.2/src/pane.rs#L2608-L2627)
4. **同步更新优化必须成套实现。** `?2026h/l` 是防撕裂优化，不得成为永久冻结开关。最小版本无条件 repaint 最新 state；以后若抑制中间帧，必须同时实现 timeout 兜底。
5. **Windows 需原机验证。** `portable-pty` 统一了 API，但 Unix 使用内核 PTY/ioctl，Windows 使用 ConPTY，行为并不等价；Linux headless 测试不能替代 ConPTY resize、Unicode、Ctrl-C 和 EOF 的 Windows smoke test。[Unix PTY resize](https://github.com/wezterm/wezterm/blob/78cd82dbba7315814bfbff40e246b8bed4b702e7/pty/src/unix.rs#L179-L225) [ConPTY resize](https://github.com/wezterm/wezterm/blob/78cd82dbba7315814bfbff40e246b8bed4b702e7/pty/src/win/conpty.rs#L84-L100)

## 推荐的阶段门

这不是新的项目分层，只是 terminal 工作的最短验收顺序：

1. **Headless loop:** `portable-pty` + `Term` 能启动 shell、解析输出、编码输入、回答终端查询并 resize；fixture 与 PTY smoke test 全过。
2. **Static GPUI render:** 用 `renderable_content()` 显示 cells/attributes/cursor；单一 wakeup 能稳定显示最后一帧。
3. **Interactive terminal:** 接通 mode-aware keyboard、IME commit、paste、scrollback、simple selection/copy 和 resize。
4. **Cross-platform gate:** Linux 自动测试全过；Windows 原机 ConPTY smoke matrix 全过。到这里即可声明“终端内可启动任意文本 agent CLI”，不需要逐 agent 集成。
5. **Parity backlog:** 仅按真实使用问题添加扩展键盘/鼠标、OSC 授权、超链接、图片和持久化/attach。

## 决策

采用 `alacritty_terminal` + `portable-pty`，但把完成标准定义为上述端到端终端契约，而不是“能画出 grid”。MVP 包含正确 VT/PTY/键盘/resize/scrollback/复制/刷新；不包含 agent CLI 自动拉起，也不以 libghostty-vt 的全部能力为目标。遇到行为差异时先以 herdr 的外部行为与测试意图为参考，再决定是否需要补齐。
