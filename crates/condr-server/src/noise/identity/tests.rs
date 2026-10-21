use super::*;
use std::io::Read;
use std::net::{TcpListener, TcpStream};
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
    // Fingerprints "qqqq…" and "q6ur…": one shared first character, distinct second.
    let first = PublicKey::from_bytes([0xaa; 32]);
    let second = PublicKey::from_bytes([0xab; 32]);
    let clients = [first, second].map(|key| AuthorizedClient {
        key,
        name: "device".into(),
        paired_at: 1,
        last_seen: 2,
        push: true,
    });
    write_authorized(&directory, &clients).unwrap();
    let before = fs::read(directory.join(AUTHORIZED_FILE)).unwrap();
    assert_eq!(
        revoke(&directory, &first.to_string()[..1])
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidInput
    );
    assert_eq!(fs::read(directory.join(AUTHORIZED_FILE)).unwrap(), before);
    assert_eq!(revoke(&directory, "bb").unwrap(), None);
    assert_eq!(revoke(&directory, "").unwrap(), None);
    assert_eq!(fs::read(directory.join(AUTHORIZED_FILE)).unwrap(), before);
    assert_eq!(
        revoke(&directory, &first.to_string()[..2]).unwrap(),
        Some(first)
    );
    assert_eq!(
        read_authorized(&directory).unwrap(),
        vec![clients[1].clone()]
    );
    fs::remove_dir_all(directory).unwrap();
}

/// compat(0.1.13): `clients` as lines of `authorized-clients`, as releases up to 0.1.13
/// wrote them.
fn write_legacy_authorized(directory: &Path, clients: &[AuthorizedClient]) {
    fs::create_dir_all(directory).unwrap();
    let lines = clients
        .iter()
        .map(|client| {
            let key = client.key.to_hex();
            format!(
                "{key} {} {} {}\n",
                client.paired_at, client.last_seen, client.name
            )
        })
        .collect::<String>();
    fs::write(directory.join(LEGACY_AUTHORIZED_FILE), lines).unwrap();
}

// compat(0.1.6): goes with `adopt_legacy_store`; the conversion after it is
// compat(0.1.13) and goes with `convert_legacy_list`.
#[test]
fn the_identity_files_move_out_of_the_legacy_directory_once() {
    let root = std::env::temp_dir().join(format!(
        "condr-noise-adopt-{}-{}",
        std::process::id(),
        now()
    ));
    let (legacy, directory) = (root.join("config"), root.join("data"));
    let key = load_device_key(&legacy).unwrap();
    let client = AuthorizedClient {
        key: DeviceKey::generate().unwrap().public(),
        name: "laptop".into(),
        paired_at: 1,
        last_seen: 2,
        push: false,
    };
    write_legacy_authorized(&legacy, std::slice::from_ref(&client));

    // The move keeps the old name; the conversion straight after gives the new file.
    adopt_legacy_store(&legacy, &directory).unwrap();
    assert_eq!(load_device_key(&directory).unwrap(), key);
    assert!(directory.join(LEGACY_AUTHORIZED_FILE).exists());
    assert!(!legacy.join(DEVICE_KEY_FILE).exists());
    assert!(!legacy.join(LEGACY_AUTHORIZED_FILE).exists());
    convert_legacy_list(&directory).unwrap();
    assert_eq!(read_authorized(&directory).unwrap(), vec![client]);
    assert!(!directory.join(LEGACY_AUTHORIZED_FILE).exists());

    // A key already in place is kept; a later file beside `config.toml` is left alone.
    let later = load_device_key(&legacy).unwrap();
    adopt_legacy_store(&legacy, &directory).unwrap();
    assert_eq!(load_device_key(&directory).unwrap(), key);
    assert_eq!(load_device_key(&legacy).unwrap(), later);
    fs::remove_dir_all(root).unwrap();
}

