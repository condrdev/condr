//! The standalone Condr runtime and its local client endpoint.

mod client_writer;
mod endpoint;
pub(crate) mod persistence;
mod server;

pub use endpoint::{Endpoint, EndpointStream, default_socket_path};
pub use server::{
    BoundServer, ClientConnection, ServerConfig, ServerHandle, ensure_local_server, stop_server,
};

/// Binds the configured server and runs until an explicit stop request.
pub fn run(config: ServerConfig) -> std::io::Result<()> {
    BoundServer::bind(config)?.run()
}
