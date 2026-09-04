# Layout Mutations Do Not Require Session Control

Session control stays exclusive and keeps guarding what it was introduced for: terminal input, resize, selection and the focused Terminal. It is not a lock on the Session's structure. Any client that completed the Bootstrap may send `LayoutCommand`s (create, rename, activate, move, split, close, worktrees), and the Server applies them under the same validation and ordering as before; every subscriber learns the outcome through the ordered event stream, the requester additionally through `LayoutApplied` / `LayoutRejected`.

The reason is the `condr` CLI. Agent orchestration means a program inside a Pane asks the Server to open a Workspace, add a Tab or split a Pane while the GUI is connected and holds control. Gating layout on control would make that impossible, or force the CLI to steal control from the person typing. The GUI already treats structure as Server-owned and projects events from any origin, so a second structural author costs it nothing.

Consequences: `layout_authority_error` no longer checks the controller. The GUI keeps acquiring control for its terminals, and a read-only viewer still cannot type, but it can rearrange the layout. Collaborative editing of terminal input remains deferred, as ADR 0003 states.
