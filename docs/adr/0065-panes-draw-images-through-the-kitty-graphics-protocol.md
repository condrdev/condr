# Panes Draw Images Through the Kitty Graphics Protocol

Status: accepted 2026-11-13. Amends ADR 0051: kitty graphics are on, XTVERSION answers `ghostty (Condr <version>)`, and the cell size the Server holds is in device pixels. Amends ADR 0061: a Stream connection an image opened stays up as long as the terminal connection. Builds on ADR 0004, whose visual frames carry the placements; ADR 0036, whose sizing Client sets the cell size images are measured in; ADR 0037, whose image decoding the GUI reuses; and ADR 0049, whose release carries the Companion's part. ADR 0051 and 0061 carry notes pointing here.

Programs draw images with the kitty graphics protocol: OpenCode's image preview, yazi's previews, `kitten icat`, viu, ranger, image.nvim, presenterm, mpv's `--vo=kitty`, anything built on ratatui-image, and Claude Code's own marks. ADR 0051 kept the protocol off, so they fall back to text or never ask. The roadmap held it back until it was known which programs draw; reading each one's source answered that (below). libghostty-vt implements the whole protocol: parsing, storage and eviction, Unicode placeholders, relative placements, animation, and for each frame the list of what to draw where. Condr turns it on, carries that list to the Clients and draws it. Ghostty's renderer is not part of the library, so the drawing is Condr's own.

Two terms, both the protocol's own and used here only in that sense. An **image** is pixels a program transmitted. A **placement** is one place an image is shown in a Pane. Neither joins `CONTEXT.md`, whose Pane entry gains one sentence; neither is the Preview Tab's image file (ADR 0037).

## What is on

**Storage.** `GHOSTTY_TERMINAL_OPT_KITTY_IMAGE_STORAGE_LIMIT` is 64 MiB for each screen, so a Pane holds at most 64 MiB on its primary screen and 64 MiB on its alternate one. The library's default of 10 MB does not hold one 4K screenshot, 33 MB as RGBA; Ghostty and kitty give 320 MB to each screen of a window, where one Condr Server holds many Panes. Eviction is the library's: unused and transient images first, oldest first. The limit is written in, with no setting; a setting would set it to 0, at which the library answers nothing, which programs read as no support. `GhosttyTerminalMemoryUsage` reports each screen's image bytes for a later per-Pane memory view.

**PNG.** The library needs the embedder to decode PNG, through the process-global `GHOSTTY_SYS_OPT_DECODE_PNG`. `condr-ghostty-vt` installs it once per process beside the log callback and decodes with the `png` crate, already in `Cargo.lock` through `image`; `condr-core` takes the same crate under its `runtime` feature to encode. The callback asks the library's allocator for the pixels before it decodes, so an image over the library's per-image cap is refused unread, and it catches its own panic, which rejects the image rather than unwinding across the FFI boundary. Raw RGB and RGBA and zlib compression need nothing from Condr.

**All four transmission media.** Direct (`t=d`) carries base64 through the PTY; file (`t=f`), temporary file (`t=t`) and shared memory (`t=s`) name something the terminal reads. The library reads them on the Server, which is the machine the program runs on, as the user the program runs as. It applies kitty's rules: a path is resolved through the open file to defeat a swap between check and use, only regular files are read, `/proc`, `/sys` and `/dev` other than `/dev/shm` are refused, a read stops at 400 MB, and a temporary file is deleted only from a temporary directory and only when its name contains `tty-graphics-protocol`. The temporary directory the library is given is the Server's `std::env::temp_dir()`: on macOS `$TMPDIR` is under `/var/folders`, which the library's built-in `/tmp`, `/private/tmp` and `/dev/shm` miss. What remains is a host reached by `ssh` inside a Pane naming a path on the Server's machine. A file that cannot be opened, checked or read answers `EINVAL: invalid data` whatever the cause, so `t=f` tells it nothing about the path. `t=t` opens before it checks the directory and the name, and those two checks answer errors of their own, so it can tell that a path outside a temporary directory exists; it cannot read or delete it. Any pixels show only on the person's own screen. The media matter: image.nvim and snacks.nvim always send `t=f`, ranger and presenterm prefer it, Claude Code probes it, and `kitten icat` prefers shared memory. On Windows the library refuses shared memory itself.

