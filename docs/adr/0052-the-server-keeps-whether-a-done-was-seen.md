# The Server Keeps Whether a Done Was Seen

Status: accepted 2026-10-28. Supersedes the per-Client Done of ADR 0041 and ADR 0044; the Agent's own `Done` of ADR 0044 stands.

ADR 0044 made `Done` a state the Agent reports and gave each Client one bit of its own: whether it had looked. That bit lived only in the Client's memory. Claude Code reports `done` at the end of a turn and keeps it until the next one, so every restart of the GUI or the Companion forgot every Done the person had already looked at, and the sidebar came back full of Done marks.

The Server keeps the set of Panes whose `Done` no Client has looked at. A `Done` that arrives while no Client has the Pane focused joins it; one that arrives while a Client has it focused does not. Any Client focusing the Pane removes it, through the `TerminalCommand::Focus(true)` both the GUI and the Companion already report, the way it acknowledges a bell (ADR 0036). The Agent moving to any other state removes it too, and so does a new Terminal in the Pane. A repeated `Done` keeps what it had. The set is not persisted: a Server restart starts its Agents again.

`PaneAgentSnapshot` carries `unseen` in the Bootstrap, and `SessionEvent::AgentUnseenChanged { pane_id, unseen }` follows each change after the `AgentChanged` that caused it. A Client shows `Done` for an Agent in `Done` that is unseen, else `Idle` (`AgentDisplayState::of`). It clears the bit locally as soon as it focuses a Pane, ahead of the Server's answer, and ignores `unseen` for the Pane it has in front of the person, since the Server's focus is only whoever reported it last.

What is given up is ADR 0041's Done per Client: an Agent watched finishing on the desktop now shows as Idle on the phone too, as a bell already does. Keeping it per Client across restarts would have needed each Client to store what it saw against a revision of the Agent's state, in the GUI's state file and the Companion's storage alike, and the Server to send that revision.

`PROTOCOL_VERSION` stays `1`, the project being unreleased.

Tests: the Server marks a Done that arrives unfocused, clears it on another Client's focus, leaves a Done arriving in the focused Pane seen, clears it when the Agent moves on, and reports it in the Bootstrap; the event and the Bootstrap record round-trip; the display mapping; the GUI's Done turning Idle once its Pane is opened.
