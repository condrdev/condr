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
fn skill_prints_the_bundled_document_without_a_server() {
    let root = std::env::temp_dir().join(format!("condr-skill-{}", uuid::Uuid::new_v4()));
    let output = Command::new(env!("CARGO_BIN_EXE_condr"))
        .arg("--skill")
        .env("CONDR_SOCKET_PATH", root.join("absent.sock"))
        .env("CONDR_CONFIG_DIR", &root)
        .env("CONDR_SERVER_EXECUTABLE", root.join("absent-server"))
        .env_remove("CONDR_ENV")
        .env_remove("CONDR_PANE_ID")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty());
    assert_eq!(
        output.stdout,
        include_bytes!("../../../skills/condr/SKILL.md")
    );
    assert!(
        !root.exists(),
        "printing the skill initialized runtime files"
    );
}

#[cfg(unix)]
#[test]
fn omp_hook_status_uses_the_native_config_root_and_agent_override() {
    let home = std::path::PathBuf::from(std::env::var_os("HOME").unwrap());
    let name = format!(".condr-omp-path-test-{}", uuid::Uuid::new_v4());
    let mut command = Command::new(env!("CARGO_BIN_EXE_condr"));
    command.args(["agent", "hooks", "status", "omp"]);
    let status_path = |command: &mut Command| {
        let output = command.output().unwrap();
        assert!(output.status.success(), "{output:?}");
        let report: Value = serde_json::from_slice(&output.stdout).unwrap();
        std::path::PathBuf::from(report["path"].as_str().unwrap())
    };
    // Native OMP joins the config root name to home, including a leading '/'.
    // Status only reads; neither these directories nor the user's config are written.
    for config_name in [name.clone(), format!("/{name}")] {
        command
            .env("PI_CONFIG_DIR", config_name)
            .env_remove("PI_CODING_AGENT_DIR");
        assert_eq!(
            status_path(&mut command),
            home.join(&name).join("agent/extensions/condr-omp.ts")
        );
        command.env("PI_CODING_AGENT_DIR", "");
        assert_eq!(
            status_path(&mut command),
            home.join(&name).join("agent/extensions/condr-omp.ts")
        );
        let agent_dir = std::env::temp_dir().join(&name).join("agent override");
        command.env("PI_CODING_AGENT_DIR", &agent_dir);
        assert_eq!(
            status_path(&mut command),
            agent_dir.join("extensions/condr-omp.ts")
        );
    }
    assert!(!home.join(name).exists());
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
    let server =
        BoundServer::bind(ServerConfig::ephemeral(endpoint.as_local_path().unwrap())).unwrap();
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
    assert_eq!(
        ok(&endpoint_path, &["agent", "list"])["agents"],
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

    // Panes: split the Tab's Pane in the background, type into the new one, read it back.
    let root_pane = created["root_pane"]["pane_id"].as_u64().unwrap();
    let root_pane_arg = root_pane.to_string();
    let split = ok(
        &endpoint_path,
        &["pane", "split", &root_pane_arg, "--direction", "right"],
    );
    let new_pane = split["pane"]["pane_id"].as_u64().unwrap();
    assert_ne!(new_pane, root_pane);
    assert_eq!(split["pane"]["tab_id"], first_tab);
    assert_eq!(split["pane"]["focused"], false, "--focus was not given");
    assert_eq!(split["pane"]["agent_status"], "unknown");
    let new_pane_arg = new_pane.to_string();
    assert_eq!(
        ok(
            &endpoint_path,
            &["pane", "list", "--workspace", &workspace_arg]
        )["panes"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(
        ok(&endpoint_path, &["pane", "focus", &new_pane_arg])["pane"]["focused"],
        true
    );
    // Rearrangements report whether anything moved.
    let left = ok(
        &endpoint_path,
        &["pane", "focus", &new_pane_arg, "--direction", "left"],
    );
    assert_eq!(left["pane"]["pane_id"], root_pane);
    assert_eq!(left["changed"], true);
    assert_eq!(
        ok(
            &endpoint_path,
            &["pane", "focus", &root_pane_arg, "--direction", "up"]
        )["changed"],
        false,
        "nothing above the root Pane"
    );
    assert_eq!(
        ok(
            &endpoint_path,
            &["pane", "resize", &root_pane_arg, "--direction", "right"]
        )["changed"],
        true
    );
    let swapped = ok(
        &endpoint_path,
        &["pane", "swap", &root_pane_arg, "--direction", "right"],
    );
    assert_eq!(swapped["changed"], true);
    let zoomed = ok(&endpoint_path, &["pane", "zoom", &new_pane_arg, "--on"]);
    assert_eq!(zoomed["pane"]["zoomed"], true);
    assert_eq!(zoomed["changed"], true);
    assert_eq!(
        ok(&endpoint_path, &["pane", "zoom", &new_pane_arg, "--on"])["changed"],
        false
    );
    assert_eq!(
        ok(&endpoint_path, &["pane", "zoom", &new_pane_arg])["pane"]["zoomed"],
        false
    );
    // After the swap the new Pane is on the left, so its right neighbor is the root.
    let layout = ok(&endpoint_path, &["pane", "layout", &new_pane_arg]);
    assert_eq!(layout["tab_id"], first_tab);
    assert_eq!(layout["pane"]["neighbors"]["right"], root_pane);
    assert_eq!(layout["pane"]["neighbors"]["left"], Value::Null);
    assert_eq!(layout["pane"]["edges"]["left"], 0.0);
    assert_eq!(layout["pane"]["edges"]["top"], 0.0);
    assert_eq!(layout["pane"]["edges"]["bottom"], 1.0);
    assert!(layout["pane"]["edges"]["right"].as_f64().unwrap() < 1.0);
    assert_eq!(layout["panes"].as_array().unwrap().len(), 2);
    assert_eq!(layout["tree"]["split"], "horizontal");
    assert_eq!(layout["tree"]["first"]["pane"], new_pane);
    assert_eq!(layout["tree"]["second"]["pane"], root_pane);
    // The shell echoes what it is sent; the marker must show up in the read text.
    assert_eq!(
        ok(
            &endpoint_path,
            &["pane", "send-text", &new_pane_arg, "condr-cli-marker"]
        )["ok"],
        true
    );
    // A cold shell can take seconds to start and echo.
    let mut text = String::new();
    for _ in 0..300 {
        let output = condr(
            &endpoint_path,
            &["pane", "read", &new_pane_arg, "--lines", "5"],
        );
        assert!(output.status.success());
        text = String::from_utf8(output.stdout).unwrap();
        // A long prompt can wrap the marker across two rows; compare without row breaks.
        if text.replace(['\r', '\n'], "").contains("condr-cli-marker") {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
    assert!(
        text.replace(['\r', '\n'], "").contains("condr-cli-marker"),
        "read text: {text:?}"
    );
    assert_eq!(
        ok(
            &endpoint_path,
            &["pane", "send-keys", &new_pane_arg, "ctrl+u", "esc"]
        )["ok"],
        true
    );
    assert_eq!(
        err(
            &endpoint_path,
            &["pane", "send-keys", &new_pane_arg, "hyper+x"]
        )["code"],
        "invalid_key"
    );
    assert_eq!(
        err(&endpoint_path, &["pane", "read", "424242"])["code"],
        "pane_not_found"
    );
    assert_eq!(
        err(&endpoint_path, &["pane", "current"])["code"],
        "no_current_pane"
    );
    assert_eq!(
        ok(&endpoint_path, &["pane", "close", &new_pane_arg])["ok"],
        true
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
