use std::net::TcpListener;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use condr_core::Session;
use condr_core::protocol::{ClientMessage, LayoutCommand, ServerId, ServerMessage, SessionEvent};
use condr_server::{ClientConnection, Endpoint, ServerConfig, ensure_local_server, stop_server};

const HELPER_ENV: &str = "CONDR_LOCAL_SERVER_TEST_HELPER";
const WORKSPACE_ENV: &str = "CONDR_LOCAL_SERVER_TEST_WORKSPACE";
const DETACHED_HELPER_ENV: &str = "CONDR_DETACHED_SERVER_TEST_HELPER";

#[test]
fn default_server_endpoint_is_private_and_local() {
    assert!(matches!(
        ServerConfig::default().endpoint,
        Endpoint::Local(_)
    ));
}

#[test]
fn ensure_local_server_reuses_a_live_standalone_process() {
    let endpoint_path = std::env::temp_dir().join(format!(
        "condr-process-test-{}-{}.sock",
        std::process::id(),
        unique_suffix()
    ));
    let workspace_path = endpoint_path.with_extension("workspace");
    let snapshot_path = endpoint_path.with_extension("snapshot");
    let output = Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("local_server_helper")
        .arg("--nocapture")
        .env(HELPER_ENV, "1")
        .env(WORKSPACE_ENV, &workspace_path)
        .env("CONDR_SOCKET_PATH", &endpoint_path)
        .env("CONDR_SNAPSHOT_PATH", &snapshot_path)
        .env("CONDR_SERVER_EXECUTABLE", env!("CARGO_BIN_EXE_condr"))
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "helper failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let _ = std::fs::remove_dir_all(workspace_path);
    let _ = std::fs::remove_file(&snapshot_path);
    let mut lock_path = snapshot_path.into_os_string();
    lock_path.push(".lock");
    let _ = std::fs::remove_file(lock_path);
}

