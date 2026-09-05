use super::*;

use std::fs::{self, OpenOptions};
use std::path::Path;
#[cfg(not(windows))]
use std::process::{Command, Stdio};
#[cfg(windows)]
use windows_spawn::{Command, CreationFlags, SpawnOptions, Stdio};

pub(super) fn default_snapshot_path(socket_path: &Path) -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("CONDR_SNAPSHOT_PATH")
        && !path.is_empty()
    {
        return Some(PathBuf::from(path));
    }
    snapshot_path_for_endpoint(socket_path)
}

pub(super) fn snapshot_path_for_endpoint(socket_path: &Path) -> Option<PathBuf> {
    endpoint_file(condr_core::state_directory(), socket_path, "snapshot")
}

fn endpoint_file(
    directory: Option<PathBuf>,
    socket_path: &Path,
    extension: &str,
) -> Option<PathBuf> {
    directory.map(|directory| {
        directory.join(format!(
            "condr-server-{:016x}.{extension}",
            stable_endpoint_id(socket_path)
        ))
    })
}

/// The Server identity is its local socket path; the TCP listener is an extra door
/// to the same Server and does not change it.
pub(super) fn stable_endpoint_id(socket_path: &Path) -> u64 {
    #[cfg(unix)]
    let path = std::path::absolute(socket_path).unwrap_or_else(|_| socket_path.to_path_buf());
    #[cfg(not(unix))]
    let path = socket_path.to_path_buf();
    let text = format!("local:{}", path.to_string_lossy());
    text.bytes().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    })
}

pub(super) fn runtime_epoch() -> u128 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    (u128::from(std::process::id()) << 96) ^ nanos
}

pub fn ensure_local_server() -> io::Result<Endpoint> {
    ensure_server(ServerConfig::default())
}

/// Connects to the host's Server, starting it detached first if nothing answers on its
/// socket. Returns the local endpoint to talk to it on.
pub fn ensure_server(config: ServerConfig) -> io::Result<Endpoint> {
    let endpoint = config.local_endpoint();
    if let Ok(stream) = endpoint.connect() {
        match probe_protocol(stream) {
            Ok(()) => return Ok(endpoint),
            Err(error) if error.kind() == io::ErrorKind::InvalidData => return Err(error),
            Err(_) => {}
        }
    }

    let server_executable = resolve_server_executable()?;
    let log_path = server_log_path(&config.socket_path)?;
    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut log_options = OpenOptions::new();
    log_options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        log_options.mode(0o600);
    }
    let log = log_options.open(&log_path).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "failed to open condr-server log at {}: {error}",
                log_path.display()
            ),
        )
    })?;
    let stderr = log.try_clone()?;
    let mut command = Command::new(&server_executable);
    command.args(["server", "run"]);
    command.arg("--endpoint").arg(&config.socket_path);
    if let Some(address) = config.listen {
        command.arg("--listen").arg(address.to_string());
    }
    if let Some(path) = config.snapshot_path() {
        command.arg("--snapshot").arg(path);
    }
    command
        .arg("--detached")
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(stderr));
    #[cfg(windows)]
    let spawn_result =
        command.spawn_with(SpawnOptions::new().creation_flags(CreationFlags::DETACHED_PROCESS));
    #[cfg(not(windows))]
    let spawn_result = command.spawn();
    spawn_result.map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "failed to start the Server with {}: {error}",
                server_executable.display()
            ),
        )
    })?;

    for _ in 0..150 {
        if let Ok(stream) = endpoint.connect()
            && probe_protocol(stream).is_ok()
        {
            return Ok(endpoint);
        }
        thread::sleep(Duration::from_millis(10));
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        format!(
            "condr-server did not become ready; see {}",
            log_path.display()
        ),
    ))
}

fn server_log_path(socket_path: &Path) -> io::Result<PathBuf> {
    endpoint_file(condr_core::log_directory(), socket_path, "log").ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "no platform log directory is available",
        )
    })
}

pub(super) fn resolve_server_executable() -> io::Result<PathBuf> {
    if let Some(path) = std::env::var_os("CONDR_SERVER_EXECUTABLE") {
        let path = PathBuf::from(path);
        return path.is_file().then_some(path).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "CONDR_SERVER_EXECUTABLE does not name a file",
            )
        });
    }

    let current_executable = std::env::current_exe()?;
    // The Server and the CLI share the `condr` binary, which the GUI ships beside itself.
    let server_name = if cfg!(windows) { "condr.exe" } else { "condr" };
    let sibling = current_executable
        .parent()
        .map(|parent| parent.join(server_name))
        .unwrap_or_else(|| PathBuf::from(server_name));
    sibling.is_file().then_some(sibling).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "condr is not installed beside condr-gui; build or install the `condr` binary",
        )
    })
}

pub(super) fn probe_protocol(stream: EndpointStream) -> io::Result<()> {
    ClientConnection::welcome(stream, "condr-probe").map(drop)
}

/// Checks whether a protocol-compatible server is reachable at the endpoint.
pub fn probe_server(endpoint: &Endpoint) -> io::Result<()> {
    probe_protocol(endpoint.connect()?)
}

/// Asks the running Server to drop the live connections of devices whose key starts with
/// `key_prefix`; returns how many it closed. The caller has already edited the
/// authorized list, so those devices cannot come back.
pub fn revoke_devices(endpoint: &Endpoint, key_prefix: &str) -> io::Result<u32> {
    let client = ClientConnection::connect(endpoint, "condr-revoke")?;
    let mut stream = client.into_stream();
    condr_core::protocol::write_message(
        &mut stream,
        &ClientMessage::RevokeDevice {
            key_prefix: key_prefix.to_owned(),
        },
    )
    .map_err(|error| io::Error::other(error.to_string()))?;
    match condr_core::protocol::read_message(&mut stream)
        .map_err(|error| io::Error::other(error.to_string()))?
    {
        ServerMessage::DevicesRevoked { disconnected } => Ok(disconnected),
        ServerMessage::Error { message } => {
            Err(io::Error::new(io::ErrorKind::PermissionDenied, message))
        }
        other => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unexpected revoke response: {other:?}"),
        )),
    }
}

pub fn stop_server(endpoint: &Endpoint) -> io::Result<()> {
    let client = ClientConnection::connect(endpoint, "condr-stop")?;
    let server_id = client.bootstrap.server_id;
    let mut stream = client.into_stream();
    condr_core::protocol::write_message(&mut stream, &ClientMessage::StopServer { server_id })
        .map_err(|error| io::Error::other(error.to_string()))?;
    match condr_core::protocol::read_message(&mut stream)
        .map_err(|error| io::Error::other(error.to_string()))?
    {
        ServerMessage::ServerStopping => Ok(()),
        other => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unexpected stop response: {other:?}"),
        )),
    }
}
