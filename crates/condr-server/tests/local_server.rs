use std::net::TcpListener;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use condr_core::Session;
use condr_core::protocol::{ClientMessage, LayoutCommand, ServerId, ServerMessage, SessionEvent};
use condr_server::{ClientConnection, Endpoint, ServerConfig, ensure_local_server, stop_server};

mod common;
use common::unique_suffix;

const HELPER_ENV: &str = "CONDR_LOCAL_SERVER_TEST_HELPER";
const WORKSPACE_ENV: &str = "CONDR_LOCAL_SERVER_TEST_WORKSPACE";
const DETACHED_HELPER_ENV: &str = "CONDR_DETACHED_SERVER_TEST_HELPER";

#[test]
fn default_server_endpoint_is_private_and_local() {
    assert!(matches!(
        ServerConfig::default().local_endpoint(),
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
        // The host's own config.toml may name a TCP listener that is already in use.
        .env("CONDR_CONFIG_DIR", endpoint_path.with_extension("config"))
        .env("CONDR_DATA_DIR", endpoint_path.with_extension("config"))
        .env("CONDR_LOG_DIR", endpoint_path.with_extension("config"))
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
    let server_id = first.bootstrap().unwrap().server_id;
    let runtime_epoch = first.bootstrap().unwrap().runtime_epoch;
    let session_id = first.bootstrap().unwrap().session_id;
    let initial_sequence = first.bootstrap().unwrap().sequence;
    let mut first_stream = first.into_stream();
    first_stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
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
                name: None,
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
    assert!(matches!(
        read_layout_response(&mut first_stream),
        ServerMessage::LayoutApplied {
            server_id: actual_server,
            session_id: actual_session,
            request_id: 1,
            sequence: actual_sequence,
            result: condr_core::protocol::LayoutResult::WorkspaceCreated { .. },
        } if actual_server == server_id && actual_session == session_id && actual_sequence == sequence
    ));
    let authoritative = ClientConnection::connect(&first_endpoint, "snapshot-reader").unwrap();
    let expected_snapshot = authoritative.bootstrap().unwrap().snapshot.clone();
    let expected_session = Session::restore(expected_snapshot.clone()).unwrap();
    assert_eq!(expected_session.workspaces().len(), 1);
    drop(authoritative);

    thread::sleep(Duration::from_millis(30));
    let second_endpoint = ensure_local_server().unwrap();
    assert_eq!(second_endpoint, first_endpoint);
    let second = ClientConnection::connect(&second_endpoint, "second-process-client").unwrap();
    assert_eq!(second.bootstrap().unwrap().server_id, server_id);
    assert_eq!(second.bootstrap().unwrap().runtime_epoch, runtime_epoch);
    drop(second);
    drop(first_stream);

    stop_server(&first_endpoint).unwrap();
    wait_for_stop(&first_endpoint);
    guard.disarm();

    let restarted_endpoint = ensure_local_server().unwrap();
    let restarted_guard = ServerGuard(restarted_endpoint.clone());
    let restarted = ClientConnection::connect(&restarted_endpoint, "restarted-client").unwrap();
    let restarted_sequence = restarted.bootstrap().unwrap().sequence;
    let restarted_session_id = restarted.bootstrap().unwrap().session_id;
    assert_eq!(restarted.bootstrap().unwrap().server_id, server_id);
    assert_ne!(restarted.bootstrap().unwrap().runtime_epoch, runtime_epoch);
    assert_eq!(restarted.bootstrap().unwrap().snapshot, expected_snapshot);
    assert_eq!(restarted.bootstrap().unwrap().terminals.len(), 1);
    assert!(restarted.bootstrap().unwrap().agents.is_empty());
    let restarted_session =
        Session::restore(restarted.bootstrap().unwrap().snapshot.clone()).unwrap();
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
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
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
        read_layout_response(&mut restarted_stream),
        ServerMessage::LayoutApplied {
            server_id,
            session_id: restarted_session_id,
            request_id: 2,
            sequence,
            result: Default::default(),
        }
    );
    drop(restarted_stream);

    stop_server(&restarted_endpoint).unwrap();
    wait_for_stop(&restarted_endpoint);
    restarted_guard.disarm();

    let empty_endpoint = ensure_local_server().unwrap();
    let empty_guard = ServerGuard(empty_endpoint.clone());
    let empty = ClientConnection::connect(&empty_endpoint, "empty-restart-client").unwrap();
    assert_eq!(
        empty.bootstrap().unwrap().snapshot,
        Session::new().snapshot()
    );
    assert!(empty.bootstrap().unwrap().terminals.is_empty());
    drop(empty);
    stop_server(&empty_endpoint).unwrap();
    wait_for_stop(&empty_endpoint);
    empty_guard.disarm();
    let _ = std::fs::remove_file(server_log_path(
        &condr_core::log_directory().expect("log directory"),
        server_id,
    ));
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
        // The host's own config.toml may name a TCP listener that is already in use.
        .env("CONDR_CONFIG_DIR", endpoint_path.with_extension("config"))
        .env("CONDR_DATA_DIR", endpoint_path.with_extension("config"))
        .env("CONDR_LOG_DIR", endpoint_path.with_extension("config"))
        .env("CONDR_SNAPSHOT_PATH", &snapshot_path)
        .env("CONDR_SERVER_EXECUTABLE", env!("CARGO_BIN_EXE_condr"))
        .status()
        .unwrap();
    assert!(status.success(), "launcher helper failed: {status}");

    let endpoint = Endpoint::local(&endpoint_path);
    let guard = ServerGuard(endpoint.clone());
    let client = ClientConnection::connect(&endpoint, "detached-process-check").unwrap();
    let log_path = server_log_path(
        &endpoint_path.with_extension("config"),
        client.bootstrap().unwrap().server_id,
    );
    #[cfg(unix)]
    assert_left_launcher_session(client.bootstrap().unwrap().runtime_epoch);
    drop(client);
    stop_server(&endpoint).unwrap();
    wait_for_stop(&endpoint);
    guard.disarm();

    let log = std::fs::read_to_string(&log_path).unwrap();
    assert!(log.contains("detached process"));

    let _ = std::fs::remove_file(&snapshot_path);
    let mut snapshot_lock = snapshot_path.into_os_string();
    snapshot_lock.push(".lock");
    let _ = std::fs::remove_file(snapshot_lock);
    let _ = std::fs::remove_file(log_path);
}

#[test]
fn lifecycle_commands_manage_a_detached_server() {
    // The Workspace root doubles as the Pane's cwd, which the Server observes from the
    // live shell as a canonical path. macOS puts the temporary directory behind the
    // `/var -> /private/var` symlink, so start canonical or the snapshots differ.
    let temp_dir = std::env::temp_dir();
    // Windows canonicalizes to a verbatim `\\?\` path, which the Pane shell cannot use as cwd.
    #[cfg(unix)]
    let temp_dir = std::fs::canonicalize(temp_dir).unwrap();
    let data_directory = temp_dir.join(format!(
        "condr-command-{}-{}",
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
    // Every child process shares the test's config directory and socket, as the commands
    // on one host do; `start --listen` persists the TCP address there.
    let condr = |args: &[&str]| {
        let mut command = Command::new(server);
        command
            .args(args)
            .env("CONDR_CONFIG_DIR", &data_directory)
            .env("CONDR_DATA_DIR", &data_directory)
            .env("CONDR_LOG_DIR", &data_directory)
            .env("CONDR_SOCKET_PATH", &socket_path)
            .env("CONDR_SNAPSHOT_PATH", &snapshot_path)
            .env_remove("CONDR_PANE_ID");
        command
    };
    let endpoint = Endpoint::local(&socket_path);
    let guard = ServerGuard(endpoint.clone());

    let start = condr(&["server", "start", "--listen"])
        .arg(address.to_string())
        .arg("--snapshot")
        .arg(&snapshot_path)
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
    let log_path = server_log_path(&data_directory, client.bootstrap().unwrap().server_id);
    drop(client);

    // The same Server answers on TCP: a remote device pairs with the invite the host
    // prints, and afterwards connects on its key alone.
    let invite = condr(&["server", "invite"]).output().unwrap();
    assert!(
        invite.status.success(),
        "invite failed\nstderr:\n{}",
        String::from_utf8_lossy(&invite.stderr)
    );
    let invite_text = String::from_utf8(invite.stdout).unwrap();
    let locator = invite_text
        .lines()
        .map(str::trim)
        .find(|line| line.contains("@<host>:"))
        .unwrap_or_else(|| panic!("invite output names no locator:\n{invite_text}"))
        // `TCP           tcp://…@<host>:port`: the link is the value after the label.
        .rsplit(char::is_whitespace)
        .next()
        .unwrap()
        .replace("<host>", &address.ip().to_string());
    assert!(
        locator.ends_with(&format!(":{}", address.port())),
        "the invite names the configured port: {locator}"
    );
    let device_key = condr_server::DeviceKey::generate().unwrap();
    let paired =
        Endpoint::tcp(condr_server::TcpEndpoint::parse(&locator, device_key.clone()).unwrap());
    drop(ClientConnection::connect(&paired, "laptop").unwrap());
    let Endpoint::Tcp(tcp) = paired else {
        unreachable!()
    };
    let paired = Endpoint::tcp(tcp.without_invite());
    drop(ClientConnection::connect(&paired, "laptop").unwrap());
    let clients = condr(&["server", "clients"]).output().unwrap();
    assert!(
        String::from_utf8_lossy(&clients.stdout).contains("laptop"),
        "clients output lists the paired device:\n{}",
        String::from_utf8_lossy(&clients.stdout)
    );

    let status = condr(&["server", "status"]).status().unwrap();
    assert!(status.success(), "status failed: {status}");

    let second_start = condr(&["server", "start", "--snapshot"])
        .arg(&snapshot_path)
        .status()
        .unwrap();
    assert!(
        second_start.success(),
        "second start failed: {second_start}"
    );

    let mut client = ClientConnection::connect_overview(&endpoint, "before-restart").unwrap();
    let epoch = client.overview().runtime_epoch;
    let server_id = client.overview().server_id;
    client
        .layout(LayoutCommand::CreateWorkspace {
            root_directory: data_directory.clone(),
            name: Some("restart-test".into()),
        })
        .unwrap()
        .unwrap();
    let snapshot = client.overview().snapshot.clone();
    drop(client);

    // A Pane-owned CLI would be killed during shutdown, so reject before stopping.
    let pane_restart = condr(&["server", "restart"])
        .env("CONDR_PANE_ID", "1")
        .output()
        .unwrap();
    assert_eq!(pane_restart.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&pane_restart.stderr).contains("outside Condr"));
    assert_eq!(
        ClientConnection::connect_overview(&endpoint, "still-running")
            .unwrap()
            .overview()
            .runtime_epoch,
        epoch
    );

    let restart = condr(&["server", "restart"]).output().unwrap();
    assert!(
        restart.status.success(),
        "restart failed: {}",
        String::from_utf8_lossy(&restart.stderr)
    );
    let client = ClientConnection::connect_overview(&endpoint, "after-restart").unwrap();
    assert_ne!(client.overview().runtime_epoch, epoch);
    assert_eq!(client.overview().server_id, server_id);
    assert_eq!(client.overview().snapshot, snapshot);
    // Configured TCP and pairing identity survive the same restart.
    let remote = ClientConnection::connect_overview(&paired, "laptop").unwrap();
    assert_eq!(
        remote.overview().runtime_epoch,
        client.overview().runtime_epoch
    );
    drop(remote);
    drop(client);

    let stop = condr(&["server", "stop"]).status().unwrap();
    assert!(stop.success(), "stop failed: {stop}");
    wait_for_stop(&endpoint);

    let stopped_status = condr(&["server", "status"]).status().unwrap();
    assert_eq!(stopped_status.code(), Some(1));

    let restart = condr(&["server", "restart", "--snapshot"])
        .arg(&snapshot_path)
        .output()
        .unwrap();
    assert!(
        restart.status.success(),
        "restart of stopped Server failed: {}",
        String::from_utf8_lossy(&restart.stderr)
    );
    let client = ClientConnection::connect_overview(&endpoint, "started-by-restart").unwrap();
    assert_eq!(client.overview().snapshot, snapshot);
    drop(client);
    stop_server(&endpoint).unwrap();
    wait_for_stop(&endpoint);
    guard.disarm();

    let _ = std::fs::remove_file(log_path);
    std::fs::remove_dir_all(data_directory).unwrap();
}

#[test]
fn detached_server_launcher_helper() {
    if std::env::var_os(DETACHED_HELPER_ENV).is_some() {
        ensure_local_server().unwrap();
    }
}

/// The Server runs in its Supervisor's session, which left the launcher's (ADR 0058).
#[cfg(unix)]
fn assert_left_launcher_session(runtime_epoch: condr_core::protocol::RuntimeEpoch) {
    let server = server_pid(runtime_epoch);
    assert_ne!(
        nix::unistd::getsid(Some(server)).unwrap(),
        nix::unistd::getsid(None).unwrap()
    );
}

/// The Server's process ID, which its Runtime Epoch carries in its top bits.
#[cfg(unix)]
fn server_pid(runtime_epoch: condr_core::protocol::RuntimeEpoch) -> nix::unistd::Pid {
    nix::unistd::Pid::from_raw(i32::try_from(runtime_epoch.0 >> 96).unwrap())
}

/// A Server that aborts (ADR 0053) is started again by its Supervisor, which reports the
/// crash; one ended by SIGTERM stays down, and its Supervisor exits with it; and `stop`
/// asked while a Supervisor waits to restart a crashed Server stops for good (ADR 0058).
#[cfg(unix)]
#[test]
fn the_supervisor_restarts_a_crashed_server_and_not_a_stopped_one() {
    use nix::sys::signal::{Signal, kill};

    let endpoint_path = std::env::temp_dir().join(format!(
        "condr-supervisor-test-{}-{}.sock",
        std::process::id(),
        unique_suffix()
    ));
    let config_directory = endpoint_path.with_extension("config");
    let snapshot_path = endpoint_path.with_extension("snapshot");
    let with_test_host = |command: &mut Command| {
        command
            .env("CONDR_SOCKET_PATH", &endpoint_path)
            .env("CONDR_CONFIG_DIR", &config_directory)
            .env("CONDR_DATA_DIR", &config_directory)
            .env("CONDR_LOG_DIR", &config_directory)
            .env("CONDR_SNAPSHOT_PATH", &snapshot_path)
            .env("CONDR_SERVER_EXECUTABLE", env!("CARGO_BIN_EXE_condr"))
            .env_remove("CONDR_PANE_ID");
    };
    let launch = || {
        let mut launcher = Command::new(std::env::current_exe().unwrap());
        launcher
            .arg("--exact")
            .arg("detached_server_launcher_helper")
            .arg("--nocapture")
            .env(DETACHED_HELPER_ENV, "1");
        with_test_host(&mut launcher);
        assert!(launcher.status().unwrap().success());
    };
    launch();
    let endpoint = Endpoint::local(&endpoint_path);
    let guard = ServerGuard(endpoint.clone());

    let crashed = runtime_epoch(&endpoint);
    kill(server_pid(crashed), Signal::SIGABRT).unwrap();
    // macOS writes a crash report before the process is gone.
    let restarted = (0..300)
        .find_map(|_| {
            thread::sleep(Duration::from_millis(50));
            ClientConnection::connect_overview(&endpoint, "after-crash")
                .ok()
                .map(|client| client.overview().runtime_epoch)
                .filter(|epoch| *epoch != crashed)
        })
        .expect("the Supervisor started the Server again");

    let mut status = Command::new(env!("CARGO_BIN_EXE_condr"));
    status.args(["server", "status", "--json"]);
    with_test_host(&mut status);
    let status = status.output().unwrap();
    let status = String::from_utf8_lossy(&status.stdout);
    assert!(
        status
            .contains("restarted by its Supervisor after the previous Server was ended by SIGABRT"),
        "status reports the crash:\n{status}"
    );

    kill(server_pid(restarted), Signal::SIGTERM).unwrap();
    wait_for_stop(&endpoint);
    // Both the bind lock and the Supervisor's are released, and nothing came back.
    condr_server::wait_for_shutdown(&endpoint_path).unwrap();
    thread::sleep(Duration::from_millis(1500));
    assert!(
        endpoint.connect().is_err(),
        "a stopped Server is not restarted"
    );

    launch();
    kill(server_pid(runtime_epoch(&endpoint)), Signal::SIGABRT).unwrap();
    let mut stop = Command::new(env!("CARGO_BIN_EXE_condr"));
    stop.args(["server", "stop"]);
    with_test_host(&mut stop);
    let asked = std::time::Instant::now();
    let stop = stop.output().unwrap();
    assert!(
        stop.status.success(),
        "stop failed: {}",
        String::from_utf8_lossy(&stop.stderr)
    );
    assert!(
        asked.elapsed() < Duration::from_secs(20),
        "stop waited it out"
    );
    thread::sleep(Duration::from_millis(1500));
    assert!(
        endpoint.connect().is_err(),
        "the Server its Supervisor restarted was stopped too"
    );
    condr_server::wait_for_shutdown(&endpoint_path).unwrap();
    guard.disarm();

    let _ = std::fs::remove_dir_all(&config_directory);
    let _ = std::fs::remove_file(&snapshot_path);
    let mut snapshot_lock = snapshot_path.into_os_string();
    snapshot_lock.push(".lock");
    let _ = std::fs::remove_file(snapshot_lock);
}

#[cfg(unix)]
fn runtime_epoch(endpoint: &Endpoint) -> condr_core::protocol::RuntimeEpoch {
    ClientConnection::connect_overview(endpoint, "epoch")
        .unwrap()
        .overview()
        .runtime_epoch
}

/// The detached Server's stderr file under `directory`, the `CONDR_LOG_DIR` its launcher was given.
fn server_log_path(directory: &std::path::Path, server_id: ServerId) -> std::path::PathBuf {
    directory.join(format!("condr-server-{:016x}.stderr", server_id.0))
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

fn read_layout_response(stream: &mut condr_server::EndpointStream) -> ServerMessage {
    loop {
        let message = read_server(stream);
        if !matches!(
            message,
            ServerMessage::Event { .. } | ServerMessage::TerminalFrame(_)
        ) {
            return message;
        }
    }
}

fn read_server(stream: &mut condr_server::EndpointStream) -> ServerMessage {
    condr_core::protocol::read_message(stream).unwrap()
}
