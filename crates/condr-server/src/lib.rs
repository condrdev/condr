//! The standalone Condr runtime and its local client endpoint.

mod client_writer;
mod endpoint;
pub mod noise;
pub(crate) mod persistence;
pub use persistence::{read_config_text, update_config_values};
mod server;
pub mod ssh;

pub use endpoint::{
    ConnectionCancellation, Endpoint, EndpointStream, TcpEndpoint, default_socket_path,
};
pub use noise::{PublicKey, Secret, ServerIdentity, StaticKey};
pub use server::{
    BoundServer, ClientConnection, ServerConfig, ServerHandle, connected_devices,
    ensure_local_server, ensure_server, load_listen, probe_server, restart_server, revoke_devices,
    save_listen, stop_server,
};
pub use ssh::SshEndpoint;

/// Binds the configured server and runs until an explicit stop request.
pub fn run(config: ServerConfig) -> std::io::Result<()> {
    BoundServer::bind(config)?.run()
}
