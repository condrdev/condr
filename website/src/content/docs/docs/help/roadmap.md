---
title: Roadmap
description: What Condr builds next and in what order, what waits for more feedback, the longer-term plans, and the directions it will not take.
---

Condr 0.1 is in Public Preview. This page lists what comes next; the order is the order of work, and no dates are promised. To push an item forward or ask for something new, describe your use case in [GitHub Issues](https://github.com/condrdev/condr/issues).

---

## Next

### 1. More officially supported Agent CLIs

Condr will officially support more of the common Agent CLIs. Settings and the docs will show how far each Agent is supported. A fully supported Agent shows Working and Idle reliably. A recognition-only Agent shows its icon in the sidebar but no state. If a CLI you use is not on the list yet, open an Issue or a pull request.

### 2. Search in the terminal

Search the current Pane's terminal, including what has scrolled off screen, with matches highlighted and scrolled into view.

### 3. Mobile apps (iOS and Android)

Away from your computer, see the Workspaces and Agent states on each of your Devices, the Needs you list, and a read-only view of each terminal on your phone. Your phone gets a push notification when an Agent finishes or needs you. The Server encrypts each notification end to end with your phone's public key, so neither the server that relays it nor Apple or Google can read it.

The phone pairs by scanning a QR code and connects over Peer-to-peer or TCP; SSH connections are not supported. Which actions the phone can take is still being designed.

### 4. Port detection and forwarding

When an Agent starts a dev server in a Workspace, Condr shows the ports it listens on, on the Workspace and the Pane. On a Device connected over SSH, clicking a port forwards it to this machine and opens it in your browser.

### 5. GitHub integration

A Workspace shows the GitHub PR for its current branch, with the PR's state and CI check results; clicking opens it in your browser. Enter a PR number or link to check that PR out as a Managed Worktree. Once a PR is merged, Condr suggests removing its worktree but never deletes it on its own.

Creating, merging and reviewing PRs stay with the Agent and `gh`.

---

## Waiting for more feedback

These are under consideration but need more real use cases before they are scheduled. If you need one of them, describe your use case in an Issue.

- **Jump from a Diff Tab to your editor**: open the current file in an external editor straight from the Diff Tab.
- **Start an Agent from the GUI**: list the Agents installed on a Device and start one in a Pane with a click.
- **Scheduled tasks**: start an Agent on a schedule, or send a running Agent a prompt at set times.
- **Port forwarding for TCP and Peer-to-peer Devices**: extend item 4's forwarding to Devices not connected over SSH.

---

## Further out

These will happen, but after everything above, and each waits for its own trigger.

| Item | Starts when |
| :--- | :--- |
| Team collaboration and permissions | Several people really need to share one Server |
| An editor | Reviewing changes often calls for quick code edits, and Open in cannot open a remote Device's files in a local editor |
| A built-in browser | Once port forwarding ships, switching between the system browser and Condr is still a frequent annoyance |
| A task board | So many tasks run at once that the sidebar and the Needs you list can no longer keep track of them |

---

## Not planned

- **A web client**: Condr is a native app and does not move its control surface into a browser. Peer-to-peer connections and the mobile apps cover working away from your computer.
- **A chat interface of its own**: Condr orchestrates native Agent CLIs. It does not rebuild the conversation loop or tie itself to one model vendor, and leaves the prompt box, model choice and voice to each CLI.
- **Custom Agents**: Condr integrates only the Agent CLIs it officially supports, and offers no way to add another CLI through configuration or a plugin, because a CLI's state can only be trusted through a hook integration written and tested for it. To get a new CLI supported, open an Issue or a pull request.
