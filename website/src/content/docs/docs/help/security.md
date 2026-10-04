---
title: Security model
description: What Condr opens to the outside by default, whom each connection type trusts, what a paired Device can do, and which network services Condr contacts.
---

Use this page to judge whether a Device is suitable for running Condr, and to see exactly what each connection type grants.

---

## Local only by default

* **Local socket**: a fresh Server listens only on a private local socket. On macOS and Linux the socket's permissions are `0600`, so only your user can connect; on Windows the named pipe admits only your user and SYSTEM.
* **TCP port**: until `[server] listen` is configured, the Server opens no TCP port. That setting is written by `condr server start --listen` or under **Settings › Device › Remote access**.
* **Peer-to-peer access**: until Peer-to-peer is turned on, other Devices cannot connect to this Server over P2P.
* **Local connections have full rights**: any program running as your user can connect to the local socket and do everything, including managing the Server. Programs and Agents running in a Pane have the same rights, and they can use this Device's key to connect to paired remote Devices.

---

## Network services Condr contacts

Condr has no user accounts and collects no telemetry. It contacts only these three external services:

| Service | When | Purpose |
| :--- | :--- | :--- |
| `api.github.com` | 5 seconds after the window starts, then every 5 hours | Checks for updates. Turn automatic checks off under **Settings › Application › About**, or set `[client.updates] auto_check = false` |
| `relay.condr.dev` | When the Server has Peer-to-peer turned on, or this machine is dialling a `p2p://` connection | Helps with NAT hole punching, and relays encrypted traffic when a direct connection cannot be established |
| `dns.condr.dev` | When the Server has Peer-to-peer turned on, or this machine is dialling a `p2p://` connection | Publishes and looks up the relay address each Device currently uses |

---

## Device key

* **One key per Device**: every Device running Condr holds one unique Ed25519 key pair, stored in the `device-key` file in the data directory.
* **Shared by incoming and outgoing connections**: the Server accepting connections and the local window and command line connecting out all use this one key. So each Device has a single fingerprint and takes a single line in the other side's authorized list.
* **Fingerprint format**: the fingerprint is a 43-character base64url string. It appears as the Device key in connection links and as Fingerprint in `condr server status`.
* **Permissions on Linux and macOS**: the file's permissions must be `0600`. If other users have access to it, Condr refuses to load the key and reports `… is readable by other users (mode …); make it 0600 or delete it`, and TCP and Peer-to-peer connections cannot start.
* **Permissions on Windows**: the file admits only its owner and SYSTEM and does not inherit its parent folder's permissions. Condr checks and resets this ACL every time it reads the file.
* **No isolation from your own user**: the key is stored as a plain text file, not in the operating system's keychain. Any local process running as your user can read it.

### Replacing the key

Stop the Server, delete both `device-key` and `authorized-clients` from the data directory, then restart the Server. Condr generates a brand-new key, and every earlier pairing has to be set up again.

If you delete only `device-key`, the window may generate a new key before the Server does, leaving the old authorized list behind.

---

## SSH connections

* **Authentication and encryption**: handled entirely by the local OpenSSH; Condr takes no part in that security logic.
* **Non-interactive mode**: Condr calls the system `ssh` with `BatchMode=yes`, so it never prompts for a password and never asks you to confirm an unknown host key fingerprint. Host key verification follows your existing OpenSSH configuration.
* **Tunnelling**: Condr runs `condr server bridge` on the remote Device, tunnelling the remote Server's private socket to this machine. If the remote Server is not running, `bridge` starts it.
* **Equivalent rights**: the remote Server treats a connection arriving over SSH as a local connection, so a window connected over SSH has every right a local window has, including managing the Server. Being able to log in over SSH already means being able to run any command on that Device, so Condr adds no extra privilege.
* **Connection sharing**: on macOS and Linux, Condr turns on OpenSSH connection sharing by default, keeps the control socket in the runtime directory, and closes it after 60 idle seconds. If your SSH configuration already sets `ControlPath`, Condr leaves it as it is.

---

## TCP connections

* **Handshake protocol**: TCP connections complete a `Noise_IKpsk2_25519_ChaChaPoly_BLAKE2s` handshake (the same family of handshake pattern as WireGuard). Both ends authenticate each other with static keys, and the whole connection is encrypted.
* **How trust is established**: the connection link carries the Server's Device key, and the client starts the handshake only toward that key, so there is no manual fingerprint check on first connection.
* **Paired Devices**: complete the handshake directly and get an encrypted channel.
* **Unpaired Devices**: must present a valid invite. The invite acts as the handshake's pre-shared key (PSK); without it, the client cannot decrypt the Server's handshake reply, and the Server drops the connection.

---

## Invites and pairing

* **The invite**: 32 random bytes, valid for 10 minutes, usable once. Only one invite is valid at any time, and generating a new one invalidates the previous one. It is stored in the `pending-invite` file in the data directory, with the same access permissions as the Device key.
* **Who can use it**: the first Device to complete the handshake uses up the invite. Anyone holding the invite can pair, so share it only through trusted channels.
* **What pairing records**: after pairing, the Server appends a line to the `authorized-clients` file with the other side's Device key, the pairing time, the last-seen time and its host name.
* **What the client keeps**: the client stores only the Server's Device key and network address in `config.toml`, never the invite.

