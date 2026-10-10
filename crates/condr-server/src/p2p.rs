//! The Server's side of Peer-to-peer (ADR 0025, ADR 0026): the endpoint itself is
//! `condr-client`'s; what is left here is splicing a local Client's `Tunnel` onto it.

use crate::endpoint::EndpointStream;
use std::io::{self, Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

pub use condr_client::p2p::*;

/// Carries bytes between a local Client and the remote Device until either side closes.
/// The local socket has no shutdown that reaches a clone on another thread, so the
/// upstream copy polls with a timeout and leaves when the downstream copy has ended.
pub fn splice(local: EndpointStream, remote: P2pStream) -> io::Result<()> {
    let done = Arc::new(AtomicBool::new(false));
    let mut local_reader = local.try_clone()?;
    let mut remote_writer = remote.try_clone()?;
    let upstream_done = Arc::clone(&done);
    let upstream = thread::spawn(move || {
        let _ = local_reader.set_read_timeout(Some(Duration::from_millis(500)));
        let mut buffer = [0; 16 * 1024];
        loop {
            match local_reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(length) => {
                    if remote_writer.write_all(&buffer[..length]).is_err() {
                        break;
                    }
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    if upstream_done.load(Ordering::Acquire) {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        let _ = remote_writer.shutdown();
    });
    let mut remote_reader = remote;
    let mut local_writer = local;
    let result = io::copy(&mut remote_reader, &mut local_writer).map(drop);
    done.store(true, Ordering::Release);
    let _ = remote_reader.shutdown();
    drop(local_writer);
    let _ = upstream.join();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::noise::{Secret, ServerIdentity, create_invite, revoke};
    use condr_core::ProxySetting;
    use condr_core::protocol::{
        ClientHandshake, Hello, ServerMessage, read_message, write_message,
    };
    use std::sync::mpsc::Receiver;
    use tokio::runtime::Handle;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "condr-p2p-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn make_node(
        handle: &Handle,
        identity: Arc<ServerIdentity>,
        enabled: bool,
        lookup: &MemoryLookup,
    ) -> (Arc<P2pNode>, Receiver<EndpointStream>) {
        let (tx, rx) = std::sync::mpsc::channel();
        let accept: Accept = Box::new(move |stream| {
            tx.send(EndpointStream::P2p(stream))
                .map_err(|_| io::Error::other("stopped accepting"))
        });
        let node = P2pNode::new(
            handle.clone(),
            identity,
            Network::Loopback(lookup.clone()),
            ProxySetting::default(),
            enabled.then_some(accept),
        );
        (node, rx)
    }

    fn frame(message: &ClientHandshake) -> Vec<u8> {
        let mut bytes = Vec::new();
        write_message(&mut bytes, message).unwrap();
        bytes
    }

    /// The whole Peer-to-peer contract over loopback (ADR 0026): an unknown Device dials,
    /// redeems its invite, exchanges frames both ways, is recorded, and a revoke drops it.
    #[test]
    fn an_invite_pairs_a_device_and_a_revoke_drops_it() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let handle = runtime.handle().clone();
        let lookup = MemoryLookup::new();

        let server_dir = temp_dir("server");
        let server_identity = Arc::new(ServerIdentity::load_or_create(&server_dir).unwrap());
        let invite = create_invite(&server_dir).unwrap().secret;
        let client_dir = temp_dir("client");
        let client_identity = Arc::new(ServerIdentity::load_or_create(&client_dir).unwrap());
        let client_public = client_identity.public_key();
        let server_id = server_identity.public_key();

        let (server_node, accepted) = make_node(&handle, server_identity.clone(), true, &lookup);
        server_node.start().unwrap();
        let (client_node, _client_rx) = make_node(&handle, client_identity, false, &lookup);

        let client = std::thread::spawn(move || {
            let mut stream = client_node.dial(server_id, Some(&invite)).unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(10)));
            stream
                .write_all(&frame(&ClientHandshake::Hello(Hello::new("laptop"))))
                .unwrap();
            match read_message::<_, ServerMessage>(&mut stream).unwrap() {
                ServerMessage::Error { message } => message,
                other => panic!("expected the server's reply, got {other:?}"),
            }
        });

        let mut peer = match accepted
            .recv_timeout(Duration::from_secs(10))
            .expect("server accepts the connection")
        {
            EndpointStream::P2p(peer) => peer,
            other => panic!("expected a p2p stream, got {}", other.transport()),
        };
        peer.set_read_timeout(Some(Duration::from_secs(10)));
        assert!(peer.needs_credential().unwrap(), "the device is unknown");
        match read_message::<_, ClientHandshake>(&mut peer).unwrap() {
            ClientHandshake::Credential { invite } => {
                peer.present_invite(&Secret::from_bytes(invite)).unwrap();
            }
            other => panic!("expected a credential, got {other:?}"),
        }
        match read_message::<_, ClientHandshake>(&mut peer).unwrap() {
            ClientHandshake::Hello(hello) => assert_eq!(hello.client_name, "laptop"),
            other => panic!("expected Hello, got {other:?}"),
        }
        peer.complete_pairing("laptop").unwrap();
        assert!(peer.peer_authorized().unwrap(), "recorded once paired");
        write_message(
            &mut peer,
            &ServerMessage::Error {
                message: "welcome".into(),
            },
        )
        .unwrap();

        assert_eq!(client.join().unwrap(), "welcome", "frames flow both ways");
        assert!(
            server_identity.is_authorized(&client_public).unwrap(),
            "the invite recorded the device"
        );

        // A revoke closes the door: the still-open connection is no longer authorized.
        assert_eq!(
            revoke(&server_dir, &client_public.to_string()[..8]).unwrap(),
            Some(client_public)
        );
        assert!(!peer.peer_authorized().unwrap(), "revoked device refused");

        let _ = std::fs::remove_dir_all(&server_dir);
        let _ = std::fs::remove_dir_all(&client_dir);
    }
}
