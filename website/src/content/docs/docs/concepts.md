---
title: Core concepts
description: Learn the Condr terms and the three ideas that shape how it works.
---

Use these terms to understand the sidebar and the rest of the Condr documentation.

## Learn the terms

- **Device**: a machine that runs Condr. The sidebar has one entry for each Device, including your own machine.
- **Server**: the `condr` process on a Device. It owns everything below.
- **Workspace**: a project folder and the terminals you open in it.
- **Tab**: one screen inside a Workspace. A Tab contains one or more Panes.
- **Pane**: one terminal. Split a Tab to create more Panes.
- **Agent**: a program in a Pane that Condr recognizes, such as Claude Code or Codex. Condr shows its state in the sidebar.

## Let the Server own the runtime

The Server is the source of truth for Workspaces, terminals, and Agent state. Clients connect to it and display what it reports. Local and remote Clients use the same protocol, so they behave the same way over a network.

## Keep Agents native

Condr runs each Agent's own CLI as a child process. The Agent keeps its conversation and tool calls. Condr provides the terminal and shows the Agent's activity.

## Treat the window as a Client

Closing the window only disconnects it. The Server, its terminals, and your Agents keep running. Open the window again to reconnect to them.
