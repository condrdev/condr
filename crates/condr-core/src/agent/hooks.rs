//! Installing Condr's hooks into an agent CLI's own configuration (ADR 0014).
//!
//! Every hook runs `condr agent-hook <agent> <event>`. Agents whose hooks live in a JSON
//! map (`{"hooks": {"<Event>": [{"matcher": …, "hooks": [{"type": "command", …}]}]}}`)
//! get Condr's entries merged in beside the user's, recognizable by the command they run,
//! so they can be reported, refreshed and removed without touching anything else. OpenCode
//! takes a plugin instead: a file Condr owns outright in its plugin directory, bridging
//! OpenCode's event stream onto the same command.

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
    /// OpenCode's global plugin directory (`$XDG_CONFIG_HOME/opencode/plugins` or
    /// `~/.config/opencode/plugins`, on every platform).
    pub opencode_plugins_dir: PathBuf,
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
            opencode_plugins_dir: std::env::var_os("XDG_CONFIG_HOME")
                .filter(|dir| !dir.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".config"))
                .join("opencode")
                .join("plugins"),
            command: command_name(&exe),
        })
    }

    /// The file Condr's hooks for `agent` live in.
    pub fn path(&self, agent: AgentKind) -> PathBuf {
        match agent {
            AgentKind::Claude => self.claude_dir.join("settings.json"),
            AgentKind::Codex => self.codex_dir.join("hooks.json"),
            AgentKind::OpenCode => self.opencode_plugins_dir.join("condr.js"),
        }
    }

    /// The executable as a single shell word, for a runtime that does its own quoting.
    fn executable(&self) -> &str {
        self.command.trim_matches('"')
    }

    fn hook_command(&self, agent: AgentKind, event: AgentEventKind) -> String {
        // Codex runs hooks through the session shell, PowerShell on Windows, where a quoted
        // path is a string literal unless the call operator precedes it. Claude Code runs
        // them through Git Bash, which would choke on the `&`.
        let call = if cfg!(windows) && agent == AgentKind::Codex && self.command.starts_with('"') {
            "& "
        } else {
            ""
        };
        format!(
            "{call}{} agent-hook {} {}",
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
    /// Seconds the agent waits for the hook; it finishes in milliseconds.
    timeout_secs: u64,
}

const fn entry(native: &'static str, event: AgentEventKind) -> HookEntry {
    HookEntry {
        native,
        event,
        matcher: None,
        timeout_secs: HOOK_TIMEOUT_SECS,
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
        timeout_secs: HOOK_TIMEOUT_SECS,
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
    // Codex clamps Interrupt hooks to 3 s and warns in the TUI about anything longer.
    HookEntry {
        native: "Interrupt",
        event: AgentEventKind::Interrupt,
        matcher: None,
        timeout_secs: 3,
    },
];

/// Seconds Claude Code and Codex wait for the hook by default; it finishes in milliseconds.
const HOOK_TIMEOUT_SECS: u64 = 5;
const MAX_CONFIG_BYTES: u64 = 4 * 1024 * 1024;

fn entries(agent: AgentKind) -> &'static [HookEntry] {
    match agent {
        AgentKind::Claude => CLAUDE_HOOKS,
        AgentKind::Codex => CODEX_HOOKS,
        AgentKind::OpenCode => unreachable!("OpenCode takes a plugin, not a hook map"),
    }
}

/// The plugin OpenCode loads at startup. It follows the user's root sessions, including
/// the ones `/new` starts later: a task subagent runs in a child session whose events
/// look the same, so children are remembered from `session.created` and skipped, or the
/// pane would report the subagent's turns as its own. Inert outside a Condr Pane.
fn opencode_plugin(target: &HookTarget) -> String {
    let exe = serde_json::to_string(target.executable()).expect("string serializes");
    format!(
        r#"// condr agent-hook opencode — generated by condr, do not edit.
// Reports this session's state to the Condr Pane it runs in (ADR 0014).
export const CondrAgentStatus = async ({{ $ }}) => {{
  if (process.env.CONDR_ENV !== "1") return {{}}
  const exe = {exe}
  const report = (event) => $`${{exe}} agent-hook opencode ${{event}}`.quiet().nothrow()
  // Root sessions are the user's own; `/new` starts another. Children are subagents.
  const roots = new Set()
  const children = new Set()
  const announced = new Set()
  const own = (id) => {{
    if (!id || children.has(id)) return false
    if (roots.size === 0) roots.add(id)
    return roots.has(id)
  }}
  const announce = async (id) => {{
    if (announced.has(id)) return
    announced.add(id)
    await report("session-start")
  }}
  return {{
    "tool.execute.before": async (input) => {{
      const id = input?.sessionID
      if (!own(id)) return
      await announce(id)
      await report("tool-start")
    }},
    event: async ({{ event }}) => {{
      const properties = event.properties ?? {{}}
      const info = properties.info
      if (info?.id) {{
        if (info.parentID) children.add(info.id)
        else if (event.type === "session.created") roots.add(info.id)
      }}
      const id = properties.sessionID ?? info?.id
      if (!own(id)) return
      await announce(id)
      const key = event.type === "session.status" ? `session.status.${{properties.status?.type}}` : event.type
      switch (key) {{
        case "session.status.busy": return report("prompt-submit")
        case "session.status.idle":
        case "session.idle": return report("stop")
        case "session.error": return report("stop-failure")
        case "permission.asked": return report("permission-request")
        case "question.asked": return report("question-asked")
        case "permission.replied":
        case "question.replied":
        case "question.rejected": return report("tool-complete")
      }}
    }},
  }}
}}
"#
    )
}

/// What marks the plugin file as Condr's, whichever `condr` wrote it.
const OPENCODE_MARKER: &str = "condr agent-hook opencode";

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
            "timeout": entry.timeout_secs,
        }]
    });
    if let Some(matcher) = entry.matcher {
        group["matcher"] = Value::String(matcher.to_owned());
    }
    group
}

