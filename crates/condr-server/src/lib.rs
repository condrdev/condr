//! The standalone Condr runtime and its local client endpoint.

mod client;
mod client_writer;
mod endpoint;
pub mod install;
pub mod logging;
pub mod noise;
pub mod p2p;
pub(crate) mod persistence;
mod server;
pub mod ssh;
#[cfg(test)]
pub(crate) mod test_support;

pub use client::ClientConnection;
pub use endpoint::{
    P2pEndpoint,
    ConnectionCancellation, Endpoint, EndpointStream, SavedServer, TcpEndpoint,
    default_socket_path, load_saved_servers, save_saved_servers,
};
pub use noise::{DeviceKey, PublicKey, Secret, ServerIdentity};
pub use server::{
    BoundServer, ServerConfig, ServerHandle, connected_devices, ensure_local_server, ensure_server,
    ensure_server_from, load_listen, load_p2p, probe_server, restart_server,
    restart_server_from, revoke_devices, save_listen, save_p2p, server_status, stop_server,
    wait_for_shutdown,
};
pub use ssh::SshEndpoint;

/// Binds the configured server and runs until an explicit stop request.
pub fn run(config: ServerConfig) -> std::io::Result<()> {
    BoundServer::bind(config)?.run()
}
