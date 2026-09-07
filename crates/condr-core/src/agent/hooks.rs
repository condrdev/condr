//! Installing Condr's hooks into an agent CLI's own configuration (ADR 0014).
//!
//! Every hook runs `condr agent-hook <agent> <event>`. Agents whose hooks live in a JSON
//! map (`{"hooks": {"<Event>": [{"matcher": …, "hooks": [{"type": "command", …}]}]}}`)
//! get Condr's entries merged in beside the user's, recognizable by the command they run,
//! so they can be reported, refreshed and removed without touching anything else.

use std::io;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::{AgentEventKind, AgentKind};

/// Where and how hooks are installed on one machine.
#[derive(Clone, Debug)]
pub struct HookTarget {
    /// Claude Code's configuration directory (`CLAUDE_CONFIG_DIR` or `~/.claude`).
    pub claude_dir: PathBuf,
    /// Codex's configuration directory (`CODEX_HOME` or `~/.codex`).
    pub codex_dir: PathBuf,
    /// How a hook command names this `condr`: a bare name when PATH resolves it, otherwise
    /// the quoted absolute path.
    pub command: String,
}

impl HookTarget {
    /// This machine and this executable.
    pub fn local() -> Option<Self> {
        let home = dirs::home_dir()?;
        let exe = std::env::current_exe().ok()?;
        Some(Self {
            claude_dir: std::env::var_os("CLAUDE_CONFIG_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".claude")),
            codex_dir: std::env::var_os("CODEX_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".codex")),
            command: command_name(&exe),
        })
    }

    /// The file Condr's hooks for `agent` live in; `None` for an agent Condr has no hooks
    /// for yet.
    pub fn path(&self, agent: AgentKind) -> Option<PathBuf> {
        match agent {
            AgentKind::Claude => Some(self.claude_dir.join("settings.json")),
            AgentKind::Codex => Some(self.codex_dir.join("hooks.json")),
            AgentKind::OpenCode => None,
        }
    }

    fn hook_command(&self, agent: AgentKind, event: AgentEventKind) -> String {
        format!(
            "{} agent-hook {} {}",
            self.command,
            agent.id(),
            event.name()
        )
    }
}

/// `condr` if that is what PATH resolves `exe` to, so the hook survives a moved install
/// and needs no quoting under PowerShell; otherwise the absolute path, quoted.
fn command_name(exe: &Path) -> String {
    let resolved = std::fs::canonicalize(exe).ok();
    let on_path = std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|dir| {
            let candidate = dir.join(if cfg!(windows) { "condr.exe" } else { "condr" });
            std::fs::canonicalize(candidate).ok() == resolved && resolved.is_some()
        })
    });
    if on_path {
        "condr".to_owned()
    } else {
        format!("\"{}\"", exe.display())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HooksState {
    /// Exactly the hooks this `condr` would install.
    Installed,
    /// Condr hooks are present, but not the ones this `condr` would install.
    Outdated,
    Missing,
}

impl HooksState {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Installed => "installed",
            Self::Outdated => "outdated",
            Self::Missing => "missing",
        }
    }
}

/// One native hook event and what it reports as.
struct HookEntry {
    native: &'static str,
    event: AgentEventKind,
    /// The native matcher, for events that would otherwise report things that do not
    /// block the user.
    matcher: Option<&'static str>,
}

const fn entry(native: &'static str, event: AgentEventKind) -> HookEntry {
    HookEntry {
        native,
        event,
        matcher: None,
    }
}

/// Claude Code. `Notification` is narrowed to the kinds that wait on the user; a user
/// interrupt fires nothing, which ADR 0014 accepts.
const CLAUDE_HOOKS: &[HookEntry] = &[
    entry("SessionStart", AgentEventKind::SessionStart),
    entry("UserPromptSubmit", AgentEventKind::PromptSubmit),
    entry("PermissionRequest", AgentEventKind::PermissionRequest),
    HookEntry {
        native: "Notification",
        event: AgentEventKind::PermissionRequest,
        matcher: Some("permission_prompt|elicitation_dialog"),
    },
    entry("PreToolUse", AgentEventKind::ToolStart),
    entry("PostToolUse", AgentEventKind::ToolComplete),
    entry("Stop", AgentEventKind::Stop),
    entry("StopFailure", AgentEventKind::StopFailure),
];

/// Codex. Hooks must be enabled (`codex features enable hooks`) and, unless managed,
/// trusted from `/hooks` inside Codex before they run.
const CODEX_HOOKS: &[HookEntry] = &[
    entry("SessionStart", AgentEventKind::SessionStart),
    entry("UserPromptSubmit", AgentEventKind::PromptSubmit),
    entry("PermissionRequest", AgentEventKind::PermissionRequest),
    entry("PreToolUse", AgentEventKind::ToolStart),
    entry("PostToolUse", AgentEventKind::ToolComplete),
    entry("Stop", AgentEventKind::Stop),
    entry("Interrupt", AgentEventKind::Interrupt),
];

