//! The Server's end of a Stream connection (ADR 0061), over the local socket and TCP.

use super::*;
use condr_client::{Stream, StreamConnection};
use condr_core::WorkspaceId;
use condr_core::protocol::{STREAM_WINDOW, StreamError, StreamFile, StreamTarget};
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener};

/// A loopback service that echoes each connection until the peer finishes, then closes.
fn echo_service() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        for socket in listener.incoming() {
            let Ok(mut socket) = socket else { return };
            thread::spawn(move || {
                let mut buffer = [0; 8192];
                loop {
                    match socket.read(&mut buffer) {
                        Ok(0) | Err(_) => break,
                        Ok(read) => socket.write_all(&buffer[..read]).unwrap(),
                    }
                }
                let _ = socket.shutdown(Shutdown::Write);
            });
        }
    });
    port
}

/// One `Ping` and its `Pong` on a terminal connection: a round trip that starts no
/// process, so no fork can hold another test's socket open.
fn ping(stream: &mut EndpointStream, server_id: ServerId, nonce: u64) {
    condr_core::protocol::write_message(stream, &ClientMessage::Ping { server_id, nonce }).unwrap();
    loop {
        match condr_core::protocol::read_message::<_, ServerMessage>(stream).unwrap() {
            ServerMessage::Pong {
                nonce: answered, ..
            } if answered == nonce => return,
            _ => {}
        }
    }
}

fn read_all(stream: &Stream) -> Vec<u8> {
    let mut all = Vec::new();
    while let Some(data) = stream.read().unwrap() {
        all.extend(data);
    }
    all
}

fn forward(port: u16) -> StreamTarget {
    StreamTarget::Forward { port: port.into() }
}

fn round_trip_through_a_forward(endpoint: &Endpoint) {
    let (connection, welcome) = StreamConnection::connect(endpoint, "streams-test").unwrap();
    assert_eq!(welcome.refusal, None);
    let stream = Arc::new(connection.open(forward(echo_service())).unwrap());
    assert_eq!(stream.opened().unwrap(), None);
    let payload: Vec<u8> = (0..2 * STREAM_WINDOW as usize + 7)
        .map(|i| i as u8)
        .collect();
    let writer = {
        let stream = Arc::clone(&stream);
        let payload = payload.clone();
        thread::spawn(move || {
            stream.write(&payload).unwrap();
            stream.finish();
        })
    };
    assert_eq!(read_all(&stream), payload);
    writer.join().unwrap();
}

#[test]
fn a_forward_round_trips_over_the_local_socket() {
    let (handle, endpoint, thread) = start();
    round_trip_through_a_forward(&endpoint);
    handle.stop();
    thread.join().unwrap().unwrap();
}

#[test]
fn a_forward_round_trips_over_tcp_and_the_stream_connection_is_a_registered_peer() {
    let server =
        BoundServer::bind(ServerConfig::ephemeral_tcp("127.0.0.1:0".parse().unwrap()).unwrap())
            .unwrap();
    let endpoint = server.endpoint().clone();
    let handle = server.handle();
    let thread = thread::spawn(move || server.run());
    round_trip_through_a_forward(&endpoint);
    let (connection, _) = StreamConnection::connect(&endpoint, "registered").unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    // RevokeDevice finds Stream connections in the same table as terminal ones.
    while handle.state.lock().unwrap().tcp_peers.is_empty() {
        assert!(
            Instant::now() < deadline,
            "the Stream connection was not registered"
        );
        thread::sleep(Duration::from_millis(10));
    }
    drop(connection);
    while !handle.state.lock().unwrap().tcp_peers.is_empty() {
        assert!(
            Instant::now() < deadline,
            "the Stream connection's registration leaked"
        );
        thread::sleep(Duration::from_millis(10));
    }
    handle.stop();
    thread.join().unwrap().unwrap();
}