pub fn state(target: &HookTarget, agent: AgentKind) -> io::Result<HooksState> {
    let path = target.path(agent);
    if agent == AgentKind::OpenCode {
        return Ok(match std::fs::read_to_string(&path) {
            Ok(contents) if contents == opencode_plugin(target) => HooksState::Installed,
            Ok(contents) if contents.contains(OPENCODE_MARKER) => HooksState::Outdated,
            Ok(_) => HooksState::Missing,
            Err(error) if error.kind() == io::ErrorKind::NotFound => HooksState::Missing,
            Err(error) => return Err(error),
        });
    }
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
    let path = target.path(agent);
    if agent == AgentKind::OpenCode {
        match std::fs::read_to_string(&path) {
            // Someone else's file at our name is theirs to keep.
            Ok(contents) if !contents.contains(OPENCODE_MARKER) => {
                return Err(invalid(&path, "exists and was not written by condr"));
            }
            Ok(_) => {}
            Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
            Err(_) => {}
        }
        write_file(&path, opencode_plugin(target).as_bytes())?;
        return Ok(path);
    }
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
    let path = target.path(agent);
    if agent == AgentKind::OpenCode {
        match std::fs::read_to_string(&path) {
            Ok(contents) if contents.contains(OPENCODE_MARKER) => std::fs::remove_file(&path)?,
            Ok(_) => return Err(invalid(&path, "exists and was not written by condr")),
            Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
            Err(_) => {}
        }
        return Ok(path);
    }
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
    let mut text = serde_json::to_string_pretty(root)?;
    text.push('\n');
    write_file(path, text.as_bytes())
}

/// Writes through a symlink (dotfile managers keep the real file elsewhere), replaces
/// the target atomically, and leaves an already identical file untouched so a status
/// check or a no-op uninstall never dirties the user's dotfiles.
fn write_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let path = match std::fs::canonicalize(path) {
        Ok(real) => real,
        Err(error) if error.kind() == io::ErrorKind::NotFound => path.to_path_buf(),
        Err(error) => return Err(error),
    };
    if std::fs::read(&path).is_ok_and(|current| current == bytes) {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("condr-tmp");
    std::fs::write(&temporary, bytes)?;
    std::fs::rename(&temporary, &path)
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
            opencode_plugins_dir: root.join(".config/opencode/plugins"),
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
        let path = target.path(AgentKind::Claude);
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
        // The user's key order survives; a no-op rewrite would show up as a dotfile diff.
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.find("\"model\"").unwrap() < text.find("\"hooks\"").unwrap());
        assert!(text.find("\"Stop\"").unwrap() < text.find("\"PreToolUse\"").unwrap());
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
        let codex = target.path(AgentKind::Codex);
        assert!(read(&codex)["hooks"]["Interrupt"].is_array());
        uninstall(&target, AgentKind::Codex).unwrap();
        assert_eq!(read(&codex), json!({}));
        std::fs::remove_dir_all(root).unwrap();
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
        assert!(path.ends_with(".config/opencode/plugins/condr.js"));
        assert_eq!(
            state(&target, AgentKind::OpenCode).unwrap(),
            HooksState::Missing
        );

        install(&target, AgentKind::OpenCode).unwrap();
        assert_eq!(
            state(&target, AgentKind::OpenCode).unwrap(),
            HooksState::Installed
        );
        let plugin = std::fs::read_to_string(&path).unwrap();
        assert!(plugin.contains(r#"const exe = "/opt/condr bin/condr""#));
        assert!(plugin.contains("agent-hook opencode ${event}"));
        assert!(plugin.contains(r#"process.env.CONDR_ENV !== "1""#));
        assert!(
            plugin.contains(r#"else if (event.type === "session.created") roots.add(info.id)"#)
        );
        for event in [
            "session-start",
            "prompt-submit",
            "stop",
            "stop-failure",
            "permission-request",
            "question-asked",
            "tool-complete",
            "tool-start",
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
        uninstall(&target, AgentKind::OpenCode).unwrap();

        // Somebody else's condr.js is left alone by install and uninstall alike.
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
}
