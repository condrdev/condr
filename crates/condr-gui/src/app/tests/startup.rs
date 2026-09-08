use super::*;

#[test]
fn server_connection_preserves_start_error_and_final_concurrent_probe() {
    let local = condr_server::ServerConfig::default().local_endpoint();
    let (_, failed) = connect_to_server_with(
        local.clone(),
        || {
            Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "invalid endpoint marker",
            ))
        },
        |_| Err::<(), String>("named pipe was not found".into()),
    );
    assert_eq!(
        failed.unwrap_err(),
        "Failed to start or discover the local Server: invalid endpoint marker"
    );

    let (_, concurrent) = connect_to_server_with(
        local,
        || Err(std::io::Error::other("concurrent launcher won")),
        |_| Ok("connected concurrently"),
    );
    assert_eq!(concurrent.unwrap(), "connected concurrently");

    let remote = tcp("127.0.0.1:4242");
    let (connected_endpoint, connected) = connect_to_server_with(
        remote.clone(),
        || panic!("remote endpoints must not start the local server"),
        |_| Ok(()),
    );
    assert_eq!(connected_endpoint, remote);
    connected.unwrap();
}

#[test]
fn condr_assets_include_custom_and_kit_icons() {
    let assets = CondrAssets::new();
    let listed = assets.list("icons/").unwrap();
    for path in [
        "icons/circle.svg",
        "icons/circle-filled.svg",
        "icons/circle-alert.svg",
        "icons/server-plus.svg",
        "icons/claude.svg",
        "icons/codex.svg",
        "icons/opencode.svg",
        "icons/pi.svg",
        "icons/omp.svg",
        "icons/copilot.svg",
        "icons/kimi.svg",
        "icons/kilo.svg",
        "icons/qoder.svg",
        "icons/qwen.svg",
        "icons/cursor.svg",
        "icons/grok.svg",
        "icons/antigravity.svg",
        "icons/info.svg",
        "icons/settings.svg",
    ] {
        let bytes = assets
            .load(path)
            .unwrap()
            .expect("Condr and GPUI Kit icons should be embedded");
        assert!(bytes.starts_with(b"<svg"), "invalid SVG asset: {path}");
        assert!(listed.iter().any(|listed| listed.as_ref() == path));
    }
    let listed = assets.list("icons/circle").unwrap();
    assert!(
        listed
            .iter()
            .any(|path| path.as_ref() == "icons/circle.svg")
    );
    assert!(
        listed
            .iter()
            .any(|path| path.as_ref() == "icons/circle-alert.svg")
    );
}

#[test]
fn single_instance_lock_admits_one_holder_at_a_time() {
    let directory = std::env::temp_dir().join(format!(
        "condr-instance-lock-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let path = directory.join("condr.lock");

    let held = lock_exclusively(&path).expect("the first GUI takes the lock");
    let refused = lock_exclusively(&path).expect_err("a second GUI must be refused");
    assert_eq!(refused.kind(), std::io::ErrorKind::WouldBlock);

    drop(held);
    lock_exclusively(&path).expect("the lock is free once the first GUI exits");
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn single_instance_lock_lives_in_the_platform_runtime_directory() {
    assert_eq!(
        single_instance_lock_path(),
        condr_core::runtime_directory().map(|root| root.join("condr.lock"))
    );
}