#[test]
fn opens_the_server_cannot_serve_fail_with_their_reason() {
    let (handle, endpoint, thread) = start();
    let (connection, _) = StreamConnection::connect(&endpoint, "streams-test").unwrap();
    let closed_port = {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    };
    let refused = connection.open(forward(closed_port)).unwrap();
    assert_eq!(
        refused.opened().unwrap_err().error,
        StreamError::ConnectionRefused
    );
    let invalid = connection.open(StreamTarget::Forward { port: 0 }).unwrap();
    assert_eq!(
        invalid.opened().unwrap_err().error,
        StreamError::InvalidPort
    );
    let too_high = connection
        .open(StreamTarget::Forward { port: 70_000 })
        .unwrap();
    assert_eq!(
        too_high.opened().unwrap_err().error,
        StreamError::InvalidPort
    );
    // A Workspace that does not exist.
    let download = connection
        .open(StreamTarget::Download(StreamFile {
            workspace: WorkspaceId::from_u64(1),
            path: "a".into(),
        }))
        .unwrap();
    assert_eq!(
        download.opened().unwrap_err().error,
        StreamError::WorkspaceGone
    );
    assert!(!connection.is_closed());
    handle.stop();
    thread.join().unwrap().unwrap();
}

#[test]
fn a_response_delimited_by_closing_arrives_whole() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        socket.read_to_end(&mut request).unwrap();
        socket.write_all(b"HTTP/1.0 200 OK\r\n\r\n").unwrap();
        socket.write_all(&request).unwrap();
    });
    let (handle, endpoint, thread) = start();
    let (connection, _) = StreamConnection::connect(&endpoint, "streams-test").unwrap();
    let stream = connection.open(forward(port)).unwrap();
    stream.write(b"GET / HTTP/1.0\r\n\r\n").unwrap();
    stream.finish();
    assert_eq!(
        read_all(&stream),
        b"HTTP/1.0 200 OK\r\n\r\nGET / HTTP/1.0\r\n\r\n"
    );
    handle.stop();
    thread.join().unwrap().unwrap();
}

#[test]
fn a_forward_reaches_a_port_listening_on_ipv6_loopback_alone() {
    let Ok(listener) = TcpListener::bind("[::1]:0") else {
        return; // No IPv6 loopback here.
    };
    let port = listener.local_addr().unwrap().port();
    if TcpListener::bind(("127.0.0.1", port)).is_err() {
        return; // Something else holds the IPv4 side; the test would prove nothing.
    }
    thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket.write_all(b"v6").unwrap();
    });
    let (handle, endpoint, thread) = start();
    let (connection, _) = StreamConnection::connect(&endpoint, "streams-test").unwrap();
    let stream = connection.open(forward(port)).unwrap();
    stream.finish();
    assert_eq!(read_all(&stream), b"v6");
    handle.stop();
    thread.join().unwrap().unwrap();
}

