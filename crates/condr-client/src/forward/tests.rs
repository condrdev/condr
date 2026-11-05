use super::*;
use crate::ServerStream;
use std::sync::mpsc;
use std::time::Instant;

/// A loopback TCP connection standing in for any transport.
struct Pipe(TcpStream);

impl Read for Pipe {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.0.read(buffer)
    }
}

impl Write for Pipe {
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
        self.0.shutdown(Shutdown::Both)
    }
}

/// A Stream connection whose Server end forwards to this machine's loopback, as the real
/// Server does.
fn stream_connection() -> io::Result<StreamConnection> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let client = TcpStream::connect(listener.local_addr()?)?;
    let (server, _) = listener.accept()?;
    let server = StreamConnection::serve(Pipe(server), |stream, target| {
        let StreamTarget::Forward { port } = target else {
            return stream.refuse(StreamError::Unsupported, "");
        };
        thread::spawn(move || {
            let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port as u16));
            match TcpStream::connect(address) {
                Ok(socket) => {
                    stream.accept(None);
                    splice(&Arc::new(stream), socket);
                }
                Err(_) => stream.refuse(StreamError::ConnectionRefused, "nothing there"),
            }
        });
    })?;
    // The Server end lives as long as the Client end's process here.
    std::mem::forget(server);
    StreamConnection::start_client_for_tests(Pipe(client))
}

fn echo_service() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        for mut socket in listener.incoming().flatten() {
            thread::spawn(move || {
                let mut buffer = [0; 4096];
                while let Ok(read) = socket.read(&mut buffer) {
                    if read == 0 || socket.write_all(&buffer[..read]).is_err() {
                        break;
                    }
                }
            });
        }
    });
    port
}

fn closed_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

#[test]
fn a_taken_port_is_refused_and_the_gui_takes_the_next_free_one() {
    let held = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = held.local_addr().unwrap().port();
    let error = bind_loopback(port).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::AddrInUse);
    let listeners = near_port(port).unwrap();
    assert_ne!(listeners[0].local_addr().unwrap().port(), port);
}

#[test]
fn a_port_whose_ipv6_loopback_another_program_holds_is_taken() {
    let Ok(held) = TcpListener::bind("[::1]:0") else {
        return; // No IPv6 loopback here.
    };
    let port = held.local_addr().unwrap().port();
    // Held on `::1` only: a browser opening `localhost` might reach it first.
    let error = bind_loopback(port).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::AddrInUse);
}

#[test]
fn a_forward_carries_a_connection_to_the_devices_port_and_counts_it() {
    let opener: Arc<dyn StreamOpener> = Arc::new(stream_connection().unwrap());
    let (events, received) = mpsc::channel();
    let events = Mutex::new(events);
    let forward = Forward::bind(echo_service(), LocalPort::Exact(0), opener, move |event| {
        let _ = events.lock().unwrap().send(event);
    })
    .unwrap();
    let mut socket = TcpStream::connect(("127.0.0.1", forward.local_port())).unwrap();
    socket.write_all(b"hello").unwrap();
    let mut answer = [0; 5];
    socket.read_exact(&mut answer).unwrap();
    assert_eq!(&answer, b"hello");
    assert_eq!(forward.connections(), 1);
    drop(socket);
    let deadline = Instant::now() + Duration::from_secs(5);
    while forward.connections() != 0 {
        assert!(
            Instant::now() < deadline,
            "the closed connection was not counted out"
        );
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        received
            .try_iter()
            .all(|event| event == ForwardEvent::Changed)
    );
}

#[test]
fn a_refused_port_closes_the_browsers_connection_and_says_so() {
    let opener: Arc<dyn StreamOpener> = Arc::new(stream_connection().unwrap());
    let (events, received) = mpsc::channel();
    let events = Mutex::new(events);
    let forward = Forward::bind(closed_port(), LocalPort::Exact(0), opener, move |event| {
        let _ = events.lock().unwrap().send(event);
    })
    .unwrap();
    let mut socket = TcpStream::connect(("127.0.0.1", forward.local_port())).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut buffer = [0; 1];
    assert!(matches!(socket.read(&mut buffer), Ok(0) | Err(_)));
    let refused = received
        .iter()
        .find(|event| matches!(event, ForwardEvent::Refused(_)));
    assert!(refused.is_some());
    assert!(forward.last_refused());
}

#[test]
fn a_connection_accepted_while_reconnecting_is_closed_at_once() {
    struct Reconnecting;
    impl StreamOpener for Reconnecting {
        fn open(&self, _: StreamTarget) -> Result<Stream, StreamFailure> {
            Err(StreamFailure {
                error: StreamError::Other,
                detail: "reconnecting".into(),
            })
        }
    }
    let forward = Forward::bind(1, LocalPort::Exact(0), Arc::new(Reconnecting), |_| {}).unwrap();
    let mut socket = TcpStream::connect(("127.0.0.1", forward.local_port())).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let started = Instant::now();
    let mut buffer = [0; 1];
    assert!(matches!(socket.read(&mut buffer), Ok(0) | Err(_)));
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "closed, not queued"
    );
}

#[test]
fn a_link_dials_once_reconnects_after_a_loss_and_stops_on_a_refusal() {
    let dials = Arc::new(AtomicUsize::new(0));
    let refuse = Arc::new(AtomicBool::new(false));
    let link = {
        let dials = Arc::clone(&dials);
        let refuse = Arc::clone(&refuse);
        StreamLink::new(
            move || {
                dials.fetch_add(1, Ordering::AcqRel);
                if refuse.load(Ordering::Acquire) {
                    return Err(Refused {
                        refusal: condr_core::protocol::Refusal::DeviceRevoked,
                        server_protocol: 1,
                        server_build: "test".into(),
                    }
                    .into());
                }
                stream_connection()
            },
            || {},
        )
    };
    link.retain().unwrap();
    link.retain().unwrap();
    assert_eq!(
        dials.load(Ordering::Acquire),
        1,
        "a second forward shares the connection"
    );
    assert_eq!(link.status(), LinkStatus::Connected);

    // The terminal connection drops and returns: the link dials again at once.
    link.set_terminal_connected(false);
    assert_eq!(link.status(), LinkStatus::Idle);
    link.set_terminal_connected(true);
    let deadline = Instant::now() + Duration::from_secs(5);
    while link.status() != LinkStatus::Connected {
        assert!(Instant::now() < deadline, "the link did not reconnect");
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(dials.load(Ordering::Acquire), 2);

    // A refusal ends the retries.
    refuse.store(true, Ordering::Release);
    link.set_terminal_connected(false);
    link.set_terminal_connected(true);
    while !matches!(link.status(), LinkStatus::Refused(_)) {
        assert!(Instant::now() < deadline, "the refusal was not reported");
        thread::sleep(Duration::from_millis(10));
    }
    let after = dials.load(Ordering::Acquire);
    thread::sleep(Duration::from_millis(2500));
    assert_eq!(
        dials.load(Ordering::Acquire),
        after,
        "no retry after a refusal"
    );

    link.release();
    link.release();
    assert_eq!(link.status(), LinkStatus::Idle);
}
