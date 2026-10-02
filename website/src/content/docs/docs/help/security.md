---
title: Security model
description: See what Condr exposes by default, whom each kind of connection trusts, and what the relay can see.
---

Use this page to decide whether a machine is a good fit for Condr, and to learn what each kind of connection allows.

## Closed by default

A newly installed Server listens only on a private socket on this machine. On macOS and Linux the socket's permissions are 0600, so only your user can connect. On Windows the named pipe allows only you and SYSTEM.

The Server does not go on the public internet, opens no TCP port, and does not contact any Condr service. Until you run `--listen` or `--p2p`, it is invisible to the network.

Condr has no accounts and no telemetry. Its only hosted service is the Peer-to-peer relay, and only Devices with Peer-to-peer turned on contact it.

## One key per Device

Every machine that runs Condr has one Ed25519 key, stored in the `device-key` file in the config directory with permissions 0600. If other users can read it, Condr refuses to start and tells you to fix the permissions.

The same key is used when this machine's Server accepts connections and when its window and command line connect out. So a Device has exactly one fingerprint and appears only once in an authorized list. The fingerprint is 43 characters of base64url, and it is also the Device key in a link.

Deleting `device-key` generates a new key and clears this machine's list of authorized Devices. Every pairing has to be redone.

## Who an SSH connection trusts

Condr does not handle authentication or encryption for SSH connections. It leaves both to OpenSSH on this machine. Condr runs `condr server bridge` on the remote machine and forwards the remote Server's private socket to this machine.

The remote Server treats this connection as a local one. So a window connected over SSH can do everything a local window can, including managing the Server. Anyone who can log in to a machine over SSH can already do anything there, so Condr grants no new privileges.

## Who a TCP connection trusts

A TCP connection uses the `Noise_IKpsk2` handshake, which belongs to the same family as WireGuard. Each side authenticates with its own static key, and the connection is encrypted throughout.

Trust starts with an invite. The link carries the Server's Device key, and this machine encrypts its first message only to that key, so there is no "confirm the fingerprint on first connect" step. A paired Device goes straight into the handshake, while an unpaired Device must also present a valid invite.

An invite holds 32 random bytes, expires after 10 minutes, and can be used only once. The Server keeps it in the `pending-invite` file in the config directory with permissions 0600. The first Device to finish pairing uses it up. Anyone holding the invite can pair, so send it only over a channel you trust.

After pairing:

- The Server adds a line to `authorized-clients` that records the Device's key, pairing time, last connection time and name.
- This machine records the Server's Device key and address in its config file and does not keep the invite.

Revoking a Device removes its line from `authorized-clients`. Its existing connections close immediately, and its next handshake fails.

## Who a Peer-to-peer connection trusts

Peer-to-peer uses the same keys, invites and authorized list as TCP. Revoking a Device applies to both kinds of connection.

The two Devices encrypt data end to end with their own keys. Two Condr services take part in setting up the connection, but they cannot see its contents:

- **The relay (`relay.condr.dev`)** helps Devices punch holes through NAT. When a direct connection fails, it forwards the encrypted bytes, and once a direct connection succeeds, it leaves the data path. It can see which Device connects to which Device, when, and from which IP. It stores no data on disk and has no accounts.
- **DNS (`dns.condr.dev`)** lets other Devices find the relay a Server uses. A Server with Peer-to-peer turned on publishes a signed record under its own Device key. The record holds only the relay address, not an IP. Anyone who holds the Device key can look up which relay it is on.

The Server contacts these two services only when its config turns Peer-to-peer on, or while this machine is dialing out a `p2p://` connection. It disconnects one minute after the last outgoing connection closes. A Device that only dials out publishes no record.

## What a paired Device can do

Pairing currently grants the whole Session. A Device connected over TCP or Peer-to-peer can:

- Create, change and close Workspaces, Tabs and Panes.
- Type, paste and copy in terminals.
- Read Pane contents and Git changes, and browse any directory or file on the Server's machine.
- Start Agents and install Agent hooks.
- Stop the Server.

It cannot manage the Server, which covers turning listening and Peer-to-peer on or off, creating invites, listing and revoking Devices, and restarting the Server. These operations accept only local and SSH connections.

So pair only your own Devices with a Server. Read-only and control permissions are not separated yet, and sharing a Server with other people has to wait for that feature.

## Report a security problem

Do not open a public issue. Report it privately on GitHub through **Security › Report a vulnerability**, and include your Condr version, operating system and steps to reproduce. You will get a reply within 7 days. See [SECURITY.md](https://github.com/condrdev/condr/blob/main/SECURITY.md) for details.
