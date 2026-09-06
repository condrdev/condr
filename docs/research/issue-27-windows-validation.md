# Issue 27: Windows Validation

Validated on 2026-09-06 for [issue #27](https://github.com/condrdev/condr/issues/27).
Base commit before the fixes: `cc6c9fa22db0531b57069b48b603b480e1a3dca4`.

## Environment

- Windows 11 Pro for Workstations, x86_64, build 26200.
- PowerShell 7.6.5 for real-agent acceptance; Windows PowerShell
  5.1.26100.9168 for native argument integration tests.
- Codex CLI 0.153.4; Cargo 1.95.0.
- Separate temporary config, named pipe, snapshot and Git workspace. Both the
  Condr executable directory and the workspace path contain spaces.
- Accepted executable SHA-256:
  `9143DBACFDA9A12A394517B56F506CFB85ED63C039945AE6DDA44F332F56AAEE`.

## Results

| Acceptance item | Result |
| --- | --- |
| Installed agents | Discovered the actual Claude and Codex executables on the Server's PATH. |
| Native argument preservation | Windows PowerShell + ConPTY preserved an empty argument, spaces, embedded quotes, literal shell expressions and trailing backslashes. No injected command ran. |
| Named agent lifecycle | Startup reservation, concurrent launch rejection, prompt/wait, blocked/timeout/exit, disconnect handling and name reuse passed. |
| Pane environment | The CLI remained callable through `CONDR_BIN_PATH` after clearing PATH. |
| Real orchestration | Codex in Pane 3 read `--skill`, discovered agents, split Pane 5, started `winworker`, prompted it and waited for idle, then read its output. |
| Independent worker output | `result.txt` contained exactly `35 30 31 37 0A` (`5017` plus LF), produced by the worker for `173*29`. |
| Automatic submission | Both coordinator and worker prompts completed without supplementary `send-keys` or manual Enter. |
| External restart | Server PID changed from 19172 to 4008; all 9 old owned processes exited. Pane IDs 3/5, layout, focus and cwd were preserved; agent list became empty. |
| Additional lifecycle checks | Automated tests covered restart of a stopped Server, rejection inside a Pane, changed runtime epoch and preserved TCP pairing. |
| Cleanup | The isolated acceptance Server was stopped after validation. |

The coordinator's final response was `WINDOWS_ORCHESTRATION_COMPLETE 5017`.

## Fixes Required

1. Moved Unix-only argument fixtures into their conditional compilation block:
   the prior placement prevented Windows test compilation with E0283.
2. Read process command lines into a separate, short-lived `sysinfo::System`.
   In sysinfo 0.31, partial refreshes leave `updated` flags set; mixing them into
   the shared process table retained an exited process through another full
   refresh, causing Windows `prompt --wait` to time out instead of reporting exit.
3. Queued prompt text and delayed Enter as one bounded input entry. Immediate
   Enter, and a 300 ms interval, did not submit the real Windows Codex long prompt.
   A 1 second Windows interval completed both real-agent prompts; Unix uses
   300 ms. Shutdown cancels pending Enter, and other input cannot interleave.
   Ordinary key events have no added delay.

The existing agent integration suite now runs on Windows as well as Unix. Its
Windows fixture is the test executable launched through PowerShell as `codex.exe`;
real installed Codex was validated separately as described above.

## Checks

- Windows and Linux arm64: full `condr-core` / `condr-server` test suites passed
  with `--test-threads=1` after the process-table fix and initial submission fix.
- Final Windows submission interval: both submission unit tests and all agent
  integration tests passed again, followed by the real-agent acceptance above.
- Final code: `cargo clippy -p condr-core -p condr-server --all-targets -- -D warnings`
  passed on both platforms; formatting and `git diff --check` passed.
- Linux checks used an isolated Git worktree on `jpdev` with a Git patch.

## Codex Permissions

The first real-agent attempt used `workspace-write` with
`sandbox_workspace_write.network_access=true`. Windows denied the agent's access
to the private named pipe with `not_authorized` / OS error 5. The successful
isolated acceptance used `--no-alt-screen -s danger-full-access -a never` as
per-invocation native arguments. This disables the Codex command sandbox; Condr
does not inject it or change global agent configuration. See the
[official permission documentation](https://learn.chatgpt.com/docs/agent-approvals-security).

## Local Evidence

Artifacts remain under `%TEMP%\condr-issue27-windows-cc6c9fa`:
`commands.jsonl`, `binary.json`, `environment.json`, `coordinator-prompt.json`,
`worker-wait.json`, both terminal transcripts, before/after restart snapshots,
`restart.json`, and `workspace with spaces\acceptance.json` / `result.txt`.
The earlier failed attempts are retained separately from the successful result.