/// The hard constraint of ADR 0061: a Stream that cannot move, in either direction,
/// leaves the same Client's terminal connection answering.
#[test]
fn stalled_streams_leave_the_terminal_connection_answering() {
    // Accepts and never reads, so the Client's uploads stall at the Server's socket.
    let sink = TcpListener::bind("127.0.0.1:0").unwrap();
    let sink_port = sink.local_addr().unwrap().port();
    let held = Arc::new(Mutex::new(Vec::new()));
    {
        let held = Arc::clone(&held);
        thread::spawn(move || {
            for socket in sink.incoming().flatten() {
                held.lock().unwrap().push(socket);
            }
        });
    }
    // Sends far more than any window, so the Server's sends stall at the Client.
    let flood = TcpListener::bind("127.0.0.1:0").unwrap();
    let flood_port = flood.local_addr().unwrap().port();
    thread::spawn(move || {
        for mut socket in flood.incoming().flatten() {
            thread::spawn(move || {
                let block = vec![7; 64 * 1024];
                while socket.write_all(&block).is_ok() {}
            });
        }
    });

    let (handle, endpoint, thread) = start();
    let terminal = ClientConnection::connect_overview(&endpoint, "terminal").unwrap();
    let server_id = terminal.overview().server_id;
    let mut terminal = terminal.into_stream();
    let (streams, _) = StreamConnection::connect(&endpoint, "streams").unwrap();
    let upload = Arc::new(streams.open(forward(sink_port)).unwrap());
    upload.opened().unwrap();
    let uploading = {
        let upload = Arc::clone(&upload);
        thread::spawn(move || {
            let _ = upload.write(&vec![0; 64 * STREAM_WINDOW as usize]);
        })
    };
    let download = streams.open(forward(flood_port)).unwrap();
    download.opened().unwrap();
    // Let both stall: the upload behind the sink, the download behind this unread handle.
    thread::sleep(Duration::from_millis(500));
    assert!(
        !uploading.is_finished(),
        "the upload should be waiting for credit"
    );

    for nonce in 0..5 {
        let started = Instant::now();
        ping(&mut terminal, server_id, nonce);
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "a request on the terminal connection took {:?}",
            started.elapsed()
        );
    }
    // Another Stream on the same connection still flows.
    let lively = streams.open(forward(echo_service())).unwrap();
    lively.write(b"lively").unwrap();
    lively.finish();
    assert_eq!(read_all(&lively), b"lively");

    drop(download);
    drop(streams);
    let _ = uploading.join();
    handle.stop();
    thread.join().unwrap().unwrap();
}

/// Not a gate (ADR 0061): how a 1 GiB download on the Stream connection moves a round
/// trip on the terminal connection. Loopback has no bottleneck queue, so this shows only
/// what Condr itself adds. Run with
/// `cargo test --release -p condr-server --lib terminal_round_trip -- --ignored --nocapture`.
#[test]
#[ignore = "benchmark; run in release with --ignored --nocapture"]
fn terminal_round_trip_with_and_without_a_concurrent_download() {
    use condr_core::protocol::{LayoutCommand, LayoutResult};
    let root = std::env::temp_dir().join(format!("condr-bench-{}", unique_suffix()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::File::create(root.join("big.bin"))
        .unwrap()
        .set_len(1 << 30)
        .unwrap();
    let (handle, endpoint, thread) = start();
    let mut terminal = ClientConnection::connect_overview(&endpoint, "terminal").unwrap();
    let LayoutResult::WorkspaceCreated { workspace_id, .. } = terminal
        .layout(LayoutCommand::CreateWorkspace {
            root_directory: root.clone(),
            name: None,
        })
        .unwrap()
        .unwrap()
    else {
        panic!("expected a Workspace");
    };
    let server_id = terminal.overview().server_id;
    let mut terminal = terminal.into_stream();
    let mut nonce = 0;
    let mut measure = |label: &str| {
        let mut samples: Vec<Duration> = (0..200)
            .map(|_| {
                let started = Instant::now();
                nonce += 1;
                ping(&mut terminal, server_id, nonce);
                started.elapsed()
            })
            .collect();
        samples.sort();
        println!(
            "{label}: p50 {:?}, p99 {:?}",
            samples[samples.len() / 2],
            samples[samples.len() * 99 / 100]
        );
    };
    measure("idle");
    let (streams, _) = StreamConnection::connect(&endpoint, "streams").unwrap();
    let destination = root.join("copy.bin");
    let downloading = thread::spawn(move || {
        condr_client::download(
            &streams,
            StreamFile {
                workspace: workspace_id,
                path: "big.bin".into(),
            },
            &destination,
            false,
            &condr_client::TransferCancel::default(),
            |_, _| {},
        )
    });
    thread::sleep(Duration::from_millis(200));
    measure("during a 1 GiB download");
    let bytes = downloading.join().unwrap().unwrap();
    println!("downloaded {bytes} bytes");
    handle.stop();
    thread.join().unwrap().unwrap();
    std::fs::remove_dir_all(root).unwrap();
}
