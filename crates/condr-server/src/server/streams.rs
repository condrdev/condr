//! The Server's end of a Stream connection (ADR 0061): after `Welcome` the connection
//! carries Streams, and each `Open` is answered here. A forward reaches this machine's own
//! loopback and nothing else.

use condr_client::{Stream, StreamConnection, isolated, splice};
use condr_core::WorkspaceId;
use condr_core::protocol::{MAX_STREAM_CHUNK, StreamError, StreamFile, StreamTarget};
use std::io::{self, Read as _};
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use super::RuntimeState;
use crate::endpoint::EndpointStream;
use crate::poison::OrAbort as _;

/// A loopback connect either answers or is refused at once; this only bounds a port
/// whose listener stopped accepting.
const LOOPBACK_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// Serves one Stream connection until it ends.
pub(super) fn serve(stream: EndpointStream, client_name: &str, state: Arc<Mutex<RuntimeState>>) {
    let connection =
        match StreamConnection::serve(stream, move |stream, target| open(&state, stream, target)) {
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

/// What a Stream's worker thread does with it.
type Work = dyn FnOnce(&Arc<Stream>) + Send;

fn open(state: &Arc<Mutex<RuntimeState>>, stream: Stream, target: StreamTarget) {
    let work: Box<Work> = match target {
        StreamTarget::Forward { port } => match u16::try_from(port) {
            Ok(port) if port != 0 => {
                tracing::debug!(stream = stream.id(), port, "forward opened");
                Box::new(move |stream| forward(stream, port))
            }
            _ => return stream.refuse(StreamError::InvalidPort, format!("{port} is not a port")),
        },
        StreamTarget::Download(file) => {
            tracing::debug!(stream = stream.id(), path = %file.path, "download opened");
            let state = Arc::clone(state);
            Box::new(move |stream| download(&state, stream, &file))
        }
        StreamTarget::Upload { file, replace } => {
            tracing::debug!(stream = stream.id(), path = %file.path, "upload opened");
            let state = Arc::clone(state);
            Box::new(move |stream| upload(&state, stream, &file, replace))
        }
        StreamTarget::Unknown => return stream.refuse(StreamError::Unsupported, ""),
    };
    // Off the reader thread, which must never block on a consumer.
    let spawned = thread::Builder::new()
        .name("condr-stream-worker".into())
        .spawn(move || isolated(&Arc::new(stream), work));
    if let Err(error) = spawned {
        tracing::warn!("stream worker failed to start: {error}");
    }
}

/// The Workspace's Root Directory, looked up once at `Open`; the data path then takes no
/// Server lock.
fn workspace_root(state: &Arc<Mutex<RuntimeState>>, workspace: WorkspaceId) -> Option<PathBuf> {
    state
        .lock()
        .or_abort()
        .session
        .workspace(workspace)
        .map(|workspace| workspace.root_directory().to_path_buf())
}

fn download(state: &Arc<Mutex<RuntimeState>>, stream: &Arc<Stream>, file: &StreamFile) {
    let Some(root) = workspace_root(state, file.workspace) else {
        return stream.refuse(StreamError::WorkspaceGone, "");
    };
    let (mut source, size) = match condr_core::open_download(&root, &file.path) {
        Ok(opened) => opened,
        Err((error, detail)) => return stream.refuse(error, detail),
    };
    stream.accept(Some(size));
    let mut buffer = vec![0; MAX_STREAM_CHUNK];
    loop {
        match source.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => {
                if stream.write(&buffer[..read]).is_err() {
                    return;
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => {
                tracing::warn!(path = %file.path, "download failed: {error}");
                return stream.reset(StreamError::Io, error.to_string());
            }
        }
    }
    stream.finish();
    // The Client answers with its own Finish once the file is in place.
    while let Ok(Some(_)) = stream.read() {}
}

fn upload(
    state: &Arc<Mutex<RuntimeState>>,
    stream: &Arc<Stream>,
    file: &StreamFile,
    replace: bool,
) {
    let Some(root) = workspace_root(state, file.workspace) else {
        return stream.refuse(StreamError::WorkspaceGone, "");
    };
    let mut upload = match condr_core::begin_upload(&root, &file.path, replace) {
        Ok(upload) => upload,
        Err((error, detail)) => return stream.refuse(error, detail),
    };
    stream.accept(None);
    loop {
        match stream.read() {
            Ok(Some(data)) => {
                if let Err(error) = upload.write(&data) {
                    tracing::warn!(path = %file.path, "upload failed: {error}");
                    return stream.reset(StreamError::Io, error.to_string());
                }
            }
            Ok(None) => break,
            // Cancelled or lost: dropping the upload removes its temporary file.
            Err(_) => return,
        }
    }
    if workspace_root(state, file.workspace).is_none() {
        return stream.reset(StreamError::WorkspaceGone, "");
    }
    match upload.commit() {
        Ok(()) => {
            if let Some(watcher) = &state.lock().or_abort().git_watcher {
                watcher.refresh(file.workspace);
            }
            stream.finish();
        }
        Err((error, detail)) => {
            tracing::warn!(path = %file.path, "upload failed: {detail}");
            stream.reset(error, detail);
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
