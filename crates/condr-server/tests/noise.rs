use condr_server::noise::{NoiseStream, Secret, ServerIdentity, StaticKey};
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;

fn pair(
    identity: &Arc<ServerIdentity>,
    client: &StaticKey,
    invite: Option<&Secret>,
) -> (NoiseStream, thread::JoinHandle<io::Result<NoiseStream>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let responder = Arc::clone(identity);
    let server = thread::spawn(move || {
        let (socket, _) = listener.accept()?;
        NoiseStream::responder(socket, responder)
    });
    let socket = TcpStream::connect(address).unwrap();
    let client = NoiseStream::initiator(socket, &identity.public_key(), client, invite).unwrap();
    (client, server)
}

fn identity() -> (Arc<ServerIdentity>, StaticKey) {
    let client = StaticKey::generate().unwrap();
    let identity = ServerIdentity::ephemeral()
        .unwrap()
        .with_authorized(client.public());
    (Arc::new(identity), client)
}

/// Runs `f` on the Server side of a pair once its connection is accepted.
fn serve<T: Send + 'static>(
    server: thread::JoinHandle<io::Result<NoiseStream>>,
    f: impl FnOnce(&mut NoiseStream) -> io::Result<T> + Send + 'static,
) -> thread::JoinHandle<io::Result<T>> {
    thread::spawn(move || f(&mut server.join().unwrap()?))
}

#[test]
fn unknown_peer_without_an_invite_is_refused_before_any_payload() {
    let (identity, _) = identity();
    let stranger = StaticKey::generate().unwrap();
    let (mut client, server) = pair(&identity, &stranger, None);
    // The handshake runs on first use, so the Server side reads on its own thread.
    let server = serve(server, |server| server.read(&mut [0; 16]).map(drop));
    let error = client
        .write_all(b"hello")
        .and_then(|()| client.flush())
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    let error = server.join().unwrap().unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
}

#[test]
fn wrong_server_key_fails_the_client_handshake() {
    let (identity, client) = identity();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (socket, _) = listener.accept().unwrap();
        let mut server = NoiseStream::responder(socket, identity).unwrap();
        server.read(&mut [0; 16])
    });
    let impostor = StaticKey::generate().unwrap();
    let mut stream = NoiseStream::initiator(
        TcpStream::connect(address).unwrap(),
        &impostor.public(),
        &client,
        None,
    )
    .unwrap();
    let error = stream
        .write_all(b"hello")
        .and_then(|()| stream.flush())
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    assert!(server.join().unwrap().is_err());
}
