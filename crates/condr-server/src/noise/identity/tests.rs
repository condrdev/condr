use super::*;
use std::net::TcpListener;
use std::thread;

fn pair(
    identity: &Arc<ServerIdentity>,
    client: &DeviceKey,
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

#[test]
fn revoke_requires_a_unique_prefix_without_changing_ambiguous_or_missing_keys() {
    let directory = std::env::temp_dir().join(format!(
        "condr-noise-revoke-{}-{}",
        std::process::id(),
        now()
    ));
    let first = PublicKey::parse(&format!("aa01{}", "00".repeat(30))).unwrap();
    let second = PublicKey::parse(&format!("aa02{}", "00".repeat(30))).unwrap();
    let clients = [first, second].map(|key| AuthorizedClient {
        key,
        paired_at: 1,
        last_seen: 2,
        name: "device".into(),
    });
    write_authorized(&directory, &clients).unwrap();
    let before = fs::read(directory.join(AUTHORIZED_FILE)).unwrap();
    assert_eq!(
        revoke(&directory, "AA").unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    assert_eq!(fs::read(directory.join(AUTHORIZED_FILE)).unwrap(), before);
    assert_eq!(revoke(&directory, "bb").unwrap(), None);
    assert_eq!(revoke(&directory, "").unwrap(), None);
    assert_eq!(fs::read(directory.join(AUTHORIZED_FILE)).unwrap(), before);
    assert_eq!(revoke(&directory, "AA01").unwrap(), Some(first));
    assert_eq!(
        read_authorized(&directory).unwrap(),
        vec![clients[1].clone()]
    );
    fs::remove_dir_all(directory).unwrap();
}

/// Runs `f` on the Server side of a pair once its connection is accepted.
fn serve<T: Send + 'static>(
    server: thread::JoinHandle<io::Result<NoiseStream>>,
    f: impl FnOnce(&mut NoiseStream) -> io::Result<T> + Send + 'static,
) -> thread::JoinHandle<io::Result<T>> {
    thread::spawn(move || f(&mut server.join().unwrap()?))
}

#[test]
fn an_invite_pairs_a_new_device_once_and_a_bad_invite_does_not() {
    let directory = std::env::temp_dir().join(format!(
        "condr-noise-pairing-{}-{}",
        std::process::id(),
        now()
    ));
    let identity = Arc::new(ServerIdentity::load_or_create(&directory).unwrap());
    let device = DeviceKey::generate().unwrap();

    let invite = create_invite(&directory).unwrap();
    let wrong = Secret::generate().unwrap();
    let (mut client, server) = pair(&identity, &device, Some(&wrong));
    let server = serve(server, |server| {
        // The Client cannot decrypt the second message and hangs up; the Server saw no
        // transport record, so the invite is not proven and nothing is recorded.
        let read = server.read(&mut [0; 16]).unwrap_or(0);
        let paired = server.complete_pairing("laptop");
        Ok((read, paired.is_err()))
    });
    let error = client
        .write_all(b"hello")
        .and_then(|()| client.flush())
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    drop(client);
    assert_eq!(server.join().unwrap().unwrap(), (0, true));
    assert!(read_authorized(&directory).unwrap().is_empty());

    let (mut client, server) = pair(&identity, &device, Some(&invite.secret));
    let server = serve(server, |server| {
        let mut received = [0; 5];
        server.read_exact(&mut received)?;
        assert_eq!(&received, b"hello");
        server.complete_pairing("laptop")
    });
    client.write_all(b"hello").unwrap();
    client.flush().unwrap();
    server.join().unwrap().unwrap();
    let clients = read_authorized(&directory).unwrap();
    assert_eq!(clients.len(), 1);
    assert_eq!(clients[0].key, device.public());
    assert_eq!(clients[0].name, "laptop");
    assert!(
        read_invite(&directory).unwrap().is_none(),
        "invite is one-time"
    );

    // A second device that finished its handshake with the same invite loses the
    // race: the invite is gone by the time it proves itself.
    let invite = create_invite(&directory).unwrap();
    let first = DeviceKey::generate().unwrap();
    let second = DeviceKey::generate().unwrap();
    let (mut first_client, first_server) = pair(&identity, &first, Some(&invite.secret));
    let (mut second_client, second_server) = pair(&identity, &second, Some(&invite.secret));
    let first_server = serve(first_server, |server| {
        server.read_exact(&mut [0; 2])?;
        server.complete_pairing("first")
    });
    let second_server = serve(second_server, |server| {
        server.read_exact(&mut [0; 2])?;
        server.complete_pairing("second")
    });
    // Both handshakes complete while the invite is still pending; only the first
    // device to prove itself gets recorded.
    first_client.handshake().unwrap();
    second_client.handshake().unwrap();
    first_client.write_all(b"hi").unwrap();
    first_client.flush().unwrap();
    first_server.join().unwrap().unwrap();
    second_client.write_all(b"hi").unwrap();
    second_client.flush().unwrap();
    assert_eq!(
        second_server.join().unwrap().unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
    let paired = read_authorized(&directory).unwrap();
    assert!(paired.iter().any(|client| client.key == first.public()));
    assert!(!paired.iter().any(|client| client.key == second.public()));

    // A handshake made with an older invite cannot pair after a newer invite replaced
    // it, and does not consume the newer one.
    let stale = create_invite(&directory).unwrap();
    let late = DeviceKey::generate().unwrap();
    let (mut late_client, late_server) = pair(&identity, &late, Some(&stale.secret));
    let late_server = serve(late_server, |server| {
        server.read_exact(&mut [0; 2])?;
        server.complete_pairing("late")
    });
    late_client.handshake().unwrap();
    let fresh = create_invite(&directory).unwrap();
    late_client.write_all(b"hi").unwrap();
    late_client.flush().unwrap();
    assert_eq!(
        late_server.join().unwrap().unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
    assert_eq!(read_invite(&directory).unwrap(), Some(fresh));
    let _ = fs::remove_file(directory.join(INVITE_FILE));

    // Paired: connects with the zero PSK, no invite needed.
    let (mut client, server) = pair(&identity, &device, None);
    let server = serve(server, |server| {
        let mut received = [0; 5];
        server.read_exact(&mut received)?;
        Ok(received)
    });
    client.write_all(b"again").unwrap();
    client.flush().unwrap();
    assert_eq!(&server.join().unwrap().unwrap(), b"again");

    assert_eq!(
        revoke(&directory, &device.public().to_hex()[..8]).unwrap(),
        Some(device.public())
    );
    let (mut client, server) = pair(&identity, &device, None);
    let server = serve(server, |server| server.read(&mut [0; 16]).map(drop));
    assert!(
        client
            .write_all(b"x")
            .and_then(|()| client.flush())
            .is_err()
    );
    assert!(server.join().unwrap().is_err());
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn authorized_clients_keep_last_seen_and_reject_legacy_lines() {
    let directory =
        std::env::temp_dir().join(format!("condr-noise-seen-{}-{}", std::process::id(), now()));
    fs::create_dir_all(&directory).unwrap();
    // Creating the key discards any authorized list left from before it existed.
    fs::write(directory.join(AUTHORIZED_FILE), "stale\n").unwrap();
    let identity = ServerIdentity::load_or_create(&directory).unwrap();
    assert!(read_authorized(&directory).unwrap().is_empty());
    let old = DeviceKey::generate().unwrap().public();
    let new = DeviceKey::generate().unwrap().public();
    fs::write(
        directory.join(AUTHORIZED_FILE),
        format!("{old} 100 old laptop\n"),
    )
    .unwrap();
    assert!(read_authorized(&directory).is_err());
    fs::write(
        directory.join(AUTHORIZED_FILE),
        format!("{old} 100 100 old laptop\n{new} 200 300 new laptop\n"),
    )
    .unwrap();

    let clients = read_authorized(&directory).unwrap();
    assert_eq!((clients[0].paired_at, clients[0].last_seen), (100, 100));
    assert_eq!(clients[0].name, "old laptop");
    assert_eq!((clients[1].paired_at, clients[1].last_seen), (200, 300));
    assert_eq!(clients[1].name, "new laptop");

    identity.record_seen(&old).unwrap();
    let clients = read_authorized(&directory).unwrap();
    assert!(clients[0].last_seen >= now() - 5);
    assert_eq!(clients[0].name, "old laptop", "rewriting keeps the name");
    assert_eq!(clients[1].last_seen, 300, "other devices are untouched");
    let _ = fs::remove_dir_all(directory);
}
