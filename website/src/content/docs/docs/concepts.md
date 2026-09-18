---
title: Core concepts
description: The small set of ideas behind Condr.
---

## The Server Owns the Runtime

The Server is the source of truth for workspaces, terminals, and agent state. Clients connect to it and render what it reports. Local and remote clients use the same protocol.

## Agents Stay Native

Condr runs each agent's own CLI as a child process. The conversation and tool loop stay in the agent you already use. Condr only shows its state and gives it a terminal.

## The GUI Is a Client

Closing the window only disconnects. The Server, its terminals, and your agents keep running. Open the window again and it reconnects.