/// `icacls` on `path`: one `account:(flags)` entry per line, `(I)` marking inherited ones.
#[cfg(windows)]
fn icacls(path: &Path) -> String {
    let output = std::process::Command::new("icacls")
        .arg(path)
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[cfg(windows)]
#[test]
fn secret_files_admit_only_their_owner_on_windows() {
    let directory =
        std::env::temp_dir().join(format!("condr-noise-dacl-{}-{}", std::process::id(), now()));
    write_secret_file(&directory.join("fresh"), "secret").unwrap();
    // compat(0.1.6): the "old" file goes with `windows::restrict_to_owner`. A file from an
    // older release inherits its directory's entries until it is read.
    fs::write(directory.join("old"), "secret\n").unwrap();
    assert!(icacls(&directory.join("old")).contains("(I)"));
    assert_eq!(
        read_secret_file(&directory.join("old")).unwrap().as_deref(),
        Some("secret\n")
    );
    for name in ["fresh", "old"] {
        let acl = icacls(&directory.join(name));
        let entries: Vec<&str> = acl.lines().filter(|line| line.contains(":(")).collect();
        assert_eq!(entries.len(), 2, "{acl}");
        assert!(entries.iter().all(|entry| !entry.contains("(I)")), "{acl}");
        assert!(entries.iter().all(|entry| entry.ends_with(":(F)")), "{acl}");
    }
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
        revoke(&directory, &device.public().to_string()[..8]).unwrap(),
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
fn authorized_clients_keep_last_seen_and_a_missing_push_reads_false() {
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
        format!(
            r#"[{{"key": "{}", "name": "old laptop", "paired_at": 100, "last_seen": 100}}]"#,
            old.to_hex()
        ),
    )
    .unwrap();
    assert!(
        read_authorized(&directory).is_err(),
        "keys are Device ids, not hex"
    );
    fs::write(
        directory.join(AUTHORIZED_FILE),
        format!(
            r#"[
  {{"key": "{old}", "name": "old laptop", "paired_at": 100, "last_seen": 100}},
  {{"key": "{new}", "name": "new laptop", "paired_at": 200, "last_seen": 300, "push": true}}
]"#
        ),
    )
    .unwrap();

    let clients = read_authorized(&directory).unwrap();
    assert_eq!((clients[0].paired_at, clients[0].last_seen), (100, 100));
    assert_eq!(clients[0].name, "old laptop");
    assert!(!clients[0].push, "a missing push reads as false");
    assert_eq!((clients[1].paired_at, clients[1].last_seen), (200, 300));
    assert_eq!(clients[1].name, "new laptop");
    assert!(clients[1].push);

    identity.record_seen(&old, "", true).unwrap();
    let clients = read_authorized(&directory).unwrap();
    assert!(clients[0].last_seen >= now() - 5);
    assert_eq!(clients[0].name, "old laptop", "no name keeps the old one");
    assert!(clients[0].push, "the handshake's push is recorded");
    assert_eq!(clients[1].last_seen, 300, "other devices are untouched");

    identity
        .record_seen(&old, " renamed\tlaptop ", false)
        .unwrap();
    let clients = read_authorized(&directory).unwrap();
    assert_eq!(
        clients[0].name, "renamed laptop",
        "a renamed device is listed by it"
    );
    assert!(!clients[0].push, "a later handshake without push clears it");
    assert_eq!(clients[1].name, "new laptop");
    assert!(clients[1].push, "another device's handshake keeps its push");
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn authorized_clients_round_trip_as_pretty_json_with_device_ids() {
    let directory =
        std::env::temp_dir().join(format!("condr-noise-json-{}-{}", std::process::id(), now()));
    let clients = [(0x11, "phone", true), (0x22, "desk top", false)].map(|(byte, name, push)| {
        AuthorizedClient {
            key: PublicKey::from_bytes([byte; 32]),
            name: name.into(),
            paired_at: 10,
            last_seen: 20,
            push,
        }
    });
    write_authorized(&directory, &clients[..1]).unwrap();
    assert_eq!(
        fs::read_to_string(directory.join(AUTHORIZED_FILE)).unwrap(),
        format!(
            r#"[
  {{
    "key": "{}",
    "name": "phone",
    "paired_at": 10,
    "last_seen": 20,
    "push": true
  }}
]
"#,
            clients[0].key
        )
    );
    write_authorized(&directory, &clients).unwrap();
    assert_eq!(read_authorized(&directory).unwrap(), clients);
    write_authorized(&directory, &[]).unwrap();
    assert!(read_authorized(&directory).unwrap().is_empty());
    fs::remove_dir_all(directory).unwrap();
}

