---
title: Connect a remote Device
description: Choose SSH, TCP, or Peer-to-peer, then connect and manage another machine.
---

Add another machine to the sidebar, choose its connection type, and recover when it disconnects or versions differ. For trust rules, read the [security model](/docs/security/). For connection checks, read [troubleshooting](/docs/troubleshooting/).

A **Device key** identifies one Device and has 43 characters. An **invite** is a one-time invitation. Keep it secret. It expires after 10 minutes.

## Choose a connection

Install `condr` on the remote Device first. See [Install](/docs/install/). Then choose the connection that matches your setup:

| Your situation | Pick | Link form |
| --- | --- | --- |
| You can already log in to the Device with SSH | SSH | `ssh://user@host` |
| The Device has an IP you can reach, such as a server on your network | TCP | `tcp://<device key>.<invite>@host:port` |
| Both machines sit behind routers and neither has a public address | Peer-to-peer | `p2p://<device key>.<invite>` |

For all three, click **Connect Remote Device** in the sidebar and paste the link. A connected remote Device works like your own machine with the same window, Workspaces, and Agent states.

## Connect over SSH

```text
ssh://user@build-box
```

Condr passes this address unchanged to OpenSSH on your machine. Aliases, ports, keys, and jump hosts in `~/.ssh/config` therefore apply. Add a port with `ssh://user@host:2222`.

When connecting, Condr runs `condr server bridge` on the remote Device. If no Server runs there, Condr starts one.

Prepare these items:

- **Passwordless login.** Condr connects with `BatchMode`, so it never prompts for a password. Set up keys, ssh-agent, and host trust in your terminal first.
- **`condr` on the remote PATH.** A non-interactive SSH shell usually does not read `~/.profile` or `~/.zprofile`, where the install script adds PATH. If the connection reports `condr: not found`, put the path in the link:

  ```text
  ssh://user@host?bin=/home/user/.local/bin/condr
  ```

- **Hosts with a second factor.** Log in once by hand in a terminal and keep that session open. Condr reuses it.

On macOS and Linux, Condr reuses SSH connections automatically with `ControlMaster`, so later calls do not authenticate again. If your SSH config already sets `ControlPath`, Condr uses it and adds nothing. Windows has no connection sharing, so every call opens a new SSH connection. A Windows machine as the remote Device is not supported yet.

## Connect over TCP

```text
tcp://<device key>.<invite>@<host>:<port>
```

The Server listens only on the local machine until you enable a listener. On the remote Device:

1. Start the Server with a listen address. Use an IP and port, not a host name. Condr writes the address to the configuration, so later starts use it. If the Server already runs, use `restart` instead of `start`:

   ```sh
   condr server start --listen 0.0.0.0:2637
   ```

2. Create an invite:

   ```sh
   condr server invite
   ```

   The command prints a TCP link. Replace its `<host>` placeholder with this Device's IP.

3. Within 10 minutes, open Condr on your work computer, click **Connect Remote Device**, and paste the link. Repeat step 2 if it expires.

An invite works once. The first Device that finishes pairing uses it. Creating another invite replaces the old one. After pairing, Condr saves only the Device key and address, not the invite.

## Connect over Peer-to-peer

```text
p2p://<device key>.<invite>
```

Use this for two machines behind routers, such as a home desktop and a work laptop. You need no fixed IP or VPN.

1. Start the Server with Peer-to-peer enabled. Condr writes this setting to the configuration. If the Server already runs, use `restart`:

   ```sh
   condr server start --p2p
   ```

2. Run `condr server invite`. It prints a Peer-to-peer link without a placeholder.
3. Paste the link into Condr within 10 minutes.

The Devices connect directly when possible and use Condr's relay when not. Their own keys encrypt traffic end to end, so the relay cannot read it. If the relay or DNS service is down, only Peer-to-peer connections fail. SSH and TCP continue to work. Read the [security model](/docs/security/) for what the relay can see.

A Peer-to-peer Device's default sidebar name is `p2p` plus the first 8 characters of its Device key. Right-click the Device and choose **Edit Remote Device** to rename it.

## Switch between Devices

The sidebar groups Workspaces under each Device. Click a Workspace to switch to it.

The mark on a Device heading shows its connection state:

- Spinner: connecting.
- Red alert: cannot connect. Hover for the reason, then click to reconnect.
- Yellow triangle: connected, but the Condr builds differ. Hover to see which side to update, then click to dismiss.

When a connection drops, Condr waits 2 seconds before showing anything. It then shows "Reconnecting to …" above the Workspace and retries every half second for 45 seconds. It stops after that and shows **Disconnected**. Click **Connect** to reconnect by hand. When the computer wakes from sleep, Condr probes every Device and starts reconnecting to one that does not answer within 10 seconds.

A Device that fails to connect when Condr starts is not retried automatically. Click **Connect** on that Device's page.

## Manage paired Devices

The Server records Devices paired over TCP and Peer-to-peer. On the Server machine, list or revoke them:

```sh
condr server clients
condr server revoke <fingerprint or a prefix of it>
```

`clients` lists each Device's name, last connection time, and Device key. `revoke` disconnects the Device immediately, and it needs a new invite to pair again. The prefix is case-sensitive and must match exactly one Device.

In Condr, use **Settings › Device › Paired devices** for the same actions. **Generate invite** creates and copies a link. Each row has **Revoke**.

Only local and SSH connections can manage the Server. They can create invites, turn listening and Peer-to-peer on or off, revoke Devices, and restart the Server. A window connected over TCP or Peer-to-peer can use Workspaces and Agents, but these settings are read-only.

## Resolve version differences

During connection, Condr exchanges protocol versions and build numbers:

- **Compatible protocol, different build.** Everything works and the sidebar shows a yellow triangle. Update both sides to the same version and restart the Server to remove the mark.
- **Incompatible protocol.** The connection is refused and the window says "speak different protocol versions". Update both sides to the same version and connect again.

After updating a remote Device, restart its Server. Run `condr server restart` there, or click **Restart Condr** in **Settings › Device › Daemon**.

## Can't connect

| Message | Most likely cause |
| --- | --- |
| SSH could not reach the device | SSH itself cannot connect, or the remote has no `condr` |
| SSH reached the device, but Condr is not running there | The remote Server is not running and could not be started |
| this device is not authorized | The invite expired, was used, or this Device was revoked |
| speak different protocol versions | The versions are too far apart |

See [troubleshooting](/docs/troubleshooting/) for each check.

## Reach a remote Device from the command line

Saved Devices are available on the command line:

```sh
condr device list
condr --device build-box workspace list
condr workspace list --all-devices
```

`--device` takes the Device name shown in the sidebar. Set the default with `CONDR_DEVICE`. `server` and `agent hooks` act only on this machine and do not accept `--device`. See [Agent automation](/docs/automation/) for details.
