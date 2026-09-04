//! `condr workspace …` / `condr tab …` against a real Server, the way a program in a
//! Pane runs them: `CONDR_SOCKET_PATH` names the Server, JSON comes back on stdout,
//! errors on stderr with exit 1.

use std::path::Path;
use std::process::{Command, Output};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use condr_server::{BoundServer, Endpoint, ServerConfig};
use serde_json::Value;

fn condr(endpoint: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_condr"))
        .args(args)
        .env("CONDR_SOCKET_PATH", endpoint)
        .env_remove("CONDR_PANE_ID")
        .output()
        .unwrap()
}

/// Success: exit 0 and a JSON object on stdout.
fn ok(endpoint: &Path, args: &[&str]) -> Value {
    let output = condr(endpoint, args);
    assert!(
        output.status.success(),
        "{args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

/// Failure: exit 1 and `{"error":{"code":…}}` on stderr, nothing on stdout.
fn err(endpoint: &Path, args: &[&str]) -> Value {
    let output = condr(endpoint, args);
    assert_eq!(output.status.code(), Some(1), "{args:?}");
    assert!(output.stdout.is_empty());
    serde_json::from_slice::<Value>(&output.stderr).unwrap()["error"].clone()
}

#[test]
fn workspace_and_tab_commands_drive_a_live_server() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let endpoint_path =
        std::env::temp_dir().join(format!("condr-cli-{}-{suffix}.sock", std::process::id()));
    let root = std::env::temp_dir().join(format!("condr-cli-root-{suffix}"));
    std::fs::create_dir_all(&root).unwrap();
    let endpoint = Endpoint::local(&endpoint_path);
    let server = BoundServer::bind(ServerConfig::ephemeral(endpoint.clone())).unwrap();
    let handle = server.handle();
    let server_thread = thread::spawn(move || server.run());
    for _ in 0..100 {
        if endpoint.connect().is_ok() {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }

    assert_eq!(
        ok(&endpoint_path, &["workspace", "list"])["workspaces"],
        Value::Array(vec![])
    );

    let root_text = root.to_str().unwrap();
    let created = ok(
        &endpoint_path,
        &["workspace", "create", "--cwd", root_text, "--label", "demo"],
    );
    let workspace_id = created["workspace"]["workspace_id"].as_u64().unwrap();
    assert_eq!(created["workspace"]["name"], "demo");
    assert_eq!(created["workspace"]["tab_count"], 1);
    // The first Workspace stays active: there was nothing to go back to.
    assert_eq!(created["workspace"]["focused"], true);
    let first_tab = created["tab"]["tab_id"].as_u64().unwrap();
    assert_eq!(created["tab"]["focused"], true);
    assert!(created["root_pane"]["pane_id"].is_u64());
    let workspace_arg = workspace_id.to_string();

    // A background Tab: created, named, but the active Tab does not change.
    let tab = ok(
        &endpoint_path,
        &[
            "tab",
            "create",
            "--workspace",
            &workspace_arg,
            "--label",
            "logs",
        ],
    );
    let tab_id = tab["tab"]["tab_id"].as_u64().unwrap();
    assert_eq!(tab["tab"]["name"], "logs");
    assert_eq!(tab["tab"]["focused"], false);
    assert_eq!(
        ok(&endpoint_path, &["tab", "get", &first_tab.to_string()])["tab"]["focused"],
        true
    );
    assert_eq!(
        ok(
            &endpoint_path,
            &["tab", "list", "--workspace", &workspace_arg]
        )["tabs"]
            .as_array()
            .unwrap()
            .len(),
        2
    );

    let tab_arg = tab_id.to_string();
    assert_eq!(
        ok(&endpoint_path, &["tab", "focus", &tab_arg])["tab"]["focused"],
        true
    );
    assert_eq!(
        ok(&endpoint_path, &["tab", "rename", &tab_arg, "build"])["tab"]["name"],
        "build"
    );
    assert_eq!(
        ok(
            &endpoint_path,
            &["workspace", "rename", &workspace_arg, "renamed"]
        )["workspace"]["name"],
        "renamed"
    );

    assert_eq!(
        err(&endpoint_path, &["workspace", "get", "424242"])["code"],
        "workspace_not_found"
    );
    assert_eq!(
        err(&endpoint_path, &["tab", "close", "424242"])["code"],
        "tab_not_found"
    );
    // Usage errors are clap's: exit 2, no JSON.
    assert_eq!(
        condr(&endpoint_path, &["tab", "frobnicate"]).status.code(),
        Some(2)
    );

    assert_eq!(ok(&endpoint_path, &["tab", "close", &tab_arg])["ok"], true);
    assert_eq!(
        ok(&endpoint_path, &["tab", "list"])["tabs"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        ok(&endpoint_path, &["workspace", "close", &workspace_arg])["ok"],
        true
    );
    assert_eq!(
        ok(&endpoint_path, &["workspace", "list"])["workspaces"],
        Value::Array(vec![])
    );

    handle.stop();
    server_thread.join().unwrap().unwrap();
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_missing_server_is_a_json_error() {
    let missing = std::env::temp_dir().join("condr-cli-no-server.sock");
    assert_eq!(
        err(&missing, &["workspace", "list"])["code"],
        "server_not_running"
    );
}