#[test]
fn local_server_helper() {
    if std::env::var_os(HELPER_ENV).is_none() {
        return;
    }

    let first_start = thread::spawn(ensure_local_server);
    let second_start = thread::spawn(ensure_local_server);
    let first_endpoint = first_start.join().unwrap().unwrap();
    let concurrently_discovered_endpoint = second_start.join().unwrap().unwrap();
    assert_eq!(concurrently_discovered_endpoint, first_endpoint);

    let guard = ServerGuard(first_endpoint.clone());
    let first = ClientConnection::connect(&first_endpoint, "first-process-client").unwrap();
    let server_id = first.bootstrap().server_id;
    let runtime_epoch = first.bootstrap().runtime_epoch;
    let session_id = first.bootstrap().session_id;
    let initial_sequence = first.bootstrap().sequence;
    let mut first_stream = first.into_stream();
    first_stream
        .set_handshake_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    condr_core::protocol::write_message(
        &mut first_stream,
        &ClientMessage::AcquireControl { session_id },
    )
    .unwrap();
    assert!(matches!(
        read_server(&mut first_stream),
        ServerMessage::ControlGranted { .. }
    ));
    condr_core::protocol::write_message(
        &mut first_stream,
        &ClientMessage::Subscribe {
            session_id,
            after_sequence: initial_sequence,
        },
    )
    .unwrap();
    assert!(matches!(
        read_server(&mut first_stream),
        ServerMessage::Subscribed { .. }
    ));
    let workspace_root = std::env::var_os(WORKSPACE_ENV)
        .map(std::path::PathBuf::from)
        .expect("helper Workspace path is configured");
    std::fs::create_dir_all(&workspace_root).unwrap();
    condr_core::protocol::write_message(
        &mut first_stream,
        &ClientMessage::Layout {
            server_id,
            session_id,
            request_id: 1,
            command: LayoutCommand::CreateWorkspace {
                root_directory: workspace_root.clone(),
            },
        },
    )
    .unwrap();
    let sequence = loop {
        if let ServerMessage::Event {
            sequence,
            event: SessionEvent::LayoutChanged { .. },
            ..
        } = read_server(&mut first_stream)
        {
            break sequence;
        }
    };
    assert_eq!(
        read_server(&mut first_stream),
        ServerMessage::LayoutApplied {
            server_id,
            session_id,
            request_id: 1,
            sequence,
        }
    );
    let authoritative = ClientConnection::connect(&first_endpoint, "snapshot-reader").unwrap();
    let expected_snapshot = authoritative.bootstrap().snapshot.clone();
    let expected_session = Session::restore(expected_snapshot.clone()).unwrap();
    assert_eq!(expected_session.workspaces().len(), 1);
    drop(authoritative);

    thread::sleep(Duration::from_millis(30));
    let second_endpoint = ensure_local_server().unwrap();
    assert_eq!(second_endpoint, first_endpoint);
    let second = ClientConnection::connect(&second_endpoint, "second-process-client").unwrap();
    assert_eq!(second.bootstrap().server_id, server_id);
    assert_eq!(second.bootstrap().runtime_epoch, runtime_epoch);
    drop(second);
    drop(first_stream);

    stop_server(&first_endpoint).unwrap();
    wait_for_stop(&first_endpoint);
    guard.disarm();

    let restarted_endpoint = ensure_local_server().unwrap();
    let restarted_guard = ServerGuard(restarted_endpoint.clone());
    let restarted = ClientConnection::connect(&restarted_endpoint, "restarted-client").unwrap();
    let restarted_sequence = restarted.bootstrap().sequence;
    let restarted_session_id = restarted.bootstrap().session_id;
    assert_eq!(restarted.bootstrap().server_id, server_id);
    assert_ne!(restarted.bootstrap().runtime_epoch, runtime_epoch);
    assert_eq!(restarted.bootstrap().snapshot, expected_snapshot);
    assert_eq!(restarted.bootstrap().terminals.len(), 1);
    assert!(restarted.bootstrap().agents.is_empty());
    let restarted_session = Session::restore(restarted.bootstrap().snapshot.clone()).unwrap();
    assert_eq!(
        restarted_session
            .workspaces()
            .first()
            .unwrap()
            .root_directory(),
        workspace_root.as_path()
    );
    let workspace_id = restarted_session.workspaces()[0].id();
    let mut restarted_stream = restarted.into_stream();
    restarted_stream
        .set_handshake_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    condr_core::protocol::write_message(
        &mut restarted_stream,
        &ClientMessage::AcquireControl {
            session_id: restarted_session_id,
        },
    )
    .unwrap();
    assert!(matches!(
        read_server(&mut restarted_stream),
        ServerMessage::ControlGranted { .. }
    ));
    condr_core::protocol::write_message(
        &mut restarted_stream,
        &ClientMessage::Subscribe {
            session_id: restarted_session_id,
            after_sequence: restarted_sequence,
        },
    )
    .unwrap();
    assert!(matches!(
        read_server(&mut restarted_stream),
        ServerMessage::Subscribed { .. }
    ));
    condr_core::protocol::write_message(
        &mut restarted_stream,
        &ClientMessage::Layout {
            server_id,
            session_id: restarted_session_id,
            request_id: 2,
            command: LayoutCommand::CloseWorkspace { workspace_id },
        },
    )
    .unwrap();
    let sequence = loop {
        if let ServerMessage::Event {
            sequence,
            event: SessionEvent::LayoutChanged { .. },
            ..
        } = read_server(&mut restarted_stream)
        {
            break sequence;
        }
    };
    assert_eq!(
        read_server(&mut restarted_stream),
        ServerMessage::LayoutApplied {
            server_id,
            session_id: restarted_session_id,
            request_id: 2,
            sequence,
        }
    );
    drop(restarted_stream);

    stop_server(&restarted_endpoint).unwrap();
    wait_for_stop(&restarted_endpoint);
    restarted_guard.disarm();

    let empty_endpoint = ensure_local_server().unwrap();
    let empty_guard = ServerGuard(empty_endpoint.clone());
    let empty = ClientConnection::connect(&empty_endpoint, "empty-restart-client").unwrap();
    assert_eq!(empty.bootstrap().snapshot, Session::new().snapshot());
    assert!(empty.bootstrap().terminals.is_empty());
    drop(empty);
    stop_server(&empty_endpoint).unwrap();
    wait_for_stop(&empty_endpoint);
    empty_guard.disarm();
    let _ = std::fs::remove_file(server_log_path(server_id));
}

#[test]
fn auto_started_server_survives_launcher_exit() {
    let endpoint_path = std::env::temp_dir().join(format!(
        "condr-detached-test-{}-{}.sock",
        std::process::id(),
        unique_suffix()
    ));
    let snapshot_path = endpoint_path.with_extension("snapshot");
    let status = Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("detached_server_launcher_helper")
        .arg("--nocapture")
        .env(DETACHED_HELPER_ENV, "1")
        .env("CONDR_SOCKET_PATH", &endpoint_path)
        .env("CONDR_SNAPSHOT_PATH", &snapshot_path)
        .env("CONDR_SERVER_EXECUTABLE", env!("CARGO_BIN_EXE_condr"))
        .status()
        .unwrap();
    assert!(status.success(), "launcher helper failed: {status}");

    let endpoint = Endpoint::local(&endpoint_path);
    let guard = ServerGuard(endpoint.clone());
    let client = ClientConnection::connect(&endpoint, "detached-process-check").unwrap();
    let log_path = server_log_path(client.bootstrap().server_id);
    #[cfg(unix)]
    assert_is_session_leader(client.bootstrap().runtime_epoch);
    drop(client);
    stop_server(&endpoint).unwrap();
    wait_for_stop(&endpoint);
    guard.disarm();

    let log = std::fs::read_to_string(&log_path).unwrap();
    assert!(log.contains("condr-server: detached process"));

    let _ = std::fs::remove_file(&snapshot_path);
    let mut snapshot_lock = snapshot_path.into_os_string();
    snapshot_lock.push(".lock");
    let _ = std::fs::remove_file(snapshot_lock);
    let _ = std::fs::remove_file(log_path);
}

