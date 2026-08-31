# Herdr v0.8.2 daemon lifecycle on Linux, Windows, and macOS

Scope: Herdr commit [`9eb521456ac0d19d3ab3d9d7cea3cca10baa8a4c`](https://github.com/herdrdev/herdr/tree/9eb521456ac0d19d3ab3d9d7cea3cca10baa8a4c) (`v0.8.2`). This is a source-code investigation; no Herdr server was started or stopped.

## Conclusion

Herdr ships an application-managed, on-demand detached server on all three desktop platforms. It does **not** install a systemd unit, launchd agent, or Windows Service, and it provides no built-in crash restart or boot startup. A normal `herdr` launch probes the private local endpoint, starts one detached `herdr server` process if needed, waits for readiness, then attaches the client. The server remains alive with zero clients. ([default dispatch](https://github.com/herdrdev/herdr/blob/9eb521456ac0d19d3ab3d9d7cea3cca10baa8a4c/src/main.rs#L803-L815), [autodetect flow](https://github.com/herdrdev/herdr/blob/9eb521456ac0d19d3ab3d9d7cea3cca10baa8a4c/src/server/autodetect.rs#L280-L305), [zero-client contract](https://github.com/herdrdev/herdr/blob/9eb521456ac0d19d3ab3d9d7cea3cca10baa8a4c/src/server/headless.rs#L1-L15))

Linux and macOS use the same Unix detachment primitive: null stdio followed by `setsid()` in `pre_exec`. Windows uses `DETACHED_PROCESS`; when the launcher is inside a Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, it asks WMI `Win32_Process.Create` to create the daemon outside that parent Job. ([daemon command](https://github.com/herdrdev/herdr/blob/9eb521456ac0d19d3ab3d9d7cea3cca10baa8a4c/src/server/autodetect.rs#L179-L235), [Unix detach](https://github.com/herdrdev/herdr/blob/9eb521456ac0d19d3ab3d9d7cea3cca10baa8a4c/src/platform/mod.rs#L72-L94), [Windows launch selection](https://github.com/herdrdev/herdr/blob/9eb521456ac0d19d3ab3d9d7cea3cca10baa8a4c/src/platform/windows.rs#L782-L855), [Job inspection](https://github.com/herdrdev/herdr/blob/9eb521456ac0d19d3ab3d9d7cea3cca10baa8a4c/src/platform/windows.rs#L923-L958))

## Platform comparison

| Concern | Linux | macOS | Windows |
|---|---|---|---|
| Local IPC | Unix-domain socket; liveness is a connect probe | Same Unix-domain socket path | `interprocess` namespaced local socket backed by a named pipe, plus a marker file |
| Normal detach | `stdin/out/err = null`, then `setsid()` | Same | `stdin/out/err = null` plus `DETACHED_PROCESS` |
| Parent containment escape | New Unix session separates it from the launching terminal/session | Same | If the current Job has kill-on-close, launch through WMI; otherwise spawn directly |
| Detached status test | `getsid(0) == getpid()` | Same | No console window and not inside a Job Object |
| Server-only startup difference | No-op file-limit hook | Raises `RLIMIT_NOFILE` soft limit toward 8192 | No-op file-limit hook |
| SSH remote host | Supported | Supported | Not supported in v0.8.2 |
| OS service / restart | None | None | None |

The IPC distinction is implemented centrally: Unix uses `GenericFilePath`, while Windows uses `GenericNamespaced` and writes a marker after binding. ([IPC connect/bind](https://github.com/herdrdev/herdr/blob/9eb521456ac0d19d3ab3d9d7cea3cca10baa8a4c/src/ipc.rs#L35-L78)) The macOS server alone raises its file-descriptor soft limit; Linux and Windows leave it unchanged. ([macOS limit](https://github.com/herdrdev/herdr/blob/9eb521456ac0d19d3ab3d9d7cea3cca10baa8a4c/src/platform/macos.rs#L220-L264), [Linux no-op](https://github.com/herdrdev/herdr/blob/9eb521456ac0d19d3ab3d9d7cea3cca10baa8a4c/src/platform/linux.rs#L38), [Windows no-op](https://github.com/herdrdev/herdr/blob/9eb521456ac0d19d3ab3d9d7cea3cca10baa8a4c/src/platform/windows.rs#L477))

## Local lifecycle

`auto_detect_launch` reuses a compatible server when its endpoint responds; otherwise it starts the current executable as `herdr server`, carrying the launch directory in `HERDR_STARTUP_CWD`, and waits up to 15 seconds. Unix probes the client socket directly; Windows uses the JSON status API first and uses a protocol hello while waiting for readiness. ([liveness checks](https://github.com/herdrdev/herdr/blob/9eb521456ac0d19d3ab3d9d7cea3cca10baa8a4c/src/server/autodetect.rs#L39-L96), [Windows readiness check](https://github.com/herdrdev/herdr/blob/9eb521456ac0d19d3ab3d9d7cea3cca10baa8a4c/src/server/autodetect.rs#L98-L148), [spawn and wait](https://github.com/herdrdev/herdr/blob/9eb521456ac0d19d3ab3d9d7cea3cca10baa8a4c/src/server/autodetect.rs#L242-L305))

The daemon owns the event loop, session state, PTYs, and client listeners. Client detach/disconnect only removes that client and promotes another foreground client; it does not set either shutdown flag. The loop exits only after explicit shutdown state, application quit, or process failure. ([event loop exit conditions](https://github.com/herdrdev/herdr/blob/9eb521456ac0d19d3ab3d9d7cea3cca10baa8a4c/src/server/headless.rs#L535-L579), [client removal](https://github.com/herdrdev/herdr/blob/9eb521456ac0d19d3ab3d9d7cea3cca10baa8a4c/src/server/headless.rs#L1596-L1618), [disconnect handling](https://github.com/herdrdev/herdr/blob/9eb521456ac0d19d3ab3d9d7cea3cca10baa8a4c/src/server/headless.rs#L3304-L3325))

Running `herdr server` directly is the foreground/service-style entry point: main dispatches straight into `run_server` without daemonizing. It still initializes file logging, binds both endpoints, prints readiness on stderr, and blocks in the headless loop. This makes external supervision possible, but Herdr v0.8.2 does not ship the supervisor configuration. ([foreground dispatch](https://github.com/herdrdev/herdr/blob/9eb521456ac0d19d3ab3d9d7cea3cca10baa8a4c/src/main.rs#L574-L581), [server entry point](https://github.com/herdrdev/herdr/blob/9eb521456ac0d19d3ab3d9d7cea3cca10baa8a4c/src/server/headless.rs#L5044-L5138), [ready message](https://github.com/herdrdev/herdr/blob/9eb521456ac0d19d3ab3d9d7cea3cca10baa8a4c/src/server/headless.rs#L5277-L5293))

## Windows Job Object behavior

`launch_server_daemon_command` first calls `IsProcessInJob`; if present, it reads `JOBOBJECT_EXTENDED_LIMIT_INFORMATION`. Only `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` selects the WMI path. WMI receives the fully quoted command line, effective environment, working directory, and `DETACHED_PROCESS`; otherwise ordinary `Command::spawn` is used with the same detach flag already applied. ([selection](https://github.com/herdrdev/herdr/blob/9eb521456ac0d19d3ab3d9d7cea3cca10baa8a4c/src/platform/windows.rs#L782-L855), [environment construction](https://github.com/herdrdev/herdr/blob/9eb521456ac0d19d3ab3d9d7cea3cca10baa8a4c/src/platform/windows.rs#L857-L921), [Job query and detach flag](https://github.com/herdrdev/herdr/blob/9eb521456ac0d19d3ab3d9d7cea3cca10baa8a4c/src/platform/windows.rs#L923-L958))

The server advertises whether it considers itself detached. Unix reports true when it is its own session leader; Windows requires both no console window and no Job membership. Remote compatibility logic treats a running server without this capability as needing restart. ([capability construction](https://github.com/herdrdev/herdr/blob/9eb521456ac0d19d3ab3d9d7cea3cca10baa8a4c/src/api/server.rs#L75-L79), [Windows detached check](https://github.com/herdrdev/herdr/blob/9eb521456ac0d19d3ab3d9d7cea3cca10baa8a4c/src/platform/windows.rs#L960-L966), [remote restart rule](https://github.com/herdrdev/herdr/blob/9eb521456ac0d19d3ab3d9d7cea3cca10baa8a4c/src/remote/attach.rs#L1057-L1075))

## Status, stop, and logs

- `herdr status server [--json]` pings the private JSON API and reports running state, version, protocol, compatibility, socket, session, and detached/live-handoff capabilities. ([status command](https://github.com/herdrdev/herdr/blob/9eb521456ac0d19d3ab3d9d7cea3cca10baa8a4c/src/cli/status.rs#L111-L167), [JSON fields](https://github.com/herdrdev/herdr/blob/9eb521456ac0d19d3ab3d9d7cea3cca10baa8a4c/src/cli/status.rs#L221-L275))
- `herdr server stop` sends `server.stop` over the API socket and waits until both API and client endpoints are unreachable. The server gives stop priority, rejects later API work, notifies clients, closes them, and removes owned socket files. ([CLI stop](https://github.com/herdrdev/herdr/blob/9eb521456ac0d19d3ab3d9d7cea3cca10baa8a4c/src/cli/server.rs#L27-L39), [stop request/wait](https://github.com/herdrdev/herdr/blob/9eb521456ac0d19d3ab3d9d7cea3cca10baa8a4c/src/session.rs#L236-L296), [server shutdown](https://github.com/herdrdev/herdr/blob/9eb521456ac0d19d3ab3d9d7cea3cca10baa8a4c/src/server/headless.rs#L4831-L4888))
- Detached stdio is discarded, so the server writes `herdr-server.log` in the active session data directory. Logging defaults to `herdr=info`; the current file is capped at 5 MiB and, with zero retained generations, is replaced at rollover. ([logging defaults](https://github.com/herdrdev/herdr/blob/9eb521456ac0d19d3ab3d9d7cea3cca10baa8a4c/src/logging.rs#L9-L31), [rotation](https://github.com/herdrdev/herdr/blob/9eb521456ac0d19d3ab3d9d7cea3cca10baa8a4c/src/logging.rs#L503-L568), [session data path](https://github.com/herdrdev/herdr/blob/9eb521456ac0d19d3ab3d9d7cea3cca10baa8a4c/src/session.rs#L157-L170))

## SSH lifecycle

The local client starts system `ssh -T` with hidden command `exec HERDR [--session NAME] remote-client-bridge`. On a Linux or macOS host, that helper checks the private remote socket and protocol; if absent, it calls the same detached-daemon launcher, waits up to five seconds, then pumps the existing binary protocol between SSH stdio and the Unix socket. Losing SSH ends only the bridge/client connection; the independently detached remote server continues. ([remote command](https://github.com/herdrdev/herdr/blob/9eb521456ac0d19d3ab3d9d7cea3cca10baa8a4c/src/remote/attach.rs#L1617-L1625), [SSH process](https://github.com/herdrdev/herdr/blob/9eb521456ac0d19d3ab3d9d7cea3cca10baa8a4c/src/remote/attach.rs#L1816-L1868), [remote ensure and pump](https://github.com/herdrdev/herdr/blob/9eb521456ac0d19d3ab3d9d7cea3cca10baa8a4c/src/remote/host_unix.rs#L8-L69))

Windows can initiate a remote attach, but cannot be the remote Herdr host in v0.8.2 because `remote-client-bridge` is Unix-only and the Windows implementation returns an explicit error. ([platform gate](https://github.com/herdrdev/herdr/blob/9eb521456ac0d19d3ab3d9d7cea3cca10baa8a4c/src/remote.rs#L1-L14))

## Failure and supervision boundaries

Detachment protects the server from client exit, terminal close, SSH loss, and the specific Windows parent-Job kill policy. It does not supervise the server. The built-in launcher performs one spawn followed by readiness polling; after a crash or reboot, the next normal local launch or Unix SSH bridge invocation detects no listener and starts a fresh server. There is no automatic restart loop or boot registration in this lifecycle. This is an inference from the single-spawn launch paths. ([local single-spawn path](https://github.com/herdrdev/herdr/blob/9eb521456ac0d19d3ab3d9d7cea3cca10baa8a4c/src/server/autodetect.rs#L280-L305), [remote single-spawn path](https://github.com/herdrdev/herdr/blob/9eb521456ac0d19d3ab3d9d7cea3cca10baa8a4c/src/remote/host_unix.rs#L51-L69))

Session snapshots can be restored, but a process crash or reboot cannot preserve live PTY processes merely through daemon detachment. If stronger availability is required, an external service manager must own foreground `herdr server`; Herdr itself provides no three-platform service installation layer.

## Test coverage in v0.8.2

- Linux has a process-level assertion that detached children become session leaders, plus Unix tests for live/stale socket detection and readiness polling. ([Linux `setsid` test](https://github.com/herdrdev/herdr/blob/9eb521456ac0d19d3ab3d9d7cea3cca10baa8a4c/src/server/autodetect.rs#L380-L396), [socket/readiness tests](https://github.com/herdrdev/herdr/blob/9eb521456ac0d19d3ab3d9d7cea3cca10baa8a4c/src/server/autodetect.rs#L398-L486))
- Windows tests WMI preservation of environment/working directory and verifies the WMI child reports detached; another process-level test verifies background and daemon commands do not inherit/open a console. ([WMI test](https://github.com/herdrdev/herdr/blob/9eb521456ac0d19d3ab3d9d7cea3cca10baa8a4c/src/platform/windows.rs#L2757-L2808), [console isolation test](https://github.com/herdrdev/herdr/blob/9eb521456ac0d19d3ab3d9d7cea3cca10baa8a4c/src/platform/windows.rs#L2828-L2901))
- API tests verify priority stop control, and headless tests verify stop interrupts queued client events. Remote policy tests verify that a compatible but non-detached daemon requires restart. ([API stop test](https://github.com/herdrdev/herdr/blob/9eb521456ac0d19d3ab3d9d7cea3cca10baa8a4c/src/api/server.rs#L1094-L1125), [headless stop test](https://github.com/herdrdev/herdr/blob/9eb521456ac0d19d3ab3d9d7cea3cca10baa8a4c/src/server/headless.rs#L5560-L5615), [remote daemon policy test](https://github.com/herdrdev/herdr/blob/9eb521456ac0d19d3ab3d9d7cea3cca10baa8a4c/src/remote/attach.rs#L3129-L3139))

Coverage gaps relevant to Condr: the process-level Unix detach test is Linux-only, so the shared macOS `setsid` path has no equivalent runtime test; the Windows WMI helper is tested directly, but the real `current_job_kills_processes_on_close -> WMI` selection is not exercised inside a kill-on-close Job; and there is no end-to-end crash/reboot supervision test because no supervisor exists.

## Implications for Condr

The smallest useful design to borrow is Herdr's on-demand lifecycle, not a service framework:

1. Put one `ensure_server` path behind both local GUI startup and the Unix SSH stdio bridge.
2. On Linux/macOS, launch the server with null stdio and `setsid()`; on Windows, use `DETACHED_PROCESS` and preserve the Job Object/WMI escape for launchers that kill children on close.
3. Keep `condr-server` foreground-capable for manual use or a future external supervisor, and add private-endpoint `status`/`stop` plus a server log before adding systemd, launchd, or Windows Service installation.
4. Test Linux and macOS detach separately and test the actual Windows kill-on-close Job branch. Do not claim crash recovery: reconnect can recover persisted structure, not live PTYs after server death.
