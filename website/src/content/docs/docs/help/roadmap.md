---
title: Roadmap
description: What Condr builds next and in what order, what waits for more feedback, the longer-term plans, and the directions it will not take.
---

Condr is in Public Preview. This page lists what comes next; the order is the order of work, and no dates are promised. To push an item forward or ask for something new, describe your use case in [GitHub Issues](https://github.com/condrdev/condr/issues).

---

## Next

### Search in the terminal

Search the current Pane's terminal, including what has scrolled off screen, with matches highlighted and scrolled into view.

### Port detection

When an Agent starts a dev server in a Workspace, Condr shows the ports it listens on, on its Pane and on the **Ports** page. On a remote Device, clicking one forwards it to this machine.

### GitHub integration

A Workspace shows the GitHub PR for its current branch, with the PR's state and CI check results; clicking opens it in your browser. Enter a PR number or link to check that PR out as a Managed Worktree. Once a PR is merged, Condr suggests removing its worktree but never deletes it on its own.

Creating, merging and reviewing PRs stay with the Agent and `gh`.

### Worktrees ready to use

A new Managed Worktree can start from any branch, including one that exists only on the remote, and `condr workspace create` can make one and start an Agent in it with a first prompt. A file committed to the repository names the ignored files to copy from the parent, such as `.env`, and the commands that set the worktree up and tear it down; Condr asks you before running them the first time and whenever they change.

### WSL distributions as Devices

On Windows, a WSL 2 distribution connects as a Device of its own, so Agents running in it are recognized and report their state, and Git works on the Linux side. **Connect Remote Device** lists the distributions installed on this machine.

---

## Waiting for more feedback

These are under consideration but need more real use cases before they are scheduled. If you need one of them, describe your use case in an Issue.

- **Jump from a Diff Tab to your editor**: open the current file in an external editor straight from the Diff Tab.
- **Start an Agent from the GUI**: list the Agents installed on a Device and start one in a Pane with a click.
- **Scheduled tasks**: start an Agent on a schedule, or send a running Agent a prompt at set times.

---

## Further out

These will happen, but after everything above, and each waits for its own trigger.

| Item | Starts when |
| :--- | :--- |
| Team collaboration and permissions | Several people really need to share one Server |
| An editor | Reviewing changes often calls for quick code edits, and Open in cannot open a remote Device's files in a local editor |
| A built-in browser | Switching between the system browser and Condr to check forwarded ports is still a frequent annoyance |

---

## Not planned

- **A web client**: Condr is a native app and does not move its control surface into a browser. Peer-to-peer connections and the mobile apps cover working away from your computer.
- **A chat interface of its own**: Condr orchestrates native Agent CLIs. It does not rebuild the conversation loop or tie itself to one model vendor, and leaves the prompt box, model choice and voice to each CLI.
- **Custom Agents**: Condr integrates only the Agent CLIs it officially supports, and offers no way to add another CLI through configuration or a plugin, because a CLI's state can only be trusted through a hook integration written and tested for it. To get a new CLI supported, open an Issue or a pull request.
