use condr_core::agent_hooks::*;
use condr_core::{AgentEventKind, AgentKind};
use serde_json::Value;
use std::path::{Path, PathBuf};

fn target() -> (HookTarget, PathBuf) {
    let root = std::env::temp_dir().join(format!(
        "condr-hooks-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let target = HookTarget::in_home(&root, "\"/opt/condr bin/condr\"".to_owned());
    (target, root)
}

fn read(path: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn new_hook_files_install_refresh_and_remove_without_losing_user_entries() {
    let (target, root) = target();
    for agent in [AgentKind::Grok, AgentKind::Cursor, AgentKind::Copilot] {
        let path = target.path(agent);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let user =
            serde_json::json!({"hooks":{"custom":[{"command":"user-hook"}]},"userSetting":true});
        std::fs::write(&path, user.to_string()).unwrap();
        install(&target, agent).unwrap();
        assert_eq!(state(&target, agent).unwrap(), HooksState::Installed);
        let installed = read(&path);
        install(&target, agent).unwrap();
        assert_eq!(read(&path), installed);
        assert_eq!(installed["hooks"]["custom"], user["hooks"]["custom"]);
        if agent == AgentKind::Copilot {
            assert_eq!(
                installed["hooks"]["agentStop"][0]["exec"],
                "/opt/condr bin/condr"
            );
            assert_eq!(
                installed["hooks"]["agentStop"][0]["args"],
                serde_json::json!(["agent-hook", "copilot", "stop"])
            );
            assert!(installed["hooks"].get("permissionRequest").is_none());
        }
        if agent == AgentKind::Cursor {
            assert_eq!(
                installed["hooks"]["stop"][0]["command"],
                "\"/opt/condr bin/condr\" agent-hook cursor stop"
            );
            assert!(installed["hooks"].get("beforeShellExecution").is_none());
        }
        let moved = HookTarget {
            command: "condr".into(),
            ..target.clone()
        };
        assert_eq!(state(&moved, agent).unwrap(), HooksState::Outdated);
        install(&moved, agent).unwrap();
        assert_eq!(state(&moved, agent).unwrap(), HooksState::Installed);
        uninstall(&moved, agent).unwrap();
        assert_eq!(state(&moved, agent).unwrap(), HooksState::Missing);
        assert_eq!(read(&path)["hooks"], user["hooks"]);
        assert_eq!(read(&path)["userSetting"], true);
    }
    for agent in [AgentKind::Pi, AgentKind::Omp, AgentKind::Antigravity] {
        let path = target.path(agent);
        install(&target, agent).unwrap();
        assert_eq!(state(&target, agent).unwrap(), HooksState::Installed);
        let original = std::fs::read(&path).unwrap();
        install(&target, agent).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), original);
        uninstall(&target, agent).unwrap();
        assert!(!path.exists());
        uninstall(&target, agent).unwrap();
        std::fs::write(&path, "{\"user\":true}").unwrap();
        assert!(install(&target, agent).is_err());
        assert!(uninstall(&target, agent).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{\"user\":true}");
    }
    let kimi = run(&target, AgentKind::Kimi, HooksAction::Status).unwrap();
    assert_eq!(kimi.state, HooksState::Unsupported);
    assert!(kimi.note.unwrap().contains("subagent"));
    assert_eq!(
        install(&target, AgentKind::Kimi).unwrap_err().kind(),
        std::io::ErrorKind::Unsupported
    );
    assert!(!target.path(AgentKind::Kimi).exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn run_reports_the_state_after_each_action_and_codex_carries_its_note() {
    let (target, root) = target();
    let status = |agent| run(&target, agent, HooksAction::Status).unwrap();
    assert_eq!(status(AgentKind::Claude).state, HooksState::Missing);
    assert_eq!(status(AgentKind::Claude).note, None);
    assert!(status(AgentKind::Codex).note.is_some());
    let installed = run(&target, AgentKind::Claude, HooksAction::Install).unwrap();
    assert_eq!(installed.state, HooksState::Installed);
    assert_eq!(installed.path, target.path(AgentKind::Claude));
    assert_eq!(installed.warning, None);
    assert_eq!(
        run(&target, AgentKind::Claude, HooksAction::Uninstall)
            .unwrap()
            .state,
        HooksState::Missing
    );
    // The wire shape the CLI prints and the GUI reads.
    let json = serde_json::to_value(status(AgentKind::Claude)).unwrap();
    assert_eq!(json["agent"], "claude");
    assert_eq!(json["state"], "missing");
    assert!(json["note"].is_null());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_broken_or_oversized_settings_file_is_never_overwritten() {
    let (target, root) = target();
    let path = target.path(AgentKind::Claude);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "{ not json").unwrap();
    assert!(install(&target, AgentKind::Claude).is_err());
    assert!(state(&target, AgentKind::Claude).is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ not json");

    std::fs::write(&path, "[]").unwrap();
    assert!(install(&target, AgentKind::Claude).is_err());
    std::fs::write(&path, r#"{"hooks": "nope"}"#).unwrap();
    assert!(install(&target, AgentKind::Claude).is_err());

    // Whitespace-only counts as absent; an install creates the directory too.
    let fresh = root.join("fresh/.claude/settings.json");
    let target = HookTarget {
        claude_dir: fresh.parent().unwrap().to_path_buf(),
        ..target
    };
    install(&target, AgentKind::Claude).unwrap();
    assert_eq!(
        state(&target, AgentKind::Claude).unwrap(),
        HooksState::Installed
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn the_opencode_plugin_is_an_owned_file_that_never_replaces_a_foreign_one() {
    let (target, root) = target();
    let path = target.path(AgentKind::OpenCode);
    assert!(path.ends_with(".config/opencode/condr-tui.js"));
    assert_eq!(
        state(&target, AgentKind::OpenCode).unwrap(),
        HooksState::Missing
    );

    std::fs::create_dir_all(&target.opencode_dir).unwrap();
    let config = target.opencode_dir.join("tui.json");
    let jsonc = target.opencode_dir.join("tui.jsonc");
    let original_jsonc = "{ // user's settings\n  \"theme\": \"custom\"\n}\n";
    std::fs::write(&config, r#"{"plugin":["another-plugin"],"scroll_speed":4}"#).unwrap();
    std::fs::write(&jsonc, original_jsonc).unwrap();

    install(&target, AgentKind::OpenCode).unwrap();
    assert_eq!(
        state(&target, AgentKind::OpenCode).unwrap(),
        HooksState::Installed
    );
    let plugin = std::fs::read_to_string(&path).unwrap();
    assert!(plugin.contains(r#"const exe = "/opt/condr bin/condr""#));
    assert!(plugin.contains("[\"agent-hook\", \"opencode\", event]"));
    assert!(plugin.contains(r#"process.env.CONDR_ENV !== "1""#));
    assert!(plugin.contains("api.route.current"));
    assert_eq!(
        read(&config)["plugin"],
        serde_json::json!(["another-plugin", "./condr-tui.js"])
    );
    assert_eq!(read(&config)["scroll_speed"], 4);
    assert_eq!(std::fs::read_to_string(&jsonc).unwrap(), original_jsonc);
    for event in [
        "session-start",
        "prompt-submit",
        "stop",
        "permission-request",
        "question-asked",
    ] {
        assert!(
            plugin.contains(&format!("\"{event}\"")),
            "{event} is reported"
        );
        assert!(
            AgentEventKind::parse(event).is_some(),
            "{event} is a wire event"
        );
    }

    // A plugin from another condr is ours, but outdated; installing replaces it.
    std::fs::write(
        &path,
        "// condr agent-hook opencode — generated by condr\nold",
    )
    .unwrap();
    assert_eq!(
        state(&target, AgentKind::OpenCode).unwrap(),
        HooksState::Outdated
    );
    install(&target, AgentKind::OpenCode).unwrap();
    assert_eq!(
        state(&target, AgentKind::OpenCode).unwrap(),
        HooksState::Installed
    );

    uninstall(&target, AgentKind::OpenCode).unwrap();
    assert!(!path.exists());
    assert_eq!(
        read(&config)["plugin"],
        serde_json::json!(["another-plugin"])
    );
    uninstall(&target, AgentKind::OpenCode).unwrap();

    // Somebody else's plugin file is left alone by install and uninstall alike.
    std::fs::write(&path, "export const Mine = async () => ({})\n").unwrap();
    assert_eq!(
        state(&target, AgentKind::OpenCode).unwrap(),
        HooksState::Missing
    );
    assert!(install(&target, AgentKind::OpenCode).is_err());
    assert!(uninstall(&target, AgentKind::OpenCode).is_err());
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "export const Mine = async () => ({})\n"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn a_symlinked_settings_file_is_written_through_and_an_unchanged_one_is_left_alone() {
    let (target, root) = target();
    let real = root.join("dotfiles/claude-settings.json");
    std::fs::create_dir_all(real.parent().unwrap()).unwrap();
    std::fs::write(&real, "{\n  \"model\": \"opus\"\n}\n").unwrap();
    let link = target.path(AgentKind::Claude);
    std::fs::create_dir_all(link.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&real, &link).unwrap();

    install(&target, AgentKind::Claude).unwrap();
    assert!(
        std::fs::symlink_metadata(&link).unwrap().is_symlink(),
        "link survives"
    );
    assert!(
        read(&real)["hooks"]["Stop"].is_array(),
        "the real file got the hooks"
    );

    let before = std::fs::metadata(&real).unwrap().modified().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));
    install(&target, AgentKind::Claude).unwrap();
    uninstall(&target, AgentKind::Codex).unwrap_or_else(|_| target.path(AgentKind::Codex));
    assert_eq!(
        std::fs::metadata(&real).unwrap().modified().unwrap(),
        before,
        "an identical install does not rewrite"
    );
    std::fs::remove_dir_all(root).unwrap();
}
