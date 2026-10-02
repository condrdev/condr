---
title: Architecture
description: Learn how Condr is put together, so you can read or change its code.
---

If you want to contribute code to Condr, start reading from these three crates and the protocol boundary.

Condr is made of three Rust crates:

- `condr-core`: domain types, the protocol, the PTY and terminal emulation, Agent detection, and Git. It does not depend on the GUI, so it can be tested without a display.
- `condr-server`: builds the `condr` command. It owns the Session, the terminal runtime, persistence and connections.
- `condr-gui`: builds the `condr-gui` app. It is one Client of the Server and renders with GPUI.

Local and remote Clients talk to the Server over the same protocol. Design decisions are recorded in the [ADRs](https://github.com/condrdev/condr/tree/main/docs/adr).
