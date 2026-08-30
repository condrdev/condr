# Phase 7 qualification evidence

Issue: [#16](https://github.com/bcl-dev/murmur/issues/16)

Product candidate: [`af3d47fc1f3b6ab9a7e92a7a6f20b5e8a6b4167b`](https://github.com/bcl-dev/murmur/commit/af3d47fc1f3b6ab9a7e92a7a6f20b5e8a6b4167b)

Manual capture base: [`dac50f4`](https://github.com/bcl-dev/murmur/commit/dac50f4)

## Status

All automated gates below passed on the exact product candidate. The manual screenshots were captured on `dac50f4`, before the final qualification fix. The only product commit between the capture and the candidate is `af3d47f`; it changes Windows terminal shutdown, Remove Worktree confirmation dispatch, and GUI test stability.

Phase 7 remains open. The Windows system-clipboard copy check could not be completed while the workstation session was locked (`OpenClipboard` access was denied), and the manual screenshots have not been recaptured on the exact final SHA. Neither item is claimed as passed.

## Environments

| Host | Environment |
| --- | --- |
| Windows | Windows 11 Pro for Workstations 10.0.26200, x86_64; Rust 1.95.0 (`x86_64-pc-windows-msvc`); PowerShell 7.6.5; AMD Radeon Graphics driver 31.0.21925.1001; Codex CLI 0.151.0 |
| Linux | Debian 6.1.0-42-arm64, aarch64; Rust 1.98.0 (`aarch64-unknown-linux-gnu`) |

Both hosts resolved the source checkout to `af3d47fc1f3b6ab9a7e92a7a6f20b5e8a6b4167b` for the final automated gate.

## Automated gates

Windows, exact candidate:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace`
- `cargo build --workspace`
- `cargo test -p murmur-gui --features test-support`: 43 passed
- Focused qualification: real GUI worktree scenario 20/20; ConPTY descendant cleanup 26/26

Linux/arm64, exact candidate:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace`
- `cargo test -p murmur-gui --features test-support`: 44 passed

The Linux terminal suite includes delayed-HUP tail preservation, retained-slave cleanup, and reader cleanup coverage.

## Manual evidence

| Acceptance area | Evidence |
| --- | --- |
| Remote endpoint and dual-server connection | [address input](screenshots/03-add-server-address.png), [remote root path](screenshots/11-remote-path-dialog.png), [dual terminals](screenshots/12-dual-server-terminals.png), [remote identity](screenshots/13-remote-identity.png), [local identity](screenshots/14-local-identity.png) |
| Terminal rendering and layout | [Unicode output](screenshots/16-unicode-output.png), [split grid](screenshots/19-split-grid.png), [pane zoom](screenshots/23-pane-zoom.png), [window resize](screenshots/25-window-resize-shell-size.png), [new-tab shortcut](screenshots/31-ctrl-shift-t.png) |
| Agent and interactive shell | [Codex agent](screenshots/32-codex-agent.png), [representative native-output stress](screenshots/38-agent-native-stress.png), [Ctrl-C](screenshots/42-shell-ctrl-c.png) |
| Local Git/worktree lifecycle | [creation dialog](screenshots/49-create-worktree-dialog.png), [created worktree](screenshots/50-worktree-created.png), [dirty removal refused](screenshots/54-dirty-remove-refused.png), [clean removal and branch proof](screenshots/56-clean-remove-branch-proof.png) |
| Remote Git/worktree | [remote worktree](screenshots/61-remote-worktree-created.png) |
| GUI reconnect | [before disconnect](screenshots/64-before-gui-disconnect.png), [after reconnect](screenshots/65-after-gui-reconnect.png), [remote agent reconnected](screenshots/73-remote-agent-reconnected.png) |
| Server stop and restart | [stopped](screenshots/74-remote-server-stopped.png), [restored](screenshots/76-remote-server-restored.png), [restored layout](screenshots/77-remote-restored-layout.png) |
| Parent/worktree ownership | [parent close confirmation](screenshots/79-close-parent-confirm.png), [worktree remains](screenshots/80-parent-closed-worktree-remains.png), [independent close](screenshots/82-worktree-closed-independently.png) |
| Shortcuts and failure atomicity | [shortcut set](screenshots/83-all-shortcuts-complete.png), [failed spawn](screenshots/89-failed-shell-spawn-atomic.png), [original pane remains usable](screenshots/90-failed-spawn-original-usable.png), [all panes closed](screenshots/91-close-all-start-page.png) |

## Open gate

- Complete Windows system-clipboard copy validation in an unlocked interactive session.
- Recapture or explicitly accept the manual evidence on the exact final candidate SHA.

Issue #16 intentionally remains open until those decisions are recorded.
