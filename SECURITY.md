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

## Supported versions

Condr is pre-release. Only the latest `main` and the current development build are supported.
