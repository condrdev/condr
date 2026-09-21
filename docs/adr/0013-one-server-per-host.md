# One Server Per Host

A machine runs one Condr Server. It always answers on its private local socket or named pipe, and, when `[server] listen` in the Server's `config.toml` names a TCP address, on that address as well. Both listeners feed the same accept loop and the same Session, so a GUI on the machine, the `condr` CLI inside a Pane, and a paired remote device all see one set of Workspaces. The Server identity, its Snapshot and its log are derived from the socket path alone; the TCP address is an extra door and changes none of them.

Before this decision `condr server start --listen` created a second, TCP-only Server with its own Session, and every later command needed the same `--listen` to find it. Now `condr server start --listen <addr>` writes the address to `config.toml` and starts the host's Server; `start` without arguments, `status`, `stop`, `invite`, `clients` and `revoke` take no endpoint at all. `bridge` also accepts `--endpoint` for forwarding a chosen private socket (ADR 0015). Only `run` accepts both `--endpoint` and a one-off `--listen`, for the detached child `start` spawns and for tests. A `start --listen` against an already running Server records the new address and says a restart is needed to apply it.

Because processes on the host reach the Server through the socket, no device key is authorized by default: the former "host client key" exception is gone, `CONDR_SOCKET_PATH` is always a local path, and administration such as `RevokeDevice` is accepted only on local connections. Pairing, invites and the authorized list work as ADR 0011 describes, for TCP devices only; SSH uses the remote login user’s local socket permissions (ADR 0015). `condr server invite` prints the configured port, or says that no TCP listener is configured yet.

Tests that need TCP use `ServerConfig::ephemeral_tcp`, which binds a temporary socket plus a loopback TCP port with a fresh identity and one authorized test device; `BoundServer::endpoint` hands back the endpoint that device connects to.

> ADR 0026 adds Peer-to-peer as a third door to the same Session, and one invite pairs a Device over TCP and Peer-to-peer alike.
