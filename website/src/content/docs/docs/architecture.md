---
title: Architecture
description: How Condr is shaped under the hood.
---

Condr is split into three crates:

- `condr-core` — domain types, protocol, PTY, VT, agent detection, and Git.
- `condr-server` — sessions, terminal runtime, persistence, and connections.
- `condr-gui` — a native client for rendering and interaction.

The same protocol serves local and remote clients.
