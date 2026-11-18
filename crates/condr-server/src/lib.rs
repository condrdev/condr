//! The standalone Condr runtime and its local client endpoint.

pub mod autostart;
mod client;
mod client_writer;
mod endpoint;
pub mod install;
pub mod logging;
#[cfg(unix)]
pub mod login_environment;
pub mod noise;
pub mod p2p;
pub(crate) mod persistence;
mod poison;
mod server;
pub mod ssh;
#[cfg(feature = "tasks")]
pub(crate) mod store;
pub mod supervisor;
#[cfg(feature = "tasks")]
pub(crate) mod task_store;
#[cfg(test)]
pub(crate) mod test_support;

pub use client::{ClientConnection, Refused};
pub use endpoint::{
    ConnectionCancellation, Endpoint, EndpointStream, P2pEndpoint, SavedServer, TcpEndpoint,
    default_socket_path, load_saved_servers, save_saved_servers,
};
pub use noise::{DeviceKey, PublicKey, Secret, ServerIdentity};
pub use server::{
    BoundServer, ServerConfig, ServerHandle, ensure_local_server, ensure_server,
    ensure_server_from, load_listen, load_p2p, not_running, probe_server,
    resolve_server_executable, restart_server, restart_server_from, save_listen, save_p2p,
    server_admin, server_status, set_server_executable, stop_server, wait_for_shutdown,
};
pub use ssh::SshEndpoint;

/// Binds the configured server and runs until an explicit stop request.
pub fn run(config: ServerConfig) -> std::io::Result<()> {
    BoundServer::bind(config)?.run()
}
