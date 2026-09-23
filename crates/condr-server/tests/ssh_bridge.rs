use std::io::Read;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use condr_core::protocol::{
    self, ClientHandshake, ClientMessage, Hello, PROTOCOL_VERSION, Refusal, ServerMessage, Welcome,
};
use condr_server::{BoundServer, ClientConnection, Endpoint, ServerConfig};

mod common;
use common::unique_suffix;

struct Bridge {
    child: Child,
    input: Option<ChildStdin>,
    output: mpsc::Receiver<Result<Vec<u8>, String>>,
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
        // Frames are relayed raw so the test decodes each as the type it expects.
        thread::spawn(move || {
            loop {
                let frame = (|| {
                    // The varint length, then that many bytes (ADR 0028).
                    let mut frame = Vec::new();
                    let (mut length, mut shift) = (0usize, 0);
                    loop {
                        let mut byte = [0];
                        stdout.read_exact(&mut byte)?;
                        frame.push(byte[0]);
                        length |= usize::from(byte[0] & 0x7f) << shift;
                        shift += 7;
                        if byte[0] & 0x80 == 0 {
                            break;
                        }
                    }
                    let start = frame.len();
                    frame.resize(start + length, 0);
                    stdout.read_exact(&mut frame[start..])?;
                    Ok::<_, std::io::Error>(frame)
                })()
                .map_err(|error| error.to_string());
                let failed = frame.is_err();
                if tx.send(frame).is_err() || failed {
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

    fn send<M: protocol::WireMessage>(&mut self, message: &M) {
        protocol::write_message(self.input.as_mut().unwrap(), message).unwrap();
    }

    fn read<M: protocol::WireMessage>(&self) -> M {
        let frame = self
            .output
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap();
        protocol::read_message(&mut frame.as_slice()).unwrap()
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
    let root = std::env::temp_dir().join(format!("condr-bridge-{}", unique_suffix()));
    std::fs::create_dir(&root).unwrap();
    let endpoint = Endpoint::local(root.join("server.sock"));
    // An explicit `--endpoint` only connects (ADR 0015): nothing listening is an error,
    // not a reason to start a Server there.
    let missing = Command::new(env!("CARGO_BIN_EXE_condr"))
        .args(["server", "bridge", "--endpoint"])
        .arg(endpoint.as_local_path().unwrap())
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!missing.status.success());
    assert!(missing.stdout.is_empty());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("condr server start"));
    assert!(
        !endpoint.as_local_path().unwrap().exists(),
        "an explicit endpoint must not start a Server"
    );

    let server =
        BoundServer::bind(ServerConfig::ephemeral(endpoint.as_local_path().unwrap())).unwrap();
    let handle = server.handle();
    let runtime = thread::spawn(move || server.run().unwrap());
    let direct = ClientConnection::connect(&endpoint, "direct").unwrap();
    let expected = direct.bootstrap().unwrap().clone();
    for close_input in [false, true] {
        let mut bridge = Bridge::start(&endpoint);
        bridge.send(&ClientHandshake::Hello(Hello::new("ssh")));
        let welcome: Welcome = bridge.read();
        assert_eq!(welcome.refusal, None);
        assert_eq!(welcome.server_id, expected.server_id);
        assert_eq!(welcome.session_id, expected.session_id);
        bridge.send(&ClientMessage::SnapshotRequest {
            session_id: expected.session_id,
        });
        let ServerMessage::Bootstrap(bootstrap) = bridge.read::<ServerMessage>() else {
            panic!("expected Bootstrap");
        };
        assert_eq!(bootstrap.runtime_epoch, expected.runtime_epoch);
        for _ in 0..bootstrap.batch_count {
            assert!(matches!(
                bridge.read::<ServerMessage>(),
                ServerMessage::BootstrapBatch(_)
            ));
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
    incompatible.send(&ClientHandshake::Hello(Hello {
        min_server_protocol: PROTOCOL_VERSION + 1,
        ..Hello::new("newer")
    }));
    let welcome: Welcome = incompatible.read();
    assert_eq!(welcome.refusal, Some(Refusal::IncompatibleProtocol));
    incompatible.wait_exit();
    drop(incompatible);
    drop(direct);
    handle.stop();
    runtime.join().unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

/// On the Server's own socket the bridge behaves like the GUI on its own machine (ADR
/// 0015): nobody listening means the Server is started and connected to. The Server then
/// outlives the bridge, so the test stops it the way `condr server stop` does.
#[test]
fn bridge_starts_the_server_on_the_default_socket_when_none_runs() {
    let root = std::env::temp_dir().join(format!("condr-bridge-start-{}", unique_suffix()));
    let config = root.join("config");
    std::fs::create_dir_all(&config).unwrap();
    let socket = root.join("server.sock");
    let condr = |args: &[&str]| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_condr"));
        command
            .args(args)
            .env("CONDR_SOCKET_PATH", &socket)
            .env("CONDR_CONFIG_DIR", &config)
            .env("CONDR_LOG_DIR", &config)
            .env_remove("CONDR_PANE_ID")
            .env_remove("CONDR_ENV");
        command
    };

    let mut bridge = condr(&["server", "bridge"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut input = bridge.stdin.take().unwrap();
    let mut stdout = bridge.stdout.take().unwrap();
    protocol::write_message(&mut input, &ClientHandshake::Hello(Hello::new("ssh"))).unwrap();
    let welcome: Welcome = protocol::read_message(&mut stdout).unwrap();
    assert_eq!(
        welcome.refusal, None,
        "the bridge should have started a Server and relayed its Welcome: {welcome:?}"
    );
    assert!(
        socket.exists(),
        "the started Server owns the default socket"
    );
    drop(input);
    for _ in 0..200 {
        if bridge.try_wait().unwrap().is_some() {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    let _ = bridge.kill();
    let _ = bridge.wait();

    let stop = condr(&["server", "stop"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(
        stop.status.success(),
        "the Server the bridge started keeps running until stopped: {}",
        String::from_utf8_lossy(&stop.stderr)
    );
    for _ in 0..200 {
        if std::fs::remove_dir_all(&root).is_ok() {
            return;
        }
        thread::sleep(Duration::from_millis(25));
    }
    panic!("the stopped Server should release {}", root.display());
}
