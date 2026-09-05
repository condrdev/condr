//! The standalone Condr runtime and its local client endpoint.

mod client_writer;
mod endpoint;
pub mod noise;
pub(crate) mod persistence;
pub use persistence::write_config_text;
mod server;

pub use endpoint::{Endpoint, EndpointStream, TcpEndpoint, default_socket_path};
pub use noise::{PublicKey, Secret, ServerIdentity, StaticKey};
pub use server::{
    BoundServer, ClientConnection, ServerConfig, ServerHandle, ensure_local_server, ensure_server,
    load_listen, probe_server, revoke_devices, save_listen, stop_server,
};

/// Binds the configured server and runs until an explicit stop request.
pub fn run(config: ServerConfig) -> std::io::Result<()> {
    BoundServer::bind(config)?.run()
}
