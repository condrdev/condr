---
title: Core concepts
description: The small set of ideas behind Condr.
---

## Server owns the runtime

The server is the source of truth for sessions, PTYs, terminal state, and agent lifecycle. Clients connect to it and render the current state.

## Agents stay native

Condr embeds native agent CLIs as child processes. Conversation and tool loops remain in the agent you already trust.

## GUI is a client

The GUI can close and reconnect without interrupting running work.
