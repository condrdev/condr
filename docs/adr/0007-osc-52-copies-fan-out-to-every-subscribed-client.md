# OSC 52 Copies Fan Out to Every Subscribed Client

The Server's VT accepts OSC 52 in copy direction only (`Osc52::OnlyCopy`). A program running in a Pane can fill the user's clipboard; it can never read it. The paste direction (`ClipboardLoad`) stays disabled because it lets a Terminal program exfiltrate whatever the user last copied, and no Condr workflow needs it.

The clipboard is a client resource while OSC 52 is a Server-side VT event, so each copy crosses the protocol as `ServerMessage::TerminalClipboard { pane_id, text }`. It is a reliable message without a Session sequence and is not retained in event history: a reconnecting client must not have its clipboard rewritten by a replayed copy. The Server delivers it to every subscriber that already holds its Bootstrap, without confirmation and without a per-Pane or per-connection switch. This mirrors tmux `set-clipboard on`, which forwards OSC 52 to every attached client, and matches what the user sees: all attached clients show the same Pane, so the same copy lands in each of their clipboards. Restricting delivery to the active controller would make a copy silently vanish for a viewer who has not acquired control.

A remote Server is trusted to the same degree as the programs it runs; a Condr user who attaches to a Server has already accepted that those programs act on their behalf. A Terminal retains only the latest pending copy, including an empty clipboard clear; payloads larger than `MAX_PENDING_CLIPBOARD_BYTES` are dropped. Each client writer likewise keeps one replaceable copy slot, so a burst converges on the final clipboard state without delaying control or terminal traffic.

A configuration switch to disable or confirm clipboard writes is deferred until a concrete need appears.
