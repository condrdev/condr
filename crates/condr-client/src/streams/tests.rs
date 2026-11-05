use super::*;
use condr_core::protocol::{
    MAX_STREAM_FRAME_SIZE, StreamClientMessage, StreamMessage, StreamServerMessage,
    read_message_with_limit, write_message_with_limit,
};
use std::io::Write as _;
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::time::Instant;

/// A loopback TCP connection standing in for any transport.
struct Pipe(TcpStream);

impl io::Read for Pipe {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.0.read(buffer)
    }
}

impl io::Write for Pipe {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.0.write(buffer)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

impl ServerStream for Pipe {
    fn try_clone(&self) -> io::Result<Self> {
        self.0.try_clone().map(Pipe)
    }
    fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        self.0.set_read_timeout(timeout)
    }
    fn shutdown(&self) -> io::Result<()> {
        self.0.shutdown(std::net::Shutdown::Both)
    }
}

fn pipe() -> (Pipe, Pipe) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let (server, _) = listener.accept().unwrap();
    (Pipe(client), Pipe(server))
}

/// A Client and a Server whose `on_open` hands every Stream to the test.
fn connected() -> (
    StreamConnection,
    StreamConnection,
    mpsc::Receiver<(Stream, StreamTarget)>,
) {
    let (client, server) = pipe();
    let (opened, streams) = mpsc::channel();
    let opened = Mutex::new(opened);
    let server = StreamConnection::serve(server, move |stream, target| {
        let _ = opened.lock().unwrap().send((stream, target));
    })
    .unwrap();
    let client = StreamConnection::start(client, Side::Client, None, HEARTBEAT_INTERVAL).unwrap();
    (client, server, streams)
}

fn forward(port: u32) -> StreamTarget {
    StreamTarget::Forward { port }
}

/// Echoes everything back, then finishes once the Client did.
fn echo(stream: Stream) {
    thread::spawn(move || {
        stream.accept(None);
        while let Ok(Some(data)) = stream.read() {
            stream.write(&data).unwrap();
        }
        stream.finish();
        // Hold the handle until the Client has read the echo and closed its side.
        thread::sleep(Duration::from_millis(200));
    });
}

fn read_all(stream: &Stream) -> Vec<u8> {
    let mut all = Vec::new();
    while let Some(data) = stream.read().unwrap() {
        all.extend(data);
    }
    all
}

