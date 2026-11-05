//! `condr --device <name> port forward <port>...` (ADR 0061): forwards ports of a remote
//! Device to this machine's loopback for as long as it runs. One Stream connection
//! carries every forward of the command; it rides Condr's SSH master like every other
//! `--device` call, since this process has no terminal connection to keep apart.

use super::interrupt::Interrupt;
use super::*;
use condr_client::{
    Forward, ForwardEvent, LocalPort, StreamConnection, StreamOpener, describe_stream_error,
};
use std::sync::Arc;

#[derive(Subcommand)]
pub(crate) enum PortCommand {
    /// Forward ports of a remote Device to this machine until Ctrl+C. Each port is REMOTE
    /// or LOCAL:REMOTE, the local port first as in `ssh -L`; a local 0 lets the system
    /// choose. Once every port listens, one JSON line names them.
    Forward {
        #[arg(required = true, value_name = "[LOCAL:]REMOTE")]
        ports: Vec<String>,
    },
}

pub(crate) fn run_port(device: Option<&str>, command: PortCommand) -> i32 {
    let PortCommand::Forward { ports } = command;
    let result = (|| {
        let device = device.ok_or_else(|| {
            CliError::new(
                "device_required",
                "port forward needs --device <name> (or CONDR_DEVICE); this machine's ports are already here",
            )
        })?;
        let specs = ports
            .iter()
            .map(|text| parse_spec(text))
            .collect::<Result<Vec<_>, _>>()?;
        let endpoint = device::device_endpoint(&device::saved_devices()?, device)?;
        let interrupt = Interrupt::install()?;
        serve_forwards(&endpoint, device, &specs, &interrupt, &mut io::stdout())
    })();
    match result {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("{}", json!({ "error": error }));
            1
        }
    }
}

/// One `[LOCAL:]REMOTE`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ForwardSpec {
    pub(super) local: u16,
    pub(super) remote: u16,
}

pub(super) fn parse_spec(text: &str) -> Result<ForwardSpec, CliError> {
    let invalid = || {
        CliError::new(
            "invalid_port",
            format!("{text:?} is not REMOTE or LOCAL:REMOTE, ports from 1 to 65535"),
        )
    };
    let (local, remote) = match text.split_once(':') {
        Some((local, remote)) => (local, remote),
        None => (text, text),
    };
    let remote: u16 = remote.parse().map_err(|_| invalid())?;
    let local: u16 = local.parse().map_err(|_| invalid())?;
    if remote == 0 || (local == 0 && !text.contains(':')) {
        return Err(invalid());
    }
    Ok(ForwardSpec { local, remote })
}

/// Opens the Stream connection, binds every port and runs until Ctrl+C, which returns
/// `Ok`, or until the connection is lost, which is an error.
pub(super) fn serve_forwards(
    endpoint: &Endpoint,
    device: &str,
    specs: &[ForwardSpec],
    interrupt: &Interrupt,
    out: &mut impl io::Write,
) -> Result<(), CliError> {
    let (connection, _) = StreamConnection::connect(endpoint, "condr-cli")
        .map_err(|error| stream_connect_error(endpoint, &error))?;
    let connection = Arc::new(connection);
    {
        let connection = Arc::clone(&connection);
        interrupt.on_interrupt(move || connection.close());
    }
    let opener: Arc<dyn StreamOpener> = connection.clone();
    let mut forwards = Vec::new();
    let mut listed = Vec::new();
    for spec in specs {
        let remote = spec.remote;
        let forward = Forward::bind(
            remote,
            LocalPort::Exact(spec.local),
            Arc::clone(&opener),
            move |event| {
                if let ForwardEvent::Refused(reason) = event {
                    eprintln!("port {remote}: {reason}");
                }
            },
        )
        .map_err(|error| match error.kind() {
            io::ErrorKind::AddrInUse => CliError::new("port_in_use", error.to_string()),
            _ => error.into(),
        })?;
        listed.push(json!({
            "device": device,
            "remote": remote,
            "local": format!("127.0.0.1:{}", forward.local_port()),
        }));
        forwards.push(forward);
    }
    writeln!(out, "{}", json!({ "forwards": listed }))?;
    out.flush()?;
    let reason = connection.wait_closed();
    drop(forwards);
    if interrupt.fired() {
        return Ok(());
    }
    Err(CliError::new("connection_lost", reason))
}

