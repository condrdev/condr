# Security Policy

## Reporting a vulnerability

Please do not open a public issue for security problems.

Report privately through GitHub: **Security → Report a vulnerability** on this repository
(<https://github.com/condrdev/condr/security/advisories/new>).

Include the Condr build (release name or commit), OS, a reproduction, and the impact you believe it has. You should hear back within 7 days.

## Scope

Anything that lets a party do more than the local user intended, for example:

- bypassing the `Noise_IKpsk2` handshake, invite pairing, or SSH bridge authentication;
- reaching the Server's private socket or TCP endpoint from an unexpected principal;
- escaping the terminal (OSC handling, hook events over OSC 777, clipboard/image bridging) to run code or read data the agent CLI should not;
- leaking credentials, config, or session snapshots.

Bugs in the embedded agent CLIs themselves (Claude Code, Codex, OpenCode, …) belong to their own vendors.

## Peer-to-peer and the Condr relay

Peer-to-peer is Condr's first hosted online service, and it is optional. A Server contacts the relay and DNS only while `[server.p2p] enabled` is set or while it is dialling out for a `p2p://` connection from this machine (it lets go of them a minute after the last such connection closes); a Device that never uses Peer-to-peer contacts nothing of ours. When it is on, be aware of what the Condr relay (`relay.condr.dev`) and DNS (`dns.condr.dev`) can and cannot see:

- **The relay never sees your terminals.** Peer-to-peer traffic is end-to-end encrypted between the two Devices; the relay only helps them hole-punch and, when a direct path fails, forwards opaque encrypted bytes. It leaves the data path once a direct connection succeeds.
- **The relay does see connection metadata.** While it is in the path it observes which Device connects to which, when, and from which IP address. It keeps nothing on disk and runs no accounts.
- **DNS reveals which relay an enabled Server uses, not its address.** A Server with Peer-to-peer enabled publishes its relay URL under its Device id so peers can reach it; it does not publish any IP address. Anyone holding the id can learn which relay it is on.
- **An outage affects only Peer-to-peer.** If the relay or DNS is down, Peer-to-peer connections cannot be established; TCP and SSH are unaffected.

## Condr Cloud and push notifications

Condr Cloud (`push.condr.dev`, a Cloudflare Worker) delivers the mobile app's notifications when an Agent finishes, fails or needs you: a Server asks it to push, and it forwards the push to Apple (APNs) or Google (FCM). It is unrelated to the relay. A phone whose owner declined notifications never contacts it, and turning notifications off on the phone stops pushes.

- **No accounts.** Every request is signed with the caller's Device key, and Condr Cloud checks it against the Device id, which is that key's public half. It issues no credential of its own.
- **What it stores.** For each phone that allows notifications: its Device id, its push token, which Devices may push to it, and when it last registered. It keeps no notification. A registration not refreshed for 90 days is purged, and a token is deleted as soon as Apple or Google reports it invalid.
- **What a push reveals.** Condr Cloud and Cloudflare, which runs it, Apple and Google see the Agent's name and state, the Workspace, the Agent's `detail`, the terminal title, the Server's name and both Device ids. They never see terminal contents.
- **Pushes are not end-to-end encrypted.** They travel over TLS but as plain text, the same text the phone shows on its lock screen.
- **An outage affects only pushes.** Connections to your Devices do not depend on Condr Cloud.

## Supported versions

Condr is pre-release. Only the latest `main` and the current development build are supported.