/// Seconds Claude Code and Codex wait for the hook; it finishes in milliseconds.
const HOOK_TIMEOUT_SECS: u64 = 5;
const MAX_CONFIG_BYTES: u64 = 4 * 1024 * 1024;

fn entries(agent: AgentKind) -> &'static [HookEntry] {
    match agent {
        AgentKind::Claude => CLAUDE_HOOKS,
        AgentKind::Codex => CODEX_HOOKS,
        AgentKind::OpenCode => &[],
    }
}

fn path_of(target: &HookTarget, agent: AgentKind) -> io::Result<PathBuf> {
    target.path(agent).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::Unsupported,
            format!("Condr has no hooks for {}", agent.label()),
        )
    })
}

/// What marks a hook command as Condr's, whichever `condr` wrote it.
fn marker(agent: AgentKind) -> String {
    format!(" agent-hook {} ", agent.id())
}

fn is_ours(hook: &Value, marker: &str) -> bool {
    hook.get("command")
        .and_then(Value::as_str)
        .is_some_and(|command| command.contains(marker))
}

/// The matcher group Condr installs for one entry.
fn expected_group(target: &HookTarget, agent: AgentKind, entry: &HookEntry) -> Value {
    let mut group = json!({
        "hooks": [{
            "type": "command",
            "command": target.hook_command(agent, entry.event),
            "timeout": HOOK_TIMEOUT_SECS,
        }]
    });
    if let Some(matcher) = entry.matcher {
        group["matcher"] = Value::String(matcher.to_owned());
    }
    group
}

pub fn state(target: &HookTarget, agent: AgentKind) -> io::Result<HooksState> {
    let path = path_of(target, agent)?;
    let Some(root) = read_config(&path)? else {
        return Ok(HooksState::Missing);
    };
    let marker = marker(agent);
    let mut found = Vec::new();
    if let Some(events) = root.get("hooks").and_then(Value::as_object) {
        for (native, groups) in events {
            for group in groups.as_array().into_iter().flatten() {
                let hooks = group.get("hooks").and_then(Value::as_array);
                if hooks.is_some_and(|hooks| hooks.iter().any(|hook| is_ours(hook, &marker))) {
                    found.push((native.clone(), group.clone()));
                }
            }
        }
    }
    if found.is_empty() {
        return Ok(HooksState::Missing);
    }
    let mut expected: Vec<(String, Value)> = entries(agent)
        .iter()
        .map(|entry| {
            (
                entry.native.to_owned(),
                expected_group(target, agent, entry),
            )
        })
        .collect();
    expected.sort_by(|a, b| a.0.cmp(&b.0));
    found.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(if found == expected {
        HooksState::Installed
    } else {
        HooksState::Outdated
    })
}

/// Writes this `condr`'s hooks, replacing any earlier Condr entries and leaving the
/// user's alone. Returns the file written.
pub fn install(target: &HookTarget, agent: AgentKind) -> io::Result<PathBuf> {
    let path = path_of(target, agent)?;
    let mut root = read_config(&path)?.unwrap_or_else(|| json!({}));
    let marker = marker(agent);
    let hooks = hooks_map(&mut root)?;
    strip_ours(hooks, &marker);
    for entry in entries(agent) {
        let groups = hooks
            .entry(entry.native)
            .or_insert_with(|| Value::Array(Vec::new()));
        let Some(groups) = groups.as_array_mut() else {
            return Err(invalid(
                &path,
                format!("hooks.{} is not an array", entry.native),
            ));
        };
        groups.push(expected_group(target, agent, entry));
    }
    write_config(&path, &root)?;
    Ok(path)
}

/// Removes every Condr entry, leaving the rest of the file as it was.
pub fn uninstall(target: &HookTarget, agent: AgentKind) -> io::Result<PathBuf> {
    let path = path_of(target, agent)?;
    let Some(mut root) = read_config(&path)? else {
        return Ok(path);
    };
    let marker = marker(agent);
    if let Some(hooks) = root.get_mut("hooks").and_then(Value::as_object_mut) {
        strip_ours(hooks, &marker);
        hooks.retain(|_, groups| groups.as_array().is_none_or(|groups| !groups.is_empty()));
    }
    if root
        .get("hooks")
        .and_then(Value::as_object)
        .is_some_and(serde_json::Map::is_empty)
    {
        root.as_object_mut()
            .expect("read_config checked")
            .remove("hooks");
    }
    write_config(&path, &root)?;
    Ok(path)
}

fn hooks_map(root: &mut Value) -> io::Result<&mut serde_json::Map<String, Value>> {
    let object = root
        .as_object_mut()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "top level is not an object"))?;
    object
        .entry("hooks")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "\"hooks\" is not an object"))
}

/// Drops Condr's hooks from every group of every event, and the groups that emptied.
fn strip_ours(hooks: &mut serde_json::Map<String, Value>, marker: &str) {
    for groups in hooks.values_mut() {
        let Some(groups) = groups.as_array_mut() else {
            continue;
        };
        for group in groups.iter_mut() {
            if let Some(entries) = group.get_mut("hooks").and_then(Value::as_array_mut) {
                entries.retain(|hook| !is_ours(hook, marker));
            }
        }
        groups.retain(|group| {
            group
                .get("hooks")
                .and_then(Value::as_array)
                .is_none_or(|entries| !entries.is_empty())
        });
    }
}

