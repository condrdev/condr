---
title: Why Condr
description: Decide whether Condr fits your workflow and see how it differs from other tools.
---

Use this page to decide whether Condr fits the way you run Agents.

## Problems Condr helps you solve

- Three terminal windows run three Agents, and you cannot tell which one waits for approval.
- You close the laptop, and the Agent loses the task it was running.
- You run Agents on an office server with tmux, but cannot see that work from your phone.
- Each Agent vendor has its own remote control, but you use more than one vendor.

## Keep the task running after you close the window

- The Server stays running while the window acts as a view. Agents keep going after you close the window or lose the network.
- Open the window again and pick up where you stopped. The sidebar shows which Agents finished while you were away.
- Each machine runs one Server. Local and remote work use the same model.

## Use the Agents you already have

- Condr runs vendors' own CLIs: Claude Code, Codex, OpenCode, and seven others.
- Condr does not change conversations, proxy model calls, or touch subscriptions and API keys.
- Each Pane is a real terminal, not a chat box. Any terminal program can run there.

## See who needs you

- Agent hooks report state. Condr does not infer it from screen text.
- When an Agent waits for approval, the sidebar shows the command or question.
- One list at the top of the sidebar gathers Agents waiting on every Device.
- A system notification tells you when an Agent finishes or needs you.

## Use one window for every machine

- Connect with SSH, TCP, or Peer-to-peer.
- Two machines behind routers can connect directly without a fixed IP or VPN.
- Every connection is encrypted. TCP and Peer-to-peer use a WireGuard-style handshake and a one-time invite for pairing.

## Use a native desktop app

- Condr starts fast and stays responsive while Agents stream output. The installer is small.
- macOS, Windows, and Linux use the same product. None is a port.
- Condr is written in Rust and rendered with GPUI. It uses no Electron or webview.

## Compare other approaches

| Approach | Good at | How Condr differs |
| --- | --- | --- |
| tmux + ssh | Available everywhere and fast on the keyboard | A graphical interface with a mouse, visible Agent state, and no connection setup of your own |
| Herdr | An Agent runtime in the terminal with a similar design | A native graphical interface, Windows as a first-class platform, built-in encrypted pairing, and a Git sidebar |
| A vendor's own remote control | Deep integration with its own Agent | Works across vendors and puts several machines in one window |
| Agent orchestration frameworks | Complex multi-Agent workflows | Does not orchestrate for you. Agents call each other with the `condr` command |

Condr's Agent detection and layout semantics borrow from Herdr.

## Know what Condr is not

- Not an Agent. It does not talk to models.
- Not an editor. It shows diffs and files, but does not change them.
- Not a model proxy. It never handles your requests or keys.
- Not a web app. There is no browser version or account.
