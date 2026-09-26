---
title: Architecture
description: Learn how Condr is built if you want to read or change the code.
---

Use this page to find the three crates and the protocol boundary when you contribute to Condr.

Condr has three Rust crates:

- `condr-core`: domain types, protocol, PTY and terminal emulation, Agent detection, and Git. It has no GUI dependencies, so you can test it without a display.
- `condr-server`: builds the `condr` command. It owns the Session, terminal runtime, persistence, and connections.
- `condr-gui`: builds the `condr-gui` app. It is a Server Client and renders with GPUI.

Local and remote Clients use the same protocol to speak to the Server. The design decisions are recorded in the [ADRs](https://github.com/condrdev/condr/tree/main/docs/adr).
