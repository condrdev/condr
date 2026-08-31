use super::*;

pub(super) fn default_snapshot_path(endpoint: &Endpoint) -> PathBuf {
    if let Some(path) = std::env::var_os("CONDR_SNAPSHOT_PATH")
        && !path.is_empty()
    {
        return PathBuf::from(path);
    }
    snapshot_path_for_endpoint(endpoint)
}

pub(super) fn snapshot_path_for_endpoint(endpoint: &Endpoint) -> PathBuf {
    match endpoint {
        Endpoint::Local(path) => {
            let mut snapshot = path.as_os_str().to_os_string();
            snapshot.push(".snapshot");
            PathBuf::from(snapshot)
        }
        Endpoint::Tcp(_) => default_socket_path().with_file_name(format!(
            "condr-server-{:016x}.snapshot",
            stable_endpoint_id(endpoint)
        )),
    }
}

pub(super) fn stable_endpoint_id(endpoint: &Endpoint) -> u64 {
    let text = match endpoint {
        Endpoint::Local(path) => format!("local:{}", path.to_string_lossy()),
        Endpoint::Tcp(address) => format!("tcp:{address}"),
    };
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
    let endpoint = Endpoint::local(default_socket_path());
    if let Ok(stream) = endpoint.connect() {
        match probe_protocol(stream) {
            Ok(()) => return Ok(endpoint),
            Err(error) if error.kind() == io::ErrorKind::InvalidData => return Err(error),
            Err(_) => {}
        }
    }

    let server_executable = resolve_server_executable()?;
    let mut command = std::process::Command::new(&server_executable);
    command
        .arg("--endpoint")
        .arg(endpoint.as_local_path().expect("local endpoint"))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command.spawn().map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "failed to start condr-server at {}: {error}",
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
        "condr-server did not become ready",
    ))
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
    let server_name = if cfg!(windows) {
        "condr-server.exe"
    } else {
        "condr-server"
    };
    let sibling = current_executable
        .parent()
        .map(|parent| parent.join(server_name))
        .unwrap_or_else(|| PathBuf::from(server_name));
    sibling.is_file().then_some(sibling).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "condr-server is not installed beside the GUI; build or install the standalone server",
        )
    })
}

pub(super) fn probe_protocol(stream: EndpointStream) -> io::Result<()> {
    let _ = ClientConnection::handshake(stream, "condr-probe")?;
    Ok(())
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
