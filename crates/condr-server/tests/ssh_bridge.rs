use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use condr_core::protocol::{self, ClientMessage, Hello, PROTOCOL_VERSION, ServerMessage};
use condr_server::{BoundServer, ClientConnection, Endpoint, ServerConfig};

struct Bridge {
    child: Child,
    input: Option<ChildStdin>,
    output: mpsc::Receiver<Result<ServerMessage, String>>,
}

impl Bridge {
    fn start(endpoint: &Endpoint) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_condr"))
            .args(["server", "bridge", "--endpoint"])
            .arg(endpoint.as_local_path().unwrap())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let input = child.stdin.take();
        let mut stdout = child.stdout.take().unwrap();
        let (tx, output) = mpsc::channel();
        thread::spawn(move || {
            loop {
                let message =
                    protocol::read_message(&mut stdout).map_err(|error| error.to_string());
                let failed = message.is_err();
                if tx.send(message).is_err() || failed {
                    break;
                }
            }
        });
        Self {
            child,
            input,
            output,
        }
    }

    fn send(&mut self, message: &ClientMessage) {
        protocol::write_message(self.input.as_mut().unwrap(), message).unwrap();
    }

    fn read(&self) -> ServerMessage {
        self.output
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap()
    }

    fn wait_exit(&mut self) {
        for _ in 0..200 {
            if self.child.try_wait().unwrap().is_some() {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("bridge did not exit after a direction closed");
    }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn bridge_uses_only_the_running_local_server_and_reconnects_to_its_session() {
    let root = std::env::temp_dir().join(format!("condr-bridge-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let endpoint = Endpoint::local(root.join("server.sock"));
    let missing = Command::new(env!("CARGO_BIN_EXE_condr"))
        .args(["server", "bridge"])
        .env("CONDR_SOCKET_PATH", endpoint.as_local_path().unwrap())
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!missing.status.success());
    assert!(missing.stdout.is_empty());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("condr server start"));
    assert!(
        !endpoint.as_local_path().unwrap().exists(),
        "bridge must not start a Server"
    );

    let server =
        BoundServer::bind(ServerConfig::ephemeral(endpoint.as_local_path().unwrap())).unwrap();
    let handle = server.handle();
    let runtime = thread::spawn(move || server.run().unwrap());
    let direct = ClientConnection::connect(&endpoint, "direct").unwrap();
    let expected = direct.bootstrap().unwrap().clone();
    for close_input in [false, true] {
        let mut bridge = Bridge::start(&endpoint);
        bridge.send(&ClientMessage::Hello(Hello {
            version: PROTOCOL_VERSION,
            client_name: "ssh".into(),
        }));
        assert!(
            matches!(bridge.read(), ServerMessage::Welcome { server_id, session_id, error: None, .. }
            if server_id == expected.server_id && session_id == expected.session_id)
        );
        bridge.send(&ClientMessage::SnapshotRequest {
            session_id: expected.session_id,
        });
        let ServerMessage::Bootstrap(bootstrap) = bridge.read() else {
            panic!("expected Bootstrap");
        };
        assert_eq!(bootstrap.runtime_epoch, expected.runtime_epoch);
        for _ in 0..bootstrap.batch_count {
            assert!(matches!(bridge.read(), ServerMessage::BootstrapBatch(_)));
        }
        if close_input {
            drop(bridge.input.take());
        } else {
            bridge.send(&ClientMessage::Detach);
        }
        bridge.wait_exit();
        assert_eq!(
            ClientConnection::connect(&endpoint, "reconnect")
                .unwrap()
                .bootstrap()
                .unwrap()
                .runtime_epoch,
            expected.runtime_epoch
        );
    }
    let mut incompatible = Bridge::start(&endpoint);
    incompatible.send(&ClientMessage::Hello(Hello {
        version: PROTOCOL_VERSION + 1,
        client_name: "old".into(),
    }));
    assert!(matches!(
        incompatible.read(),
        ServerMessage::Welcome { error: Some(_), .. }
    ));
    incompatible.wait_exit();
    drop(incompatible);
    drop(direct);
    handle.stop();
    runtime.join().unwrap();
    std::fs::remove_dir_all(root).unwrap();
}