// compat(0.1.13): goes with `convert_legacy_list`.
#[test]
fn the_line_file_converts_to_json_once_and_is_deleted() {
    let directory = std::env::temp_dir().join(format!(
        "condr-noise-convert-{}-{}",
        std::process::id(),
        now()
    ));
    let clients = [0x33, 0x44].map(|byte| AuthorizedClient {
        key: PublicKey::from_bytes([byte; 32]),
        name: format!("device {byte}"),
        paired_at: 5,
        last_seen: 6,
        push: false,
    });
    write_legacy_authorized(&directory, &clients);
    convert_legacy_list(&directory).unwrap();
    assert_eq!(read_authorized(&directory).unwrap(), clients);
    assert!(!directory.join(LEGACY_AUTHORIZED_FILE).exists());
    assert_eq!(
        revoke(&directory, &clients[0].key.to_string()).unwrap(),
        Some(clients[0].key)
    );
    assert_eq!(read_authorized(&directory).unwrap(), [clients[1].clone()]);

    // Only a missing JSON file triggers it: an old file beside the new one is left alone.
    write_legacy_authorized(&directory, &clients);
    convert_legacy_list(&directory).unwrap();
    assert_eq!(read_authorized(&directory).unwrap(), [clients[1].clone()]);
    assert!(directory.join(LEGACY_AUTHORIZED_FILE).exists());

    // A file that does not parse stays where it is, and nothing is written.
    fs::remove_file(directory.join(AUTHORIZED_FILE)).unwrap();
    fs::write(directory.join(LEGACY_AUTHORIZED_FILE), "stale\n").unwrap();
    assert!(convert_legacy_list(&directory).is_err());
    assert!(directory.join(LEGACY_AUTHORIZED_FILE).exists());
    assert!(!directory.join(AUTHORIZED_FILE).exists());
    fs::remove_dir_all(directory).unwrap();
}

/// A TCP Invite's host candidates (ADR 0040): the default route's address first, the
/// `.local` name next, then the other addresses, of the families the listener takes.
#[test]
fn invite_hosts_put_the_likeliest_first_and_follow_the_listener() {
    use std::net::IpAddr;
    let ip = |text: &str| text.parse::<IpAddr>().unwrap();
    let local = [ip("100.64.1.5"), ip("192.168.1.20"), ip("2001:db8::7")];

    assert_eq!(
        order_invite_hosts(
            ip("10.0.0.3"),
            Some(ip("192.168.1.20")),
            "desk.local",
            &local
        ),
        ["10.0.0.3"],
        "a listener bound to one address is reached there"
    );
    assert_eq!(
        order_invite_hosts(
            ip("0.0.0.0"),
            Some(ip("192.168.1.20")),
            "desk.local",
            &local
        ),
        ["192.168.1.20", "desk.local", "100.64.1.5"],
        "an IPv4 listener lists no IPv6 address, and none twice"
    );
    assert_eq!(
        order_invite_hosts(ip("::"), None, "desk", &local),
        ["100.64.1.5", "192.168.1.20", "2001:db8::7"],
        "a name no network resolves is not offered"
    );
    assert_eq!(
        order_invite_hosts(
            ip("0.0.0.0"),
            Some(ip("198.18.0.1")),
            "desk.local",
            &[ip("198.18.0.1"), ip("192.168.1.20")]
        ),
        ["desk.local", "192.168.1.20"],
        "a proxy's TUN address is no way in, even on the default route"
    );
}
