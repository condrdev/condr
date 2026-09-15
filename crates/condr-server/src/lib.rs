//! The standalone Condr runtime and its local client endpoint.

mod client;
mod client_writer;
mod endpoint;
pub mod install;
pub mod noise;
pub(crate) mod persistence;
mod server;
pub mod ssh;
#[cfg(test)]
pub(crate) mod test_support;

pub use client::ClientConnection;
pub use endpoint::{
    ConnectionCancellation, Endpoint, EndpointStream, TcpEndpoint, default_socket_path,
};
pub use noise::{PublicKey, Secret, ServerIdentity, StaticKey};
pub use persistence::{read_config_text, update_config_values};
pub use server::{
    BoundServer, ServerConfig, ServerHandle, connected_devices, ensure_local_server, ensure_server,
    ensure_server_from, load_listen, probe_server, restart_server, restart_server_from,
    revoke_devices, save_listen, stop_server, wait_for_shutdown,
};
pub use ssh::SshEndpoint;

/// Binds the configured server and runs until an explicit stop request.
pub fn run(config: ServerConfig) -> std::io::Result<()> {
    BoundServer::bind(config)?.run()
}
