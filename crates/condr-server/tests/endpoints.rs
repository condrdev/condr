use condr_server::SshEndpoint;

#[test]
fn addresses_round_trip_and_reject_options_commands_and_invalid_paths() {
    for address in [
        "ssh://build-box",
        "ssh://alice@host:2222",
        "ssh://user@[::1]:22",
        "ssh://host?bin=/opt/example/condr",
        "ssh://host?bin=/opt/a%20b%27%24%28x%29/condr",
        "ssh://host?bin=/opt/%E4%B8%AD%E6%96%87/condr",
    ] {
        let endpoint = SshEndpoint::parse(address).unwrap();
        assert_eq!(endpoint.to_string(), address);
        assert_eq!(SshEndpoint::parse(&endpoint.to_string()).unwrap(), endpoint);
        assert!(
            condr_server::Endpoint::parse(address, None)
                .unwrap()
                .as_local_path()
                .is_none()
        );
    }
    assert_eq!(SshEndpoint::parse("ssh://host").unwrap().binary(), "condr");
    assert_eq!(
        SshEndpoint::parse("ssh://host?bin=/opt/a+b/condr")
            .unwrap()
            .binary(),
        "/opt/a+b/condr"
    );
    for address in [
        "host",
        "tcp://host",
        "ssh://",
        "ssh://-oProxyCommand=x",
        "ssh://user@-host",
        "ssh://-user@host",
        "ssh://user@host;touch",
        "ssh://user@host/path",
        "ssh://a@b@host",
        "ssh://host:0",
        "ssh://host:65536",
        "ssh://[broken]",
        "ssh://::1",
        "ssh://host?bin=",
        "ssh://host?bin=relative",
        "ssh://host?bin=/a&bin=/b",
        "ssh://host?command=id",
        "ssh://host?bin=/a#fragment",
        "ssh://host?bin=/a%",
        "ssh://host?bin=/a%FF",
        "ssh://host?bin=/a%00",
        "ssh://host?bin=/a%0A",
        "ssh://host\ncommand",
    ] {
        assert!(SshEndpoint::parse(address).is_err(), "{address}");
    }
    let key = condr_server::StaticKey::from_private([7; 32]);
    let tcp = format!("tcp://{}@localhost:4242", key.public());
    assert!(condr_server::Endpoint::parse(&tcp, None).is_err());
    assert!(matches!(
        condr_server::Endpoint::parse(&tcp, Some(&key)).unwrap(),
        condr_server::Endpoint::Tcp(_)
    ));
    assert!(condr_server::Endpoint::parse("https://host", Some(&key)).is_err());
}
