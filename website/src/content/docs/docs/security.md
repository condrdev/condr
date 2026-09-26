---
title: Security model
description: See what Condr exposes by default, whom each connection trusts, and what the relay can see.
---

Use this page to decide whether a machine is safe for Condr and what each connection type allows.

## Keep the default closed

A new Server listens only on a private socket on this machine. On macOS and Linux, the socket has mode 0600, so only your user can connect. On Windows, the named pipe allows only you and SYSTEM.

The Server does not go on the public internet, open a TCP port, or contact a Condr service. Until you run `--listen` or `--p2p`, it is invisible to the network.

Condr has no accounts or telemetry. Its only hosted service is the Peer-to-peer relay. Only a Device with Peer-to-peer enabled contacts it.

## Use one key per Device

Every machine running Condr has one Ed25519 key. It is stored in the `device-key` file in the config folder with mode 0600. If another user can read that file, Condr refuses to start and tells you to fix the mode.

The same key authenticates this machine's Server when it accepts connections and its window and command line when they connect out. A Device therefore has one fingerprint and appears once in an authorized list. The fingerprint is 43 characters of base64url and is the Device key in links.

Deleting `device-key` creates a new key and clears this machine's authorized Device list. Repeat every pairing.

## Trust an SSH connection

Condr does not authenticate or encrypt SSH connections itself. OpenSSH on your machine does both. Condr runs `condr server bridge` on the remote side and forwards the remote Server's private socket to you.

The remote Server treats this connection as local. An SSH-connected window can therefore do everything a local window can, including Server management. SSH access already lets you do anything on that machine. Condr adds no permissions.

## Trust a TCP connection

A TCP connection uses the `Noise_IKpsk2` handshake, from the same family as WireGuard. Both sides authenticate with their static keys, and the connection stays encrypted.

Trust starts with the invite. The link carries the Server's Device key. Your machine encrypts its first message only to that key, so it does not need a “confirm the fingerprint on first connection” step. A paired Device goes straight to the handshake. An unpaired Device must also present a valid invite.

The invite contains 32 random bytes. It expires after 10 minutes and works once. The Server stores it in `pending-invite` in the config folder with mode 0600. The first Device to finish pairing uses it. Anyone holding the invite can pair, so send it only through a trusted channel.

After pairing:

- The Server adds one line to `authorized-clients` with that Device's key, pairing time, last-seen time, and name.
- Your machine records the Server's Device key and address in the configuration file. It does not save the invite.

Revoking a Device removes its line from `authorized-clients`. Its existing connections close immediately, and its next handshake fails.

## Trust a Peer-to-peer connection

Peer-to-peer uses the same keys, invites, and authorized list as TCP. A revocation applies to both types at once.

The Devices encrypt data end to end with their own keys. Two Condr services help set up the connection but cannot see its contents:

- **The relay (`relay.condr.dev`)** helps the Devices hole-punch. When direct routing fails, it forwards encrypted bytes. When direct routing works, it leaves the data path. It can see which Device connected to which, when, and from which IP address. It keeps nothing on disk and has no accounts.
- **DNS (`dns.condr.dev`)** lets other Devices find the relay used by your Server. A Server with Peer-to-peer enabled publishes a signed record under its Device key. The record contains only the relay address, never an IP. Anyone with the Device key can look up the relay.

The Server contacts these services only when Peer-to-peer is enabled in the configuration or this machine dials a `p2p://` connection. One minute after the last dialed connection closes, it disconnects from them. A Device that only dials out publishes no record.

## Understand what a paired Device can do

Pairing currently grants the whole Session. A Device connected over TCP or Peer-to-peer can:

- Create, change, and close Workspaces, Tabs, and Panes.
- Type, paste, and copy in terminals.
- Read Pane contents and Git changes, and browse any folder or file on the Server's machine.
- Start Agents and install Agent hooks.
- Stop the Server.

It cannot manage the Server by turning the listener or Peer-to-peer on or off, generating invites, listing or revoking Devices, or restarting the Server. These actions accept only local and SSH connections.

Pair only your own Devices to a Server. Separate view-only and control levels are not implemented. Sharing one Server with another person waits for that feature.

## Report a security problem

Do not open a public issue. Report privately on GitHub through **Security › Report a vulnerability**. Include the Condr version, operating system, and steps to reproduce. You will hear back within 7 days. See [SECURITY.md](https://github.com/condrdev/condr/blob/main/SECURITY.md) for details.
