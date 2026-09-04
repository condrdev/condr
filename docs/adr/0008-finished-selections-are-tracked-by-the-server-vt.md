# ADR 0008: Finished selections are tracked by the Server VT

Status: accepted 2026-09-03. Supersedes the "selection remains local to the GUI" sentence of ADR 0004 for finished selections only.

A selection anchored to viewport rows drifts onto other text as soon as the Pane scrolls: an Agent that keeps printing moves the highlight and the copied text away from what the user picked. herdr keeps selection anchors in absolute screen-buffer coordinates and Zed stores the selection inside the alacritty grid, which rotates it with scrolled output; neither exposes a scroll counter Condr could use for a purely Client-side anchor.

Condr therefore splits the interaction. The drag in progress, and every hit test, stays local to the GUI and never round-trips. When the drag ends, or a double/triple click picks a word or line, the Client sends `TerminalCommand::Select` and the Server stores it as the alacritty `Term` selection, which follows output, is cleared on resize and screen swaps, and is dropped by the Server on keyboard input like Alacritty's frontend does. Every full and delta view frame carries that selection clipped to the viewport; the Client paints it and copies through `Copy { selection: None }`, which also covers rows scrolled out of view. A Client that cannot mutate keeps its local selection and the range-based `Copy { selection: Some(_) }` from ADR 0004.

Consequences: the Server holds one selection per Terminal, owned by the controller; a read-only viewer sees the controller's highlight. Selection changes ride the existing visual stream and do not invalidate shaping caches.