**Answers.** The library answers `a=q` queries and every transmission's OK or error through `write_pty`, in stream order, so the answer comes before the DA1 reply that programs read as the end of theirs. It honours `q=1` and `q=2`.

**Not on.** Sixel and iTerm2's `OSC 1337;File=`: the library has no Sixel decoder and Ghostty declines one, and it parses `File=` only to drop it. Either would need Condr to scan the PTY's output ahead of the VT, which is what ADR 0051 removed. Kitty 0.50's feature query (`e=`), which lets a program ask whether placeholders draw instead of reading the terminal's name, comes with a Ghostty pin that has it (ghostty-org/ghostty#14866).

## Who draws

Read from each program's source in November 2026. Programs that send `a=q` and believe the answer draw as soon as the library answers: OpenCode, `kitten icat`, viu, ranger, presenterm and ratatui-image. mpv and kitcat send without asking, and image.nvim draws when configured to. Many others decide by the terminal's name, because an OK to `a=q` does not say whether Unicode placeholders draw: Konsole and VS Code answer OK and do not draw them, and placeholders are how Claude Code and yazi's newer driver place images.

| Program | Decides by | Draws with the answer below |
| --- | --- | --- |
| Claude Code | `a=q` answered and XTVERSION `kitty(` from 0.28 or starting `ghostty ` | yes, with placeholders |
| yazi | `a=q`; placeholders only for kitty, Ghostty, Rio, Rex and st-graphics by name | yes, with placeholders |
| snacks.nvim, timg | XTVERSION containing kitty or ghostty | yes |
| Codex | environment only: `KITTY_WINDOW_ID`, `WEZTERM_*`, `TERM` or `TERM_PROGRAM` naming kitty, ghostty or wezterm | no |
| chafa, fzf's preview script | environment only | no; `chafa -f kitty` does |
| Gemini CLI | draws no images | — |

## The terminal says it runs Ghostty's core

XTVERSION answers `ghostty (Condr <version>)`. It is true: the VT is Ghostty's, and what a program assumes of Ghostty's protocol, kitty keys, synchronized updates, OSC 9;4 progress, placeholders, Condr has. Claude Code, yazi, snacks.nvim and timg then draw, and yazi takes its placeholder driver. The libghostty embedders that draw images for Claude Code do the same or more: Paneflow, cmux and herdr-core answer `ghostty <version>`, a herdr fork `ghostty (herdr <version>)`. JetBrains kept its own name, and JetBrains draws no images yet; its reason, that a name containing "ghostty" made yazi send images with the protocol off, does not hold once the protocol is on. A terminal that kept its own name and waited for each program's list is still waiting for Claude Code's (anthropics/claude-code#99566 and #101306, unanswered).

`TERM_PROGRAM` stays unset and `TERM` stays `xterm-256color`. `TERM_PROGRAM=ghostty` would reach Codex and chafa too, but it is read as the Ghostty application, by Claude Code's `/terminal-setup` among others, which edits Ghostty's configuration. XTGETTCAP's name stays `xterm-256color`. Codex therefore draws nothing; its maintainers declined herdr on the ground that a multiplexer answers for a terminal it cannot vouch for (openai/codex#57541), which is not Condr's case, so an issue asking Codex to believe `a=q` or XTVERSION is drafted for the person to file.

The new name ships in the commit that turns the storage on, never before it: claiming Ghostty while the GUI cannot draw is the false OK zellij and herdr shipped (zellij-org/zellij#5811, herdrdev/herdr#4978).

## The cell size is in device pixels

A program sizes an image as the cells it covers times the cell's pixel size, which it reads from `CSI 16t` or the PTY's pixel size. Condr reported the cell in logical pixels, so on a 2× display yazi drew its preview at half resolution and the GUI scaled it up. The GUI now reports its cell in device pixels, the logical size times the window's scale factor, rounded, as kitty and Ghostty do. The sizing Client's report (ADR 0036) is what the VT holds, and the size reports, the PTY's pixel size and SGR-pixel mouse positions all follow it. A window moved to a display with another scale factor sends a resize that changes only the pixel size, and programs redraw for it.

The Companion reports its real cell size too, where it reported 0×0. With no cell pixel size the library's list of what to draw is empty, so a phone sizing a Pane would hide its images on every Client and tell programs there are no pixels.

## The Server reads what to draw

After each render state update the producer compares `IMAGE_PLACEMENTS_GENERATION` with the last it saw. When it changed, it walks the render state's image iterator: for each placement its layer, z, viewport column and row (negative when it starts above the viewport), cell offset, destination size in pixels, source rectangle, and image id and generation. Placeholders arrive resolved, one entry for each row of cells; Condr does no geometry of its own. Entries are sorted by layer, then z, then image id. The list covers the viewport, which scrolls on the Server (ADR 0036), so an image in the scrollback is drawn when the person scrolls to it.

An image a placement names whose generation the Pane has not seen is copied out and hashed under the Pane's lock, since its pixel pointer dies at the next write or animation tick: SHA-256 over its width, height, channels and pixels, once per generation. The copy goes to the Server's image cache, which keeps the pixels and encodes them as PNG at the fast compression level the first time a Client fetches them, outside any lock, so an animation frame nobody fetches costs no PNG and a large image does not hold up its frame. An image transmitted but never placed is neither copied nor hashed. The cache is the Server's, shared by every Pane, 128 MiB counted as pixels until the first fetch and as PNG after, least recently named in a frame evicted first. It is keyed by content rather than by id and generation, so the second pass of a looping animation and one image in two Panes are encoded once.

Creating a terminal now hands the library its cell size too; it took one only on a resize, and with none no image has a size to draw at.

**Placeholder cells leave blank.** A placeholder cell holds U+10EEEE and combining marks that encode its row and column, with the image id in its foreground colour. The Client needs none of it once the placement is resolved, so in rows the render state flags `KITTY_VIRTUAL_PLACEHOLDER` those cells go out as empty cells that keep their background. An older GUI, the Companion before it draws images, a copy, a selection and URL detection see blanks, not a row of private-use characters.

**Animation.** The library composes frames and keeps no timer. For a Pane with an animation playing that some Client watches, the monitor calls `ghostty_kitty_graphics_animation_tick` when the time it last returned is due, at most once per frame interval (16.667 ms). A new frame is a new generation and goes as any new image does; after the first loop every frame's hash is cached. A Pane no Client watches does not tick, and resumes when one does.

## On the wire

**Placements ride the visual frame.** `TerminalView` gains `TerminalImages images`, a list of `TerminalImagePlacement`: the image's hash and pixel size, layer (below backgrounds, below text, above text), z, viewport row and column, cell offset, destination size and source rectangle, all in the VT's pixels, which `TerminalSize`'s cell size names. A full view always carries it. `TerminalViewDelta` carries it, whole, only when it changed, so a present but empty list means no images and an absent one means unchanged. A change of placements alone makes a frame, and coalesces like any other change (ADR 0004). An older Client ignores the field and draws no image. Bootstrap's views carry the placements and clear nothing a Client has cached.

**Pixels ride a Stream.** `StreamOpen` gains `image`, a 32-byte hash. The Server opens the Stream, sends the PNG and finishes it; a hash its cache no longer holds is `NOT_FOUND`, and the Client draws nothing for it, trying again after two seconds while a frame still names it. An older Server answers `UNSUPPORTED`. A Client fetches at most four images at a time from one Device. Pixels never travel on the terminal connection: ADR 0061 found that a byte handed to the kernel there is a byte the next keystroke and frame wait behind, whatever its priority in user space, and a 500 KB image on a 10 Mbit link would hold them 400 ms. A herdr-style low-priority lane on the terminal connection was declined for that reason, and WezTerm's first design, image bytes inside the cells, for the CPU it cost (wezterm/wezterm#1237); WezTerm's second, a hash in the line and a lazy fetch, is this one.

**The Stream connection stays up** (amending ADR 0061's lifetime). The first image a GUI lacks opens the Device's Stream connection if none is up, and once an image has opened it, it stays up until the terminal connection ends rather than closing with the last forward and transfer. On an SSH Device it is a second login, with `ControlMaster=no` as ADR 0061 has it, unless the person's own `ControlPath` carries it; staying up makes that one login for each connection rather than one each time an image appears after a quiet spell. A FIDO key that wants a touch asks for it when the first image appears. A Stream connection that fails to open is logged and shown nowhere, since the person started nothing; that Device's images stay undrawn, and it is retried on ADR 0061's backoff while a frame names an image the GUI lacks.

## The GUI draws

A fetched PNG is decoded off the UI thread into a `RenderImage`, as the Preview Tab decodes (ADR 0037), and cached by hash for the whole GUI process, every window sharing it, up to 256 MiB of decoded pixels, least recently drawn evicted first and freed with `cx.drop_image`. An image not yet decoded draws nothing, except that a placement keeps the image it drew last at its place in the list while the sizes match, so an animation's next frame, still on its way, does not leave a gap.

The terminal element paints, in order: images below backgrounds, cell backgrounds, images below text, text and its decorations, images above text, the selection, then the cursor and IME text. The selection and the cursor are never hidden by an image. Each placement is one `Window::paint_image`, whose `image_bounds` is the whole image scaled and `bounds` the destination, so the source rectangle crops with no drawing code of Condr's; the Pane's content mask clips it. Positions are the VT's pixels times this window's cell size over the VT's, so the sizing window divides by its scale factor and a window at another font size scales to its own cells. Aspect ratio is the library's, already in the destination size; the GUI never fits an image itself (herdrdev/herdr#4858). Unchanged cells keep their shaping when only placements change.

## The Companion

It reports its cell size in device pixels with the grid, where it reported none, and draws images, released on its own version (ADR 0049). The library turns each placement into the screen's cells, a fraction of the image for its source, so the phone draws at its own cell size, and fetches the PNG over a Stream connection of its own: opened by the first image of the Pane on screen, at most four at a time, and held until the Device's connection ends. It keeps 64 MiB of PNG, which the app reads by the placement's key and decodes natively. Android draws the layers around its own cell backgrounds and text, as the desktop does. iOS draws every image over SwiftTerm's view, which paints backgrounds and text in one pass, so there an image a program put under its text covers the text; a placeholder image's cells are blank, so it hides nothing.

## Windows

The same code. The ConPTY Condr ships (ADR 0056, 1.25.260930003) forwards a program's output unchanged, APC included, so an image a program sends is drawn. It drops APC on the way back: an `a=q` answer never reaches the program, so a program that asks sees only the DA1 reply and decides there is no support. And when a program reads console state, or the Pane resizes, during a long chunked upload, it may write `ESC[6n` into the middle of it, cutting the image and leaving its base64 on screen (microsoft/terminal#19621; herdrdev/herdr#5657 sees the same). Neither is worked around: 1.24.261001001 is reported free of the second but not the first, and moving to it would reopen ADR 0056's measurements. The trigger is a ConPTY package with microsoft/terminal#20872, which passes APC answers back, and a fix for #19621. Windows has no pixel size in the PTY; `CSI 14t` and `16t` reach Condr's VT and are answered.

## Not done

Sixel and iTerm2's images. Images across a Server restart, which come with terminal content in an ADR of its own amending ADR 0005; the library's snapshot already carries images. A setting to turn images off or size their storage. Throttling images on slow links, until the bandwidth measured below asks for it. Saving or opening an image from a Pane. Per-Pane image memory in the CLI or Settings. `TERM_PROGRAM`. A workaround for programs that read only the environment.

## Consequences

- Codex, chafa and fzf's preview script draw nothing until they read `a=q` or XTVERSION.
- ranger turns images off when a read fails with anything but `EBADF`, so ranger in an `ssh` session inside a Pane, naming files the Server's machine lacks, shows no images. Locally it draws.
- An older GUI shows blank cells where images are, while the program believes they draw.
- Memory: a Pane may hold 128 MiB of images in its two screens, the Server 128 MiB of PNG beside them, and a GUI 256 MiB of decoded pixels.
- A Device that showed an image keeps a Stream connection for as long as the GUI stays connected to it, and an SSH Device logs in twice.

## Delivery

On `main`, in six slices. The storage limit stays 0 and XTVERSION `Condr <version>` until the third lands both, with the GUI drawing, so no nightly answers OK to what it cannot draw.

1. `condr-ghostty-vt` binds the `ghostty_kitty_graphics_*` and `ghostty_render_state_image_*` functions; the Server decodes PNG, reads placements, blanks placeholder cells, hashes, encodes and caches.
2. The protocol: placements in the views and deltas, the `image` Stream target, the Stream connection's lifetime.
3. The GUI draws; the cell size is in device pixels; the storage limit, the media and the new XTVERSION turn on.
4. Animation.
5. The Companion reports its cell size.
6. The Companion draws images.

The website's terminal page gains an Images section with what draws, the switches for programs that decide by name (`CLAUDE_CODE_FORCE_TERMINAL_IMAGES=1` where the name is not enough, `chafa -f kitty`), and the Windows limits.

## Tests

Unit tests cover placeholder cells leaving blank; a delta carrying placements only when they changed, and a placement change alone making a frame; one hash across generations with equal pixels; the Server's and the GUI's caches staying within their bounds; an evicted hash answered `NOT_FOUND`; the PNG callback rejecting bad data and surviving a panic; a scale-factor change resizing only the pixel size; and the GUI's scaling for a window at another cell size.

AGENTS.md's performance acceptance runs with an image on screen and a GIF playing: burst wakeups still coalesce to the latest revision, the final frame is never dropped, unchanged cells are not reshaped when only placements change, a real GPUI drag-selection over an image stays smooth, and on Windows interaction and frame rate are watched under an Agent's animation.

End to end: a Client fetches an image's PNG from a running Server over a Stream, and an unknown key is `NOT_FOUND`; the Companion's library, against a real Server over TCP, shows a Pane a program prints an image into, with the placement in its cells and the PNG fetched; the GUI's offscreen screenshots (`CONDR_GUI_SCREENSHOTS`) draw, as `01g-terminal-image`, a PNG the shell prints from a file (`t=f`), fetched over the Device's Stream and drawn at a 2× window's device pixels.

By hand on macOS and Linux: OpenCode, yazi, `kitten icat` over each medium, viu, a ratatui-image program (placeholders), image.nvim (`t=f`), `mpv --vo=kitty`, and Claude Code through its XTVERSION check. Images follow scrolling into the scrollback; `clear` and leaving the alternate screen leave nothing behind (herdrdev/herdr#5705); a resize and a window moved across displays redraw; a second window at another font size scales; a reconnect fetches what it lacks; an older GUI shows blanks. One large image and one GIF over Local, TCP and Peer-to-peer, with time to first draw and bytes sent recorded here. On Windows, the ConPTY behaviour above is confirmed as read from its source.
