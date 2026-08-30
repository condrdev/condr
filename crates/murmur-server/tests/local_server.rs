use std::process::Command;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use murmur_core::Session;
use murmur_core::protocol::{ClientMessage, LayoutCommand, ServerMessage, SessionEvent};
use murmur_server::{ClientConnection, Endpoint, ServerConfig, ensure_local_server, stop_server};

const HELPER_ENV: &str = "MURMUR_LOCAL_SERVER_TEST_HELPER";
const WORKSPACE_ENV: &str = "MURMUR_LOCAL_SERVER_TEST_WORKSPACE";

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
        "murmur-process-test-{}-{}.sock",
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
        .env("MURMUR_SOCKET_PATH", &endpoint_path)
        .env("MURMUR_SNAPSHOT_PATH", &snapshot_path)
        .env(
            "MURMUR_SERVER_EXECUTABLE",
            env!("CARGO_BIN_EXE_murmur-server"),
        )
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
    murmur_core::protocol::write_message(
        &mut first_stream,
        &ClientMessage::AcquireControl { session_id },
    )
    .unwrap();
    assert!(matches!(
        read_server(&mut first_stream),
        ServerMessage::ControlGranted { .. }
    ));
    murmur_core::protocol::write_message(
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
    murmur_core::protocol::write_message(
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
            event: SessionEvent::LayoutChanged,
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
    murmur_core::protocol::write_message(
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
    murmur_core::protocol::write_message(
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
    murmur_core::protocol::write_message(
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
            event: SessionEvent::LayoutChanged,
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
    let _ = empty_endpoint.cleanup();
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
        let _ = self.0.cleanup();
    }
}

fn wait_for_stop(endpoint: &Endpoint) {
    for _ in 0..100 {
        if endpoint.connect().is_err() {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!("standalone server did not stop");
}

fn read_server(stream: &mut murmur_server::EndpointStream) -> ServerMessage {
    murmur_core::protocol::read_message(stream).unwrap()
}

fn unique_suffix() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}