---

## Peer-to-peer connections

* **Shared credentials and authorization**: Peer-to-peer and TCP share the same Device key, invite mechanism and authorized-client list. Revoking a Device applies to both connection types.
* **Encryption**: the two ends connect over QUIC through `iroh` and authenticate each other with TLS 1.3 using their Device keys, so data is end-to-end encrypted. The invite is sent only after the encrypted channel is up.
* **The handshake is open to anyone**: anyone who has this Device's key can complete the basic QUIC handshake, and is only then turned away for not being authorized. Before disconnecting, the refused side can read the Server's build version. There is currently no connection rate limit.
* **Relay service `relay.condr.dev`**: helps the two ends punch through NAT, relays end-to-end encrypted data when a direct connection fails, and leaves the data path once the direct connection works. The relay can see both ends' Device key identities, when the connection was made and the source IP address, but cannot decrypt the traffic. It is operated by Condr, runs the open-source `iroh-relay` without modification, writes nothing to disk and keeps no user accounts.
* **DNS service `dns.condr.dev`**: every 5 minutes, a Server with Peer-to-peer turned on signs its current relay address with its own Device private key and publishes it as a DNS record (the record contains only the relay address, not the Device's real IP). Anyone holding that Device key can look the record up.
* **When these services are contacted**: a Server with Peer-to-peer turned on keeps a long-lived connection to the relay server. A Device that only dials out `p2p://` connections publishes no DNS record, and closes its relay connection 60 seconds after its last active P2P connection ends.

---

## Revoking a Device

Run `condr server revoke <fingerprint or prefix>`, or open **Settings › Device › Paired devices** and click **Revoke**.

* **Takes effect at once**: Condr immediately removes the Device's line from `authorized-clients` and cuts all of its active connections.
* **Later attempts**: a later TCP connection is stopped during the Noise handshake; a Peer-to-peer connection is refused after the QUIC handshake completes.

---

## What a paired Device can do

Pairing currently grants access to the whole Session. A Device connected over TCP or Peer-to-peer can:

* Create, change and close Workspaces, Tabs, Panes and their worktrees.
* Type, paste and copy in terminals (which means running any command as the user the Server runs as).
* Read Pane contents, Git changes and the files in a Workspace, and list the subdirectories of any directory on this Device.
* Start Agent processes, and install Agent hooks through Settings.
* Change the Server's default shell.
* Read the Server's status, including its listen configuration, the fingerprints of connected Devices and recent errors.

These administrative operations are accepted only from local and SSH connections:

* Turning the TCP listener and Peer-to-peer on or off.
* Generating invites, and listing and revoking paired Devices.
* Restarting or stopping the Server.
* Using this Device as a relay hop for Peer-to-peer connections.

So pair only your own Devices with the Server. Read-only and control permissions are not separated yet; sharing one Server among several people has to wait for that feature.

**Viewing only** is not a permission boundary: when several windows connect at once, it only switches the windows that did not get control to a read-only view in their interface.

---

## What programs in a terminal can do

* **Writing the clipboard**: a program running in a Pane can silently overwrite this machine's clipboard with an OSC 52 escape sequence, and so can a program on a remote Device. Every connected window applies the write, with no confirmation prompt. Programs cannot read the system clipboard back.
* **Links**: when you hold Cmd or Ctrl and click a link in the terminal, Condr opens it with the operating system's default handler, with no allowlist of URL schemes. A program can use an OSC 8 escape sequence to print a link whose visible text differs from its real target, so check the target URL before clicking.
* **Agent state can be forged**: any process inside a Pane can construct a fake Agent state event. Agent state is only for display in the interface and must not be used for authentication or security decisions.
* **Images pasted to a remote Device**: when you paste an image on a remote Device, the file is written to a private directory on that Device. On Linux and macOS the directory's permissions are `0700` and the file's are `0600`. The files are removed when the window disconnects or the Server process exits, and leftovers are deleted after 24 hours. On Windows that temporary directory inherits its parent folder's permissions.

---

## File permissions

| File | Linux and macOS | Windows |
| :--- | :--- | :--- |
| `device-key`, `pending-invite` | `0600`; refused when permissions are too broad | Only the owner and SYSTEM |
| `config.toml`, the Session snapshot, the Server's `.stderr` file | `0600` | Inherits the parent folder's ACL |
| `authorized-clients`, log files | Created with the system default umask | Inherits the parent folder's ACL |

For where these files live, see [Configuration and settings](/docs/reference/configuration/).

---

## Reporting security issues

Do not report security flaws in a public Issue. Use GitHub's [Security › Report a vulnerability](https://github.com/condrdev/condr/security/advisories/new) for private disclosure, and include:

* The exact Condr version (a release tag or Git commit hash).
* The operating system.
* Clear steps to reproduce the vulnerability.
* An assessment of its impact.

You will get a reply within 7 days. See [SECURITY.md](https://github.com/condrdev/condr/blob/main/SECURITY.md) for the full disclosure guidelines.