#[test]
fn bytes_beyond_one_window_flow_both_ways_as_credit_returns() {
    let (client, _server, streams) = connected();
    let stream = Arc::new(client.open(forward(1)).unwrap());
    echo(streams.recv().unwrap().0);
    assert_eq!(stream.opened().unwrap(), None);
    let payload: Vec<u8> = (0..3 * STREAM_WINDOW as usize).map(|i| i as u8).collect();
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
fn a_stream_whose_consumer_never_reads_holds_back_no_other() {
    let (client, _server, streams) = connected();
    let stalled = Arc::new(client.open(forward(1)).unwrap());
    // The Server takes the stalled Stream and never reads it.
    let (held, _) = streams.recv().unwrap();
    held.accept(None);
    let writer = {
        let stalled = Arc::clone(&stalled);
        thread::spawn(move || {
            let _ = stalled.write(&vec![0; 2 * STREAM_WINDOW as usize]);
        })
    };
    // Five more stalled Streams: there is no connection-wide window for them to fill.
    let mut more = Vec::new();
    for _ in 0..5 {
        let stream = Arc::new(client.open(forward(1)).unwrap());
        let (server_side, _) = streams.recv().unwrap();
        server_side.accept(None);
        let filler = Arc::clone(&stream);
        thread::spawn(move || {
            let _ = filler.write(&vec![0; STREAM_WINDOW as usize]);
        });
        more.push((stream, server_side));
    }
    let lively = client.open(forward(2)).unwrap();
    echo(streams.recv().unwrap().0);
    let started = Instant::now();
    lively.write(b"still flowing").unwrap();
    lively.finish();
    assert_eq!(read_all(&lively), b"still flowing");
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(!writer.is_finished(), "the stalled Stream waits for credit");
    drop(held);
    let _ = writer.join();
}

#[test]
fn half_close_leaves_the_other_direction_open() {
    let (client, _server, streams) = connected();
    let stream = client.open(forward(1)).unwrap();
    let (server_side, _) = streams.recv().unwrap();
    server_side.accept(None);
    stream.write(b"request").unwrap();
    stream.finish();
    assert_eq!(read_all(&server_side), b"request");
    // The Client finished; the Server still answers, then finishes in turn.
    server_side.write(b"response").unwrap();
    server_side.finish();
    assert_eq!(read_all(&stream), b"response");
}

#[test]
fn a_refused_open_fails_its_stream_and_nothing_else() {
    let (client, _server, streams) = connected();
    let refused = client.open(forward(1)).unwrap();
    let (server_side, _) = streams.recv().unwrap();
    server_side.refuse(StreamError::ConnectionRefused, "nothing there");
    drop(server_side);
    let failure = refused.opened().unwrap_err();
    assert_eq!(failure.error, StreamError::ConnectionRefused);
    assert!(refused.write(b"late").is_err());
    let next = client.open(forward(2)).unwrap();
    echo(streams.recv().unwrap().0);
    next.write(b"ok").unwrap();
    next.finish();
    assert_eq!(read_all(&next), b"ok");
}

#[test]
fn the_client_refuses_more_streams_than_the_connection_holds() {
    let (client, _server, streams) = connected();
    let mut open = Vec::new();
    for _ in 0..MAX_STREAMS {
        open.push(client.open(forward(1)).unwrap());
        open.push(streams.recv().unwrap().0);
    }
    let failure = client.open(forward(1)).err().unwrap();
    assert_eq!(failure.error, StreamError::TooManyStreams);
}

/// A raw Client, to send what the engine never would.
struct RawClient(Pipe);

impl RawClient {
    fn send(&mut self, message: StreamMessage) {
        write_message_with_limit(
            &mut self.0,
            &StreamClientMessage(message),
            MAX_STREAM_FRAME_SIZE,
        )
        .unwrap();
    }

    fn receive(&mut self) -> io::Result<StreamMessage> {
        read_message_with_limit::<_, StreamServerMessage>(&mut self.0, MAX_STREAM_FRAME_SIZE)
            .map(|(message, _)| message.0)
            .map_err(|error| io::Error::other(error.to_string()))
    }
}

fn raw_server() -> (
    RawClient,
    StreamConnection,
    mpsc::Receiver<(Stream, StreamTarget)>,
) {
    let (client, server) = pipe();
    client
        .0
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let (opened, streams) = mpsc::channel();
    let opened = Mutex::new(opened);
    let server = StreamConnection::serve(server, move |stream, target| {
        let _ = opened.lock().unwrap().send((stream, target));
    })
    .unwrap();
    (RawClient(client), server, streams)
}

#[test]
fn data_beyond_credit_closes_the_connection() {
    let (mut raw, server, streams) = raw_server();
    raw.send(StreamMessage::Open {
        stream: 1,
        target: forward(1),
    });
    let (_held, _) = streams.recv().unwrap();
    let chunk = vec![0; MAX_STREAM_CHUNK];
    for _ in 0..=(STREAM_WINDOW as usize / MAX_STREAM_CHUNK) {
        raw.send(StreamMessage::Data {
            stream: 1,
            data: chunk.clone(),
        });
    }
    assert!(server.wait_closed().contains("beyond its credit"));
}

#[test]
fn a_message_for_a_closed_stream_is_ignored_and_one_never_opened_closes() {
    let (mut raw, server, streams) = raw_server();
    raw.send(StreamMessage::Open {
        stream: 1,
        target: forward(1),
    });
    let (server_side, _) = streams.recv().unwrap();
    server_side.refuse(StreamError::ConnectionRefused, "");
    drop(server_side);
    assert!(matches!(
        raw.receive().unwrap(),
        StreamMessage::OpenFailed { stream: 1, .. }
    ));
    // Bytes the Client sent before it heard the refusal are dropped quietly.
    raw.send(StreamMessage::Data {
        stream: 1,
        data: vec![1; 16],
    });
    raw.send(StreamMessage::Finish { stream: 1 });
    raw.send(StreamMessage::Ping);
    assert_eq!(raw.receive().unwrap(), StreamMessage::Pong);
    assert!(!server.is_closed());
    raw.send(StreamMessage::Credit {
        stream: 9,
        max_offset: 1,
    });
    assert!(server.wait_closed().contains("never opened"));
}

#[test]
fn an_unknown_target_and_one_stream_too_many_are_answered_without_closing() {
    let (mut raw, server, streams) = raw_server();
    // An `Open` with no target, as a newer Client's new kind decodes here.
    raw.0.0.write_all(&[4, 0x0a, 0x02, 0x08, 0x01]).unwrap();
    assert!(matches!(
        raw.receive().unwrap(),
        StreamMessage::OpenFailed {
            stream: 1,
            error: StreamError::Unsupported,
            ..
        }
    ));
    let mut held = Vec::new();
    for stream in 2..2 + MAX_STREAMS as u64 {
        raw.send(StreamMessage::Open {
            stream,
            target: forward(1),
        });
        held.push(streams.recv().unwrap().0);
    }
    raw.send(StreamMessage::Open {
        stream: 100,
        target: forward(1),
    });
    assert!(matches!(
        raw.receive().unwrap(),
        StreamMessage::OpenFailed {
            stream: 100,
            error: StreamError::TooManyStreams,
            ..
        }
    ));
    assert!(!server.is_closed());
}

#[test]
fn dropping_the_connection_fails_every_stream_on_it() {
    let (client, server, streams) = connected();
    let stream = client.open(forward(1)).unwrap();
    let (_held, _) = streams.recv().unwrap();
    drop(server);
    assert!(stream.read().is_err());
    assert!(!client.wait_closed().is_empty());
}

/// A Server that sends a little data every 50 ms to a Client that only reads still hears
/// the Client ping on its clock; each wakeup must not push the next Ping back.
#[test]
fn the_client_pings_on_a_clock_while_it_only_receives() {
    let heartbeat = Duration::from_millis(200);
    let (client, mut server) = pipe();
    server
        .0
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let client = StreamConnection::start(client, Side::Client, None, heartbeat).unwrap();
    let stream = client.open(forward(1)).unwrap();
    let receive = |server: &mut Pipe| {
        read_message_with_limit::<_, StreamClientMessage>(server, MAX_STREAM_FRAME_SIZE)
            .map(|(message, _)| message.0)
            .unwrap()
    };
    assert!(matches!(receive(&mut server), StreamMessage::Open { .. }));
    let mut writer = Pipe(server.0.try_clone().unwrap());
    let sender = thread::spawn(move || {
        let send = |writer: &mut Pipe, message| {
            write_message_with_limit(writer, &StreamServerMessage(message), MAX_STREAM_FRAME_SIZE)
                .unwrap();
        };
        send(
            &mut writer,
            StreamMessage::Opened {
                stream: 1,
                size: None,
            },
        );
        for _ in 0..30 {
            send(
                &mut writer,
                StreamMessage::Data {
                    stream: 1,
                    data: vec![1; 10],
                },
            );
            thread::sleep(Duration::from_millis(50));
        }
    });
    let started = Instant::now();
    let mut pings = 0;
    while started.elapsed() < Duration::from_millis(1400) {
        if receive(&mut server) == StreamMessage::Ping {
            pings += 1;
        }
    }
    sender.join().unwrap();
    assert!(pings >= 4, "{pings} pings in 1.4 s at a 200 ms heartbeat");
    assert!(stream.read().unwrap().is_some());
}

/// A transport whose shutdown wakes nothing and whose reads never time out, like a
/// Windows named pipe.
struct Stubborn(TcpStream);

impl io::Read for Stubborn {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.0.read(buffer)
    }
}

impl io::Write for Stubborn {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.0.write(buffer)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

impl ServerStream for Stubborn {
    fn try_clone(&self) -> io::Result<Self> {
        self.0.try_clone().map(Stubborn)
    }
    fn set_read_timeout(&self, _: Option<Duration>) -> io::Result<()> {
        Ok(())
    }
    fn shutdown(&self) -> io::Result<()> {
        Ok(())
    }
}

/// Closing one end over such a transport still ends the other, which hears `Close`.
#[test]
fn closing_ends_the_peer_on_a_transport_that_cannot_shut_down() {
    for client_closes in [true, false] {
        let (client, server) = pipe();
        let server = StreamConnection::serve(Stubborn(server.0), |_, _| {}).unwrap();
        let client =
            StreamConnection::start(Stubborn(client.0), Side::Client, None, HEARTBEAT_INTERVAL)
                .unwrap();
        let (closing, peer) = if client_closes {
            (&client, &server)
        } else {
            (&server, &client)
        };
        let started = Instant::now();
        closing.close();
        assert_eq!(peer.wait_closed(), "the device closed the connection");
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
