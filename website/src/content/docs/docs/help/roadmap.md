---
title: Roadmap
description: What Condr builds next and in what order, what waits for more feedback, the longer-term plans, and the directions it will not take.
---

Condr 0.1 is in Public Preview. This page lists what comes next; the order is the order of work, and no dates are promised. To push an item forward or ask for something new, describe your use case in [GitHub Issues](https://github.com/condrdev/condr/issues).

---

## Next

### 1. Diff against the base branch

The Changes sidebar and the Diff Tab compare only against `HEAD`, so once an Agent commits, its changes drop off the list. You will be able to switch to comparing against the base branch and see everything the branch changed since it forked, committed or not. Reviewing a Managed Worktree before you wrap it up no longer means going back to the command line.

### 2. Search in the terminal

Search the current Pane's terminal, including what has scrolled off screen, with matches highlighted and scrolled into view.

### 3. Agent Profiles

The Agent CLIs Condr supports today are all built in. You will be able to describe another Agent CLI in a configuration entry, with its executable name, the arguments that start and resume a session, and its icon, and Condr will recognise and launch it like a built-in Agent. State still comes only from hooks: if the CLI has a hook mechanism, configure it to call `condr agent-hook` as the docs describe and its state shows; a CLI without hooks shows Unknown.

### 4. Mobile apps (iOS and Android)

Away from your computer, see the Workspaces and Agent states on each of your Devices, the Needs you list, and a read-only view of each terminal on your phone. Your phone gets a push notification when an Agent finishes or needs you. The Server encrypts each notification end to end with your phone's public key, so neither the server that relays it nor Apple or Google can read it.

The phone pairs by scanning a QR code and connects over Peer-to-peer or TCP; SSH connections are not supported. Which actions the phone can take is still being designed.

### 5. Port detection and forwarding

When an Agent starts a dev server in a Workspace, Condr shows the ports it listens on, on the Workspace and the Pane. On a Device connected over SSH, clicking a port forwards it to this machine and opens it in your browser.

### 6. GitHub integration

A Workspace shows the GitHub PR for its current branch, with the PR's state and CI check results; clicking opens it in your browser. Enter a PR number or link to check that PR out as a Managed Worktree. Once a PR is merged, Condr suggests removing its worktree but never deletes it on its own.

Creating, merging and reviewing PRs stay with the Agent and `gh`.

---

## Waiting for more feedback

These are under consideration but need more real use cases before they are scheduled. If you need one of them, describe your use case in an Issue.

- **Jump from a Diff Tab to your editor**: open the current file in an external editor straight from the Diff Tab.
- **Scheduled tasks**: start an Agent on a schedule, or send a running Agent a prompt at set times.
- **Port forwarding for TCP and Peer-to-peer Devices**: extend item 5's forwarding to Devices not connected over SSH.

---

## Further out

These will happen, but after everything above, and each waits for its own trigger.

| Item | Starts when |
| :--- | :--- |
| A plugin SDK and an Agent Profile marketplace | The community starts contributing third-party Agent integrations or workflows |
| Team collaboration and permissions | Several people really need to share one Server |
| An editor | Reviewing changes often calls for quick code edits, and Open in cannot open a remote Device's files in a local editor |
| A built-in browser | Once port forwarding ships, switching between the system browser and Condr is still a frequent annoyance |
| A task board | So many tasks run at once that the sidebar and the Needs you list can no longer keep track of them |

---

## Not planned

- **A web client**: Condr is a native app and does not move its control surface into a browser. Peer-to-peer connections and the mobile apps cover working away from your computer.
- **A chat interface of its own**: Condr orchestrates native Agent CLIs. It does not rebuild the conversation loop or tie itself to one model vendor, and leaves the prompt box, model choice and voice to each CLI.
