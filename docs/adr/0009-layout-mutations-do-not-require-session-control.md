# Layout Mutations Do Not Require Session Control

Session control stays exclusive and keeps guarding what it was introduced for: terminal input, resize, selection and the focused Terminal. It is not a lock on the Session's structure. Any client that completed the Bootstrap may send `LayoutCommand`s (create, rename, activate, move, split, close, worktrees), and the Server applies them under the same validation and ordering as before; every subscriber learns the outcome through the ordered event stream, the requester additionally through `LayoutApplied` / `LayoutRejected`.

The reason is the `condr` CLI. Agent orchestration means a program inside a Pane asks the Server to open a Workspace, add a Tab or split a Pane while the GUI is connected and holds control. Gating layout on control would make that impossible, or force the CLI to steal control from the person typing. The GUI already treats structure as Server-owned and projects events from any origin, so a second structural author costs it nothing.

The same applies to a terminal's byte stream. `TerminalCommand::Text`, `Paste` and `Key` are accepted from any client, because prompting an agent in another Pane is the point of the CLI, and `ClientMessage::ReadPane` returns a Pane's recent text to whoever asks. What stays with the controller is the state that describes one viewer: focus, mouse reporting, resize, scroll position and selection.

Consequences: `layout_authority_error` no longer checks the controller, and the terminal handler gates only the viewer-state commands on it. The GUI keeps acquiring control for its terminals. A second GUI without control can now type into a Pane; that is interleaved input, not collaborative editing, which remains deferred as ADR 0003 states.