/// How a Stream connection that would not open reads: an older Server is
/// `feature_unavailable`, naming its build (ADR 0061).
pub(super) fn stream_connect_error(endpoint: &Endpoint, error: &io::Error) -> CliError {
    match condr_server::Refused::from_error(error).map(|refused| &refused.refusal) {
        Some(Refusal::Malformed(_)) => {
            CliError::new("feature_unavailable", describe_stream_error(error))
        }
        Some(Refusal::IncompatibleProtocol) => {
            CliError::new("protocol_mismatch", endpoint.describe_connect_error(error))
        }
        _ => match error.kind() {
            io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused => {
                CliError::new("server_not_running", endpoint.describe_connect_error(error))
            }
            io::ErrorKind::PermissionDenied => {
                CliError::new("not_authorized", endpoint.describe_connect_error(error))
            }
            _ => CliError::new("io", endpoint.describe_connect_error(error)),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use condr_server::{BoundServer, ServerConfig};
    use std::io::{Read as _, Write as _};
    use std::net::{TcpListener, TcpStream};
    use std::time::Duration;

    #[test]
    fn a_port_is_remote_or_local_then_remote() {
        assert_eq!(
            parse_spec("5173").unwrap(),
            ForwardSpec {
                local: 5173,
                remote: 5173
            }
        );
        assert_eq!(
            parse_spec("8080:80").unwrap(),
            ForwardSpec {
                local: 8080,
                remote: 80
            }
        );
        assert_eq!(parse_spec("0:80").unwrap().local, 0);
        for invalid in ["0", "x", "80:0", "70000", "1:2:3", ""] {
            assert!(parse_spec(invalid).is_err(), "{invalid} should be refused");
        }
    }

    /// Writes stdout's first line to a channel, so the test reads what a script would.
    struct Lines(std::sync::mpsc::Sender<String>, Vec<u8>);

    impl io::Write for Lines {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            self.1.extend_from_slice(buffer);
            Ok(buffer.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            let _ = self
                .0
                .send(String::from_utf8_lossy(&self.1).trim().to_owned());
            Ok(())
        }
    }

    #[test]
    fn forwarding_prints_its_ports_once_bound_and_a_taken_port_fails() {
        let root = std::env::temp_dir().join(format!("condr-port-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let endpoint = Endpoint::local(root.join("server.sock"));
        let server =
            BoundServer::bind(ServerConfig::ephemeral(endpoint.as_local_path().unwrap())).unwrap();
        let handle = server.handle();
        let runtime = std::thread::spawn(move || server.run());

        let service = TcpListener::bind("127.0.0.1:0").unwrap();
        let remote = service.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut socket, _) = service.accept().unwrap();
            socket.write_all(b"from the device").unwrap();
        });

        let interrupt = Interrupt::by_hand();
        let (lines, printed) = std::sync::mpsc::channel();
        let running = {
            let endpoint = endpoint.clone();
            let interrupt = interrupt.clone();
            std::thread::spawn(move || {
                let specs = [ForwardSpec { local: 0, remote }];
                serve_forwards(
                    &endpoint,
                    "box",
                    &specs,
                    &interrupt,
                    &mut Lines(lines, Vec::new()),
                )
            })
        };
        let line: Value =
            serde_json::from_str(&printed.recv_timeout(Duration::from_secs(10)).unwrap()).unwrap();
        let local = line["forwards"][0]["local"].as_str().unwrap().to_owned();
        assert_eq!(line["forwards"][0]["remote"], remote);
        assert_eq!(line["forwards"][0]["device"], "box");
        let mut socket = TcpStream::connect(&local).unwrap();
        let mut answer = String::new();
        socket.read_to_string(&mut answer).unwrap();
        assert_eq!(answer, "from the device");

        // A taken local port fails the command rather than moving.
        let held: u16 = local.rsplit(':').next().unwrap().parse().unwrap();
        let taken = serve_forwards(
            &endpoint,
            "box",
            &[ForwardSpec {
                local: held,
                remote,
            }],
            &Interrupt::by_hand(),
            &mut Vec::new(),
        );
        assert_eq!(taken.unwrap_err().code, "port_in_use");

        // Ctrl+C ends it with success.
        interrupt.fire();
        assert!(running.join().unwrap().is_ok());
        handle.stop();
        runtime.join().unwrap().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}