/// `None` when the file does not exist. Anything unreadable, oversized or not a JSON
/// object is an error, so a broken file is never overwritten.
fn read_config(path: &Path) -> io::Result<Option<Value>> {
    let bytes = match std::fs::metadata(path) {
        Ok(metadata) if metadata.len() > MAX_CONFIG_BYTES => {
            return Err(invalid(path, "larger than 4 MiB"));
        }
        Ok(_) => std::fs::read(path)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return Ok(None);
    }
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|error| invalid(path, error.to_string()))?;
    if !value.is_object() {
        return Err(invalid(path, "top level is not a JSON object"));
    }
    Ok(Some(value))
}

fn write_config(path: &Path, root: &Value) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut text = serde_json::to_string_pretty(root)?;
    text.push('\n');
    let temporary = path.with_extension("json.condr-tmp");
    std::fs::write(&temporary, text)?;
    std::fs::rename(&temporary, path)
}

fn invalid(path: &Path, detail: impl std::fmt::Display) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("{}: {detail}", path.display()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target() -> (HookTarget, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "condr-hooks-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let target = HookTarget {
            claude_dir: root.join(".claude"),
            codex_dir: root.join(".codex"),
            command: "\"/opt/condr bin/condr\"".to_owned(),
        };
        (target, root)
    }

    fn read(path: &Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn install_is_idempotent_preserves_user_hooks_and_uninstall_leaves_only_theirs() {
        let (target, root) = target();
        let path = target.path(AgentKind::Claude).unwrap();
        assert_eq!(
            state(&target, AgentKind::Claude).unwrap(),
            HooksState::Missing
        );

        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let user = json!({
            "model": "opus",
            "hooks": {
                "Stop": [{"hooks": [{"type": "command", "command": "notify-send done"}]}],
                "PreToolUse": [{"matcher": "Bash", "hooks": [
                    {"type": "command", "command": "lint"},
                    {"type": "command", "command": "\"/old/condr\" agent-hook claude tool-start"}
                ]}]
            }
        });
        std::fs::write(&path, user.to_string()).unwrap();
        // An older condr's entry counts as ours, but not as current.
        assert_eq!(
            state(&target, AgentKind::Claude).unwrap(),
            HooksState::Outdated
        );

        assert_eq!(install(&target, AgentKind::Claude).unwrap(), path);
        assert_eq!(
            state(&target, AgentKind::Claude).unwrap(),
            HooksState::Installed
        );
        let once = read(&path);
        install(&target, AgentKind::Claude).unwrap();
        assert_eq!(read(&path), once, "a second install changes nothing");

        assert_eq!(once["model"], "opus");
        let stop = once["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop.len(), 2);
        assert_eq!(stop[0]["hooks"][0]["command"], "notify-send done");
        assert_eq!(
            stop[1]["hooks"][0]["command"],
            "\"/opt/condr bin/condr\" agent-hook claude stop"
        );
        assert_eq!(stop[1]["hooks"][0]["timeout"], 5);
        let pre = once["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(
            pre[0]["hooks"].as_array().unwrap().len(),
            1,
            "old condr entry gone"
        );
        assert_eq!(pre[0]["matcher"], "Bash");
        assert_eq!(
            once["hooks"]["Notification"][0]["matcher"],
            "permission_prompt|elicitation_dialog"
        );
        assert!(once["hooks"]["SessionStart"][0].get("matcher").is_none());
        assert_eq!(
            once["hooks"].as_object().unwrap().len(),
            CLAUDE_HOOKS
                .iter()
                .map(|entry| entry.native)
                .collect::<std::collections::BTreeSet<_>>()
                .len()
        );

        assert_eq!(uninstall(&target, AgentKind::Claude).unwrap(), path);
        let after = read(&path);
        assert_eq!(after["model"], "opus");
        assert_eq!(after["hooks"]["Stop"].as_array().unwrap().len(), 1);
        assert_eq!(
            after["hooks"]["PreToolUse"][0]["hooks"][0]["command"],
            "lint"
        );
        assert!(after["hooks"].get("SessionStart").is_none());
        assert_eq!(
            state(&target, AgentKind::Claude).unwrap(),
            HooksState::Missing
        );

        // Uninstalling everything that was ours alone removes the hooks object too.
        install(&target, AgentKind::Codex).unwrap();
        let codex = target.path(AgentKind::Codex).unwrap();
        assert!(read(&codex)["hooks"]["Interrupt"].is_array());
        uninstall(&target, AgentKind::Codex).unwrap();
        assert_eq!(read(&codex), json!({}));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_broken_or_oversized_settings_file_is_never_overwritten() {
        let (target, root) = target();
        let path = target.path(AgentKind::Claude).unwrap();
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
}
