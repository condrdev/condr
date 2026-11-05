//! The Server's end of a Stream connection (ADR 0061): after `Welcome` the connection
//! carries Streams, and each `Open` is answered here. A forward reaches this machine's own
//! loopback and nothing else.

use condr_client::{Stream, StreamConnection, isolated, splice};
use condr_core::protocol::{StreamError, StreamTarget};
use std::io;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::endpoint::EndpointStream;

/// A loopback connect either answers or is refused at once; this only bounds a port
/// whose listener stopped accepting.
const LOOPBACK_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// Serves one Stream connection until it ends.
pub(super) fn serve(stream: EndpointStream, client_name: &str) {
    let connection = match StreamConnection::serve(stream, open) {
        Ok(connection) => connection,
        Err(error) => {
            tracing::warn!(name = %client_name, "stream connection failed to start: {error}");
            return;
        }
    };
    tracing::info!(name = %client_name, "stream connection opened");
    let reason = connection.wait_closed();
    tracing::info!(name = %client_name, %reason, "stream connection closed");
}

fn open(stream: Stream, target: StreamTarget) {
    match target {
        StreamTarget::Forward { port } => match u16::try_from(port) {
            Ok(port) if port != 0 => {
                tracing::debug!(stream = stream.id(), port, "forward opened");
                let spawned = thread::Builder::new()
                    .name("condr-forward".into())
                    .spawn(move || isolated(&Arc::new(stream), |stream| forward(stream, port)));
                if let Err(error) = spawned {
                    tracing::warn!("forward thread failed to start: {error}");
                }
            }
            _ => stream.refuse(StreamError::InvalidPort, format!("{port} is not a port")),
        },
        StreamTarget::Download(_) | StreamTarget::Upload { .. } | StreamTarget::Unknown => {
            stream.refuse(StreamError::Unsupported, "")
        }
    }
}

fn forward(stream: &Arc<Stream>, port: u16) {
    let socket = match connect_loopback(port) {
        Ok(socket) => socket,
        Err(error) if error.kind() == io::ErrorKind::ConnectionRefused => {
            stream.refuse(
                StreamError::ConnectionRefused,
                format!("nothing is listening on port {port}"),
            );
            return;
        }
        Err(error) => {
            stream.refuse(StreamError::Io, error.to_string());
            return;
        }
    };
    stream.accept(None);
    splice(stream, socket);
}

/// `127.0.0.1` first, then `::1`, since many dev servers listen on IPv6 alone now that
/// Node resolves `localhost` there first.
fn connect_loopback(port: u16) -> io::Result<TcpStream> {
    let v4 = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let v6 = SocketAddr::from((Ipv6Addr::LOCALHOST, port));
    let socket = TcpStream::connect_timeout(&v4, LOOPBACK_CONNECT_TIMEOUT)
        .or_else(|_| TcpStream::connect_timeout(&v6, LOOPBACK_CONNECT_TIMEOUT))?;
    let _ = socket.set_nodelay(true);
    Ok(socket)
}