#[test]
fn lifecycle_commands_manage_a_detached_server() {
    let data_directory = std::env::temp_dir().join(format!(
        "condr-command-test-{}-{}",
        std::process::id(),
        unique_suffix()
    ));
    std::fs::create_dir_all(&data_directory).unwrap();
    let socket_path = data_directory.join("condr.sock");
    let snapshot_path = data_directory.join("state.snapshot");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let server = env!("CARGO_BIN_EXE_condr");
    let endpoint = Endpoint::tcp(address);
    let guard = ServerGuard(endpoint.clone());

    let start = Command::new(server)
        .args(["server", "start"])
        .arg("--listen")
        .arg(address.to_string())
        .arg("--snapshot")
        .arg(&snapshot_path)
        .env("CONDR_SOCKET_PATH", &socket_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let (output_sender, output_receiver) = mpsc::sync_channel(1);
    thread::spawn(move || output_sender.send(start.wait_with_output()).unwrap());
    let output = output_receiver
        .recv_timeout(Duration::from_secs(5))
        .unwrap_or_else(|_| {
            let _ = stop_server(&endpoint);
            panic!("start kept its output pipes open after launching the detached server");
        })
        .unwrap();
    assert!(
        output.status.success(),
        "start failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let client = ClientConnection::connect(&endpoint, "log-path-check").unwrap();
    let log_path = server_log_path(client.bootstrap().server_id);
    drop(client);

    let status = Command::new(server)
        .args(["server", "status"])
        .arg("--listen")
        .arg(address.to_string())
        .status()
        .unwrap();
    assert!(status.success(), "status failed: {status}");

    let second_start = Command::new(server)
        .args(["server", "start"])
        .arg("--listen")
        .arg(address.to_string())
        .arg("--snapshot")
        .arg(&snapshot_path)
        .env("CONDR_SOCKET_PATH", &socket_path)
        .status()
        .unwrap();
    assert!(
        second_start.success(),
        "second start failed: {second_start}"
    );

    let stop = Command::new(server)
        .args(["server", "stop"])
        .arg("--listen")
        .arg(address.to_string())
        .status()
        .unwrap();
    assert!(stop.success(), "stop failed: {stop}");
    wait_for_stop(&endpoint);
    guard.disarm();

    let stopped_status = Command::new(server)
        .args(["server", "status"])
        .arg("--listen")
        .arg(address.to_string())
        .status()
        .unwrap();
    assert_eq!(stopped_status.code(), Some(1));

    let _ = std::fs::remove_file(log_path);
    std::fs::remove_dir_all(data_directory).unwrap();
}

#[test]
fn detached_server_launcher_helper() {
    if std::env::var_os(DETACHED_HELPER_ENV).is_some() {
        ensure_local_server().unwrap();
    }
}

#[cfg(unix)]
fn assert_is_session_leader(runtime_epoch: condr_core::protocol::RuntimeEpoch) {
    let pid = i32::try_from(runtime_epoch.0 >> 96).unwrap();
    let pid = nix::unistd::Pid::from_raw(pid);
    assert_eq!(nix::unistd::getsid(Some(pid)).unwrap(), pid);
}

fn server_log_path(server_id: ServerId) -> std::path::PathBuf {
    condr_core::log_directory()
        .expect("platform log directory")
        .join(format!("condr-server-{:016x}.log", server_id.0))
}

struct ServerGuard(Endpoint);

impl ServerGuard {
    fn disarm(self) {
        std::mem::forget(self);
    }
}

impl Drop for ServerGuard {
    fn drop(&mut self) {
        let _ = stop_server(&self.0);
    }
}

fn wait_for_stop(endpoint: &Endpoint) {
    // 5 s: under a full parallel `cargo test --workspace` the 1 s budget flaked.
    for _ in 0..500 {
        if endpoint.connect().is_err() {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!("standalone server did not stop");
}

fn read_server(stream: &mut condr_server::EndpointStream) -> ServerMessage {
    condr_core::protocol::read_message(stream).unwrap()
}

fn unique_suffix() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}
