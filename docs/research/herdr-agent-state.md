# Herdr agent recognition and state semantics

Context: [Murmur issue "研究：herdr agent 识别与状态语义基线"](https://github.com/bcl-dev/murmur/issues/8)

Baseline: Herdr `v0.8.2`. All links below point to Herdr's official source or versioned documentation.

## Conclusion

Herdr is terminal-first, not agent-first. A pane is a real terminal; an agent is a foreground process recognized inside that terminal. Any unsupported CLI still runs normally, but rich identity and status require either built-in process recognition plus a screen manifest, or an integration report. Terminal emulation compatibility therefore makes arbitrary CLIs usable, but does not by itself make their status observable ([concepts](https://github.com/herdrdev/herdr/blob/v0.8.2/docs/next/website/src/content/docs/concepts.mdx#L20-L24), [agent support](https://github.com/herdrdev/herdr/blob/v0.8.2/docs/next/website/src/content/docs/agents.mdx#L37-L37)).

For Murmur's first version, inherit this minimum:

1. Open a shell-backed terminal in each pane; do not add an agent launcher.
2. Treat agent identity as optional terminal state, discovered from the foreground process.
3. When a known agent is present, classify its live bottom-buffer snapshot with agent-specific rules.
4. Keep `done` out of the detector; derive it from `idle` plus an unseen-completion flag in the GUI.
5. Defer lifecycle hooks and native agent-session references until Murmur needs better accuracy or conversation restore across process/server restart.

## Pane versus agent

Herdr's public model says a pane is a real terminal and an agent is a recognized process inside a pane. Its server-owned `TerminalState` holds optional `detected_agent`, fallback screen state, hook authority, session reference, and effective state. Pane/view state separately owns whether the result has been seen. Herdr notes that terminal and pane-backed PTY are still one-to-one in `v0.8.2`, even though terminal identity is no longer owned by the view ([concepts](https://github.com/herdrdev/herdr/blob/v0.8.2/docs/next/website/src/content/docs/concepts.mdx#L20-L24), [terminal state](https://github.com/herdrdev/herdr/blob/v0.8.2/src/terminal/state.rs#L115-L150)).

Agent presence and kind are detected before screen classification. Herdr probes the pane's foreground process group, recognizes known executable names (including common runtime wrappers), and only then applies that agent's manifest to screen evidence. No detected agent yields `Unknown` ([process recognition](https://github.com/herdrdev/herdr/blob/v0.8.2/src/detect/mod.rs#L216-L250), [state entry point](https://github.com/herdrdev/herdr/blob/v0.8.2/src/detect/mod.rs#L252-L289), [detection loop](https://github.com/herdrdev/herdr/blob/v0.8.2/src/pane.rs#L746-L849)).

This means Murmur does not need a separate persisted `Agent` entity for the first version. A terminal can carry `Option<AgentKind>` and an effective state; the GUI pane can carry `seen`. A separate agent identity becomes useful only if one conversation must outlive or move independently of its terminal.

## Signal responsibilities

| Signal | Responsibility in Herdr | Murmur first version |
| --- | --- | --- |
| Foreground process | Establish whether the terminal currently hosts a known agent and select its rule set. | Required for rich status; a plain shell or unknown process remains `unknown`. |
| Bottom-buffer snapshot | Default live-state authority for most agents. Match agent-specific TOML rules for `idle`, `working`, and `blocked`. | Required if Murmur promises state badges. |
| OSC title/progress | Additional screen-manifest evidence for agents that emit it; absence does not disable screen rules. | Consume through the terminal emulator when available; do not make it mandatory. |
| Full lifecycle hook/plugin | Exclusive state authority while actively reporting; screen fallback is not run at the same time. | Defer until screen accuracy is insufficient for a supported agent. |
| Session identity report | Store the agent's native conversation reference for restore/resume; it is not state authority for Claude, Codex, and most integrations. | Defer while Murmur neither launches nor resumes agent CLIs. |
| Presentation metadata | Override title/display labels only; waits, notifications, and rollups still use semantic state. | Defer with custom integrations. |

Herdr reads a recent snapshot from the live bottom of the terminal buffer, not the user-scrolled viewport, so scrolling cannot change status. Manifests evaluate explicit regions and prioritized evidence; transcript/history viewers can request that the current state be preserved instead of reclassified ([screen authority](https://github.com/herdrdev/herdr/blob/v0.8.2/docs/next/website/src/content/docs/agents.mdx#L39-L49), [bottom-buffer invariant](https://github.com/herdrdev/herdr/blob/v0.8.2/src/pane/terminal.rs#L5147-L5163), [manifest engine](https://github.com/herdrdev/herdr/blob/v0.8.2/src/detect/manifest.rs#L415-L495)).

Complete lifecycle integrations report semantic state directly. While one is live, Herdr makes it authoritative and skips parallel screen classification, avoiding competing sources of truth. Session-only integrations instead report a native session id/path while state remains screen-derived; Herdr documents Claude and Codex in this category ([authority table](https://github.com/herdrdev/herdr/blob/v0.8.2/docs/next/website/src/content/docs/integrations.mdx#L54-L63), [Claude and Codex roles](https://github.com/herdrdev/herdr/blob/v0.8.2/docs/next/website/src/content/docs/integrations.mdx#L130-L152), [screen-scan suppression](https://github.com/herdrdev/herdr/blob/v0.8.2/src/pane.rs#L857-L879)).

Native session references exist to reconstruct supported agent conversations after the original processes are gone. They require official integrations and known resume commands. They are unnecessary for detach/reattach while the original PTY process remains alive, and unnecessary for Murmur's clarified terminal-only launch flow ([restore behavior](https://github.com/herdrdev/herdr/blob/v0.8.2/docs/next/website/src/content/docs/session-state.mdx#L29-L37), [native agent restore](https://github.com/herdrdev/herdr/blob/v0.8.2/docs/next/website/src/content/docs/session-state.mdx#L48-L89)).

## State semantics

Herdr's detector has exactly four states:

| Detector state | Meaning |
| --- | --- |
| `working` | The recognized agent is actively processing. |
| `blocked` | The recognized agent needs human input, approval, or a decision. Screen-based detection intentionally requires known visible UI evidence. |
| `idle` | The recognized agent is at a prompt, finished, or waiting. For a known agent, no matching manifest rule also deliberately falls back to `idle`. |
| `unknown` | No recognized agent is present, or Herdr cannot confidently classify the current state. |

These meanings are defined in the detector and user documentation. The conservative asymmetry is intentional: a novel blocker may temporarily appear idle rather than blocked, and that error affects display and waits only ([detector enum](https://github.com/herdrdev/herdr/blob/v0.8.2/src/detect/mod.rs#L9-L20), [blocked policy](https://github.com/herdrdev/herdr/blob/v0.8.2/docs/next/website/src/content/docs/agents.mdx#L57-L61), [known-agent fallback](https://github.com/herdrdev/herdr/blob/v0.8.2/src/detect/manifest.rs#L498-L552)).

`done` is not a detector state. Herdr exposes it when the semantic state is `idle` and the pane's completion has not been seen; once seen, the same semantic state is presented as `idle`. A transition from `working` or `blocked` to `idle` marks an unfocused completion unseen, and viewing the tab marks it seen ([status projection](https://github.com/herdrdev/herdr/blob/v0.8.2/src/app/api_helpers.rs#L96-L106), [completion/seen transition](https://github.com/herdrdev/herdr/blob/v0.8.2/src/app/actions.rs#L3095-L3125), [mark seen](https://github.com/herdrdev/herdr/blob/v0.8.2/src/app/actions.rs#L1277-L1297)).

The resulting minimal Murmur state shape is therefore:

```text
terminal: agent_kind? + semantic_state(unknown|idle|working|blocked)
pane UI: seen
display: idle && !seen => done; otherwise semantic_state
```

## Scope boundary

Do not inherit Herdr's managed `agent start`, agent naming, native conversation resume, integration installer, remote manifest updater, or custom metadata API into the first version. None is required to open a terminal and manually run any agent CLI. Add them only when Murmur explicitly promises automation, restart-time conversation recovery, or status accuracy that screen manifests cannot provide.
