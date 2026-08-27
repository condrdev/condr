use std::process::Command;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use murmur_server::{ClientConnection, Endpoint, ServerConfig, ensure_local_server, stop_server};

const HELPER_ENV: &str = "MURMUR_LOCAL_SERVER_TEST_HELPER";

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
    let output = Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("local_server_helper")
        .arg("--nocapture")
        .env(HELPER_ENV, "1")
        .env("MURMUR_SOCKET_PATH", &endpoint_path)
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
    drop(first);

    thread::sleep(Duration::from_millis(30));
    let second_endpoint = ensure_local_server().unwrap();
    assert_eq!(second_endpoint, first_endpoint);
    let second = ClientConnection::connect(&second_endpoint, "second-process-client").unwrap();
    assert_eq!(second.bootstrap().server_id, server_id);
    assert_eq!(second.bootstrap().runtime_epoch, runtime_epoch);
    drop(second);

    stop_server(&first_endpoint).unwrap();
    wait_for_stop(&first_endpoint);
    guard.disarm();

    let restarted_endpoint = ensure_local_server().unwrap();
    let restarted_guard = ServerGuard(restarted_endpoint.clone());
    let restarted = ClientConnection::connect(&restarted_endpoint, "restarted-client").unwrap();
    assert_eq!(restarted.bootstrap().server_id, server_id);
    assert_ne!(restarted.bootstrap().runtime_epoch, runtime_epoch);
    drop(restarted);

    stop_server(&restarted_endpoint).unwrap();
    wait_for_stop(&restarted_endpoint);
    restarted_guard.disarm();
    let _ = restarted_endpoint.cleanup();
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

fn unique_suffix() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}
