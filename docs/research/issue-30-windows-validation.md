# Issue 30: Windows Validation

Validated on 2026-09-07 for [issue #30](https://github.com/condrdev/condr/issues/30).
Base commit before the fixes: `104d5d1` (`fix(agent): address the ADR 0014 review findings`).

## Environment

- Windows 11 Pro, x86_64, build 26100. PowerShell 7.6.5 as the Pane shell; `cmd.exe`
  started inside it for the cmd case.
- Claude Code 2.1.263 (native `claude.exe`); Codex CLI 0.146.0 installed through npm,
  resolved through a `mise` shim, run with `-m qwen3.8:27b` because the configured model
  is not served on this machine. OpenCode is not installed here.
- Isolated Server: `CONDR_CONFIG_DIR` and `--snapshot` under the scratch directory, default
  local pipe. The Server was started from inside a Claude Code session, so Panes inherited
  that session's environment; it changed nothing below.
- `condr agent hooks install claude|codex` against `target/debug/condr.exe`, which is not on
  PATH, so every hook command carries the quoted absolute path.

## Results

| Acceptance item | Result |
| --- | --- |
| Build | Failed at first: the Windows `AttachConsole` path needs `unsafe`, and the workspace forbade it (`-F unsafe-code`). It had never been compiled on Linux. |
| Claude Code in a PowerShell Pane | `agent start` returned `idle` from `SessionStart`. Prompt → `working` (prompt-submit, tool-start/complete) → `blocked` (AskUserQuestion reported as question-asked) → answer → `working` → `idle` (stop). 10 hook runs, all wrote; nothing leaked to the screen. |
| Claude Code in a cmd Pane | `cmd` typed into the PowerShell Pane, `claude` started from it. Same transitions: `idle` → `working` → `idle`, 5 events, no leak. `agent start` refuses a Pane whose foreground is a nested `cmd` (`pane_busy`), so the agent was started with `pane run`. |
| Codex in a PowerShell Pane | After trusting the six hooks from Codex's review prompt: `unknown` → `idle` → `working` → `idle` on the first prompt, no leak. Codex 0.146 fires `SessionStart` with the first turn, not at startup, so the fresh Pane stayed `unknown` for 30 s and `agent start --kind codex` timed out. |
| Hook stays inert elsewhere | The installed Claude hooks also ran in the Claude Code session driving this validation (`CONDR_ENV` unset): every run exited without writing. |
| Cleanup | The isolated Server was stopped; the hooks stay installed. |

## Fixes Required

1. `unsafe_code` is `deny` instead of `forbid` in the workspace lints, with `allow` on the
   two Windows console functions in `hook.rs`. `forbid` cannot be lifted anywhere, and the
   Win32 console calls have no safe wrapper.
2. The hook attaches to the Pane's own process, the ancestor whose parent is the Server,
   before touching the console it inherited. Claude Code spawns its Bash tool with
   `CREATE_NO_WINDOW`, so the hook starts on a private, invisible conhost: `CONOUT$` opened,
   `SetConsoleMode` succeeded, 95 bytes were written, and nothing reached the Server. On the
   Pane's console the mode already had `ENABLE_VIRTUAL_TERMINAL_PROCESSING`, and ConPTY
   passed OSC 777 through to the Server. This is tty7's choice (`e98586b`, Apache-2.0,
   `crates/tty7-core/src/core/agent_hooks.rs`: it attaches to the shell whose parent is the
   tty7 host); Condr does not copy its fallback of writing to every ancestor.
3. Nesting is counted in instances, not processes. Codex through npm and `mise` is
   `codex.exe` ← `node` ← `mise` ← `node`, three processes that identify as Codex, so the
   old count made every Codex hook believe it was nested and stay silent. Consecutive agent
   processes are now one instance; a shell between two of them starts another.
4. Codex on Windows runs hook commands through the session shell, PowerShell here
   (`build_hooks_config` derives it from the turn's shell). `"C:\…\condr.exe" agent-hook …`
   is a string literal followed by arguments to PowerShell: parse error, exit 1, shown as
   `hook (failed)` in Codex. The installed Codex command now starts with the `&` call
   operator when the path is quoted. Claude Code runs hooks through Git Bash, which accepts
   the quoted path and would reject `&`. tty7 has the same problem and only avoids it when
   the binary is on PATH; herdr wraps every hook in `powershell -File`.
5. `agent hooks install codex` could not run `codex features enable hooks`: `codex` is
   `codex.cmd` on Windows, which `Command::new("codex")` does not find. The installer now
   uses the executable that agent discovery resolves.

Ruled out along the way: same-kind nesting through the Server's own ancestry (the Server's
parent had exited, so the chain stops there); `cmd.exe /C` quote stripping (Codex wraps the
command in one more pair of quotes, and `cmd` strips exactly those); Codex's
`shell_environment_policy = core` removing `CONDR_ENV` (the hook environment is the session
snapshot and kept it).

## Checks

- Windows: `cargo test -p condr-core -p condr-server -- --test-threads=1`,
  `cargo clippy -p condr-core -p condr-server --all-targets -- -D warnings` and
  `cargo fmt --check` passed on the final tree. While the acceptance Server and its three
  agent Panes were still running, `agent_cli`'s blocked/timeout test failed twice in the
  suite and passed alone; with that Server stopped the suite passed twice in a row.
- Linux arm64 (`jpdev`): the same after pushing.
