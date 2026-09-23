//! Installing Condr's hooks into an agent CLI's own configuration (ADR 0014).
//!
//! Every hook runs `condr agent-hook <agent> <event>`. Agents whose hooks live in a JSON
//! map (`{"hooks": {"<Event>": [{"matcher": …, "hooks": [{"type": "command", …}]}]}}`)
//! get Condr's entries merged in beside the user's, recognizable by the command they run,
//! so they can be reported, refreshed and removed without touching anything else. OpenCode
//! takes a plugin instead: a file Condr owns outright in its plugin directory, bridging
//! OpenCode's event stream onto the same command.

use super::{AgentEventKind, AgentKind};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io;
use std::path::{Path, PathBuf};

/// Where and how hooks are installed on one machine.
#[derive(Clone, Debug)]
pub struct HookTarget {
    /// Claude Code's configuration directory (`CLAUDE_CONFIG_DIR` or `~/.claude`).
    pub claude_dir: PathBuf,
    /// Codex's configuration directory (`CODEX_HOME` or `~/.codex`).
    pub codex_dir: PathBuf,
    /// OpenCode's global configuration directory (`$XDG_CONFIG_HOME/opencode` or
    /// `~/.config/opencode`, on every platform).
    pub opencode_dir: PathBuf,
    pub pi_dir: PathBuf,
    pub omp_dir: PathBuf,
    pub antigravity_dir: PathBuf,
    pub grok_dir: PathBuf,
    pub cursor_dir: PathBuf,
    pub copilot_dir: PathBuf,
    pub kimi_dir: PathBuf,
    /// How a hook command names this `condr`: a bare name when PATH resolves it, otherwise
    /// the quoted absolute path.
    pub command: String,
}

impl HookTarget {
    /// Default locations under a home directory, also used for isolated installations.
    pub fn in_home(home: &Path, command: String) -> Self {
        Self {
            claude_dir: home.join(".claude"),
            codex_dir: home.join(".codex"),
            opencode_dir: home.join(".config/opencode"),
            pi_dir: home.join(".pi/agent"),
            omp_dir: home.join(".omp/agent"),
            antigravity_dir: home.join(".gemini/antigravity-cli"),
            grok_dir: home.join(".grok"),
            cursor_dir: home.join(".cursor"),
            copilot_dir: home.join(".copilot"),
            kimi_dir: home.join(".kimi"),
            command,
        }
    }

    /// This machine and this executable.
    pub fn local() -> Option<Self> {
        let home = dirs::home_dir()?;
        let exe = std::env::current_exe().ok()?;
        // OMP joins PI_CONFIG_DIR to home, even when it starts with a separator.
        let mut omp_root = home.as_os_str().to_os_string();
        omp_root.push(std::path::MAIN_SEPARATOR_STR);
        omp_root.push(env_dir("PI_CONFIG_DIR", PathBuf::from(".omp")));
        Some(Self {
            claude_dir: std::env::var_os("CLAUDE_CONFIG_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".claude")),
            codex_dir: std::env::var_os("CODEX_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".codex")),
            opencode_dir: std::env::var_os("XDG_CONFIG_HOME")
                .filter(|dir| !dir.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".config"))
                .join("opencode"),
            pi_dir: env_dir("PI_CODING_AGENT_DIR", home.join(".pi/agent")),
            omp_dir: env_dir("PI_CODING_AGENT_DIR", PathBuf::from(omp_root).join("agent")),
            grok_dir: env_dir("GROK_HOME", home.join(".grok")),
            cursor_dir: env_dir(
                "CURSOR_CONFIG_DIR",
                std::env::var_os("XDG_CONFIG_HOME")
                    .filter(|value| !value.is_empty())
                    .map(PathBuf::from)
                    .map_or_else(|| home.join(".cursor"), |dir| dir.join("cursor")),
            ),
            copilot_dir: env_dir("COPILOT_HOME", home.join(".copilot")),
            kimi_dir: env_dir("KIMI_SHARE_DIR", home.join(".kimi")),
            ..Self::in_home(&home, command_name(&exe))
        })
    }

    /// The file Condr's hooks for `agent` live in.
    pub fn path(&self, agent: AgentKind) -> PathBuf {
        match agent {
            AgentKind::Claude => self.claude_dir.join("settings.json"),
            AgentKind::Codex => self.codex_dir.join("hooks.json"),
            AgentKind::OpenCode => self.opencode_dir.join("condr-tui.js"),
            AgentKind::Pi => self.pi_dir.join("extensions/condr-pi.ts"),
            AgentKind::Omp => self.omp_dir.join("extensions/condr-omp.ts"),
            AgentKind::Antigravity => self.antigravity_dir.join("plugins/condr/hooks.json"),
            AgentKind::Grok => self.grok_dir.join("hooks/condr.json"),
            AgentKind::Cursor => self.cursor_dir.join("hooks.json"),
            AgentKind::Copilot => self.copilot_dir.join("hooks/condr.json"),
            AgentKind::Kimi => self.kimi_dir.join("config.toml"),
            // Commands never carry `Other`: the wire refuses it (ADR 0028), and `run`
            // turns it away before asking for a path.
            AgentKind::Other => unreachable!("no hooks exist for an unknown agent"),
        }
    }

    /// The executable as a single shell word, for a runtime that does its own quoting.
    fn executable(&self) -> &str {
        self.command.trim_matches('"')
    }

    fn hook_command(&self, agent: AgentKind, event: AgentEventKind) -> String {
        self.hook_command_for_shell(
            agent,
            event,
            cfg!(windows),
            std::env::var("GROK_SHELL").ok().as_deref(),
        )
    }

    fn hook_command_for_shell(
        &self,
        agent: AgentKind,
        event: AgentEventKind,
        windows: bool,
        grok_shell: Option<&str>,
    ) -> String {
        // PowerShell needs the call operator before a quoted path. Grok defaults
        // to PowerShell on Windows, but also lets users select Git Bash or cmd.
        // Claude's Git Bash and Antigravity's cmd must keep the plain form.
        let powershell = agent == AgentKind::Codex
            || (agent == AgentKind::Grok
                && !matches!(
                    grok_shell
                        .map(|shell| shell.trim().to_ascii_lowercase())
                        .as_deref(),
                    Some("bash" | "gitbash" | "git-bash" | "cmd" | "cmd.exe")
                ));
        let call = if windows && powershell && self.command.starts_with('"') {
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

fn env_dir(name: &str, fallback: PathBuf) -> PathBuf {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or(fallback)
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

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HooksState {
    /// Exactly the hooks this `condr` would install.
    Installed,
    /// Condr hooks are present, but not the ones this `condr` would install.
    Outdated,
    Missing,
    /// The native CLI cannot yet report a reliable main-agent state.
    Unsupported,
}

impl HooksState {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Installed => "installed",
            Self::Outdated => "outdated",
            Self::Missing => "missing",
            Self::Unsupported => "unsupported",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HooksAction {
    Install,
    Uninstall,
    Status,
}

/// What one action left behind: the file the hooks live in and its state afterwards.
/// `note` is standing advice for the agent; `warning` is a step the install could not
/// finish on the user's behalf.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
// No `skip_serializing_if`: bincode has no field names, so a skipped field is read
// as the next one and the frame ends early.
pub struct HooksReport {
    pub agent: AgentKind,
    pub path: PathBuf,
    pub state: HooksState,
    pub note: Option<String>,
    pub warning: Option<String>,
}

const CODEX_NOTE: &str = "Codex runs hooks only with the hooks feature enabled and, unless managed, after they are trusted from /hooks inside Codex";
const KIMI_NOTE: &str = "Kimi 1.50 hooks cannot distinguish a subagent's Stop from the main agent's Stop; status integration is unavailable until native hooks identify their agent";

/// Performs `action` for `agent` on this machine and reports the resulting state. The
/// CLI runs it locally; the Server runs it for a GUI, whose hooks files live where the
/// agents run.
pub fn run(target: &HookTarget, agent: AgentKind, action: HooksAction) -> io::Result<HooksReport> {
    if agent == AgentKind::Other {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "this build does not know that agent",
        ));
    }
    let mut warning = None;
    let path = match action {
        HooksAction::Install => {
            let path = install(target, agent)?;
            if agent == AgentKind::Codex {
                warning = enable_codex_hooks().err();
            }
            path
        }
        HooksAction::Uninstall => uninstall(target, agent)?,
        HooksAction::Status => target.path(agent),
    };
    Ok(HooksReport {
        agent,
        path,
        state: state(target, agent)?,
        note: match agent {
            AgentKind::Codex => Some(CODEX_NOTE),
            AgentKind::Pi => Some("Requires Pi 0.85.1 or newer for settled and UI prompt events"),
            AgentKind::Omp => Some("Requires Oh My Pi 18.1.17 or newer; install into each profile's agent directory when using named profiles"),
            AgentKind::Kimi => Some(KIMI_NOTE),
            AgentKind::Cursor => Some("Cursor hooks report activity and completion, but expose no event for a permission prompt"),
            AgentKind::Antigravity => Some("Enable the condr plugin in Antigravity CLI; hooks expose no permission-wait event, and startup stays Unknown until the first invocation"),
            AgentKind::Grok => Some("Grok Stop hooks can request continuation; completion may appear early when other Stop hooks block. Native idle notifications repair missed completion reports"),
            AgentKind::Copilot => Some("Copilot can omit its completion hook after an API error; status may stay Working until the next completion or process exit"),
            _ => None,
        }.map(str::to_owned),
        warning,
    })
}

/// Codex ignores hooks.json until its hooks feature is on; the user can also set
/// `[features] hooks = true` in config.toml by hand, which the error text says.
fn enable_codex_hooks() -> Result<(), String> {
    // Discovery also finds `codex.cmd`, which a bare `Command::new("codex")` cannot on
    // Windows.
    let codex = crate::agent_discovery::discover()
        .into_iter()
        .find(|found| found.kind == AgentKind::Codex)
        .map_or_else(
            || AgentKind::Codex.executable().into(),
            |found| found.executable,
        );
    let hint = "set [features] hooks = true in Codex's config.toml";
    match std::process::Command::new(codex)
        .args(["features", "enable", "hooks"])
        .stdin(std::process::Stdio::null())
        .output()
    {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) => Err(format!(
            "codex features enable hooks failed ({}): {}; {hint}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )),
        Err(error) => Err(format!(
            "could not run codex to enable hooks: {error}; {hint}"
        )),
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

const GROK_HOOKS: &[HookEntry] = &[
    entry("SessionStart", AgentEventKind::SessionStart),
    entry("UserPromptSubmit", AgentEventKind::PromptSubmit),
    entry("PreToolUse", AgentEventKind::ToolStart),
    entry("PostToolUse", AgentEventKind::ToolComplete),
    entry("PostToolUseFailure", AgentEventKind::ToolComplete),
    HookEntry {
        native: "Notification",
        event: AgentEventKind::PermissionRequest,
        matcher: Some("permission_prompt"),
        timeout_secs: HOOK_TIMEOUT_SECS,
    },
    HookEntry {
        native: "Notification",
        event: AgentEventKind::Stop,
        matcher: Some("idle_prompt"),
        timeout_secs: HOOK_TIMEOUT_SECS,
    },
    entry("Stop", AgentEventKind::Stop),
    entry("StopFailure", AgentEventKind::StopFailure),
    entry("StopCancelled", AgentEventKind::Interrupt),
];

const CURSOR_HOOKS: &[HookEntry] = &[
    entry("sessionStart", AgentEventKind::SessionStart),
    entry("beforeSubmitPrompt", AgentEventKind::PromptSubmit),
    entry("preToolUse", AgentEventKind::ToolStart),
    entry("postToolUse", AgentEventKind::ToolComplete),
    entry("postToolUseFailure", AgentEventKind::ToolComplete),
    entry("stop", AgentEventKind::Stop),
];

const COPILOT_HOOKS: &[HookEntry] = &[
    entry("sessionStart", AgentEventKind::SessionStart),
    entry("userPromptSubmitted", AgentEventKind::PromptSubmit),
    entry("preToolUse", AgentEventKind::ToolStart),
    entry("postToolUse", AgentEventKind::ToolComplete),
    entry("postToolUseFailure", AgentEventKind::ToolComplete),
    HookEntry {
        native: "notification",
        event: AgentEventKind::PermissionRequest,
        matcher: Some("permission_prompt|elicitation_dialog"),
        timeout_secs: HOOK_TIMEOUT_SECS,
    },
    entry("agentStop", AgentEventKind::Stop),
];

/// Seconds Claude Code and Codex wait for the hook by default; it finishes in milliseconds.
const HOOK_TIMEOUT_SECS: u64 = 5;
const MAX_CONFIG_BYTES: u64 = 4 * 1024 * 1024;

fn entries(agent: AgentKind) -> &'static [HookEntry] {
    match agent {
        AgentKind::Claude => CLAUDE_HOOKS,
        AgentKind::Codex => CODEX_HOOKS,
        AgentKind::Grok => GROK_HOOKS,
        AgentKind::Cursor => CURSOR_HOOKS,
        AgentKind::Copilot => COPILOT_HOOKS,
        _ => unreachable!("agent does not take a hook map"),
    }
}

/// OpenCode's TUI owns the selected conversation; backend events may belong to other
/// roots or subagents. This plugin reads the current route and its native status.
fn opencode_plugin(target: &HookTarget) -> String {
    include_str!("opencode.js").replace(
        "__CONDR_EXECUTABLE__",
        &serde_json::to_string(target.executable()).expect("string serializes"),
    )
}

fn extension(target: &HookTarget, agent: AgentKind) -> String {
    include_str!("pi-extension.js")
        .replace(
            "__CONDR_EXECUTABLE__",
            &serde_json::to_string(target.executable()).expect("string serializes"),
        )
        .replace(
            "__CONDR_AGENT__",
            &serde_json::to_string(agent.id()).expect("string serializes"),
        )
        .replace("__CONDR_AGENT_MARKER__", agent.id())
}

fn owned_state(path: &Path, expected: &str, marker: &str) -> io::Result<HooksState> {
    Ok(match std::fs::read_to_string(path) {
        Ok(contents) if contents == expected => HooksState::Installed,
        Ok(contents) if contents.contains(marker) => HooksState::Outdated,
        Ok(_) => HooksState::Missing,
        Err(error) if error.kind() == io::ErrorKind::NotFound => HooksState::Missing,
        Err(error) => return Err(error),
    })
}

fn check_owned(path: &Path, marker: &str) -> io::Result<()> {
    match std::fs::read_to_string(path) {
        Ok(contents) if !contents.contains(marker) => {
            Err(invalid(path, "exists and was not written by condr"))
        }
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

fn antigravity_hooks(target: &HookTarget) -> Value {
    let command = |event| json!({"type": "command", "command": target.hook_command(AgentKind::Antigravity, event), "timeout": HOOK_TIMEOUT_SECS});
    json!({"condr": {
        "PreInvocation": [command(AgentEventKind::PromptSubmit)],
        "PreToolUse": [{"matcher": "*", "hooks": [command(AgentEventKind::ToolStart)]}],
        "PostToolUse": [{"matcher": "*", "hooks": [command(AgentEventKind::ToolComplete)]}],
        "Stop": [command(AgentEventKind::Stop)],
    }})
}

fn antigravity_manifest() -> Value {
    json!({"name": "condr", "description": "Condr agent status hooks (generated by condr)"})
}

fn check_manifest(path: &Path) -> io::Result<()> {
    if read_config(path)?.is_some_and(|value| value != antigravity_manifest()) {
        return Err(invalid(path, "exists and was not written by condr"));
    }
    Ok(())
}

const OPENCODE_PLUGIN: &str = "./condr-tui.js";

fn opencode_config(target: &HookTarget) -> io::Result<(PathBuf, Value)> {
    let path = target.opencode_dir.join("tui.json");
    let mut root = read_config(&path)?.unwrap_or_else(|| json!({}));
    let plugins = root
        .as_object_mut()
        .expect("read_config checked")
        .entry("plugin")
        .or_insert_with(|| json!([]));
    if !plugins.is_array() {
        return Err(invalid(&path, "plugin is not an array"));
    }
    Ok((path, root))
}

/// What marks the plugin file as Condr's, whichever `condr` wrote it.
const OPENCODE_MARKER: &str = "condr agent-hook opencode";

/// What marks a hook command as Condr's, whichever `condr` wrote it.
fn marker(agent: AgentKind) -> String {
    format!(" agent-hook {} ", agent.id())
}

fn is_ours(hook: &Value, agent: AgentKind) -> bool {
    hook.get("command")
        .and_then(Value::as_str)
        .is_some_and(|command| command.contains(&marker(agent)))
        || hook.get("args").and_then(Value::as_array).is_some_and(|args| {
            matches!(args.as_slice(), [first, second, ..] if first == "agent-hook" && second == agent.id())
        })
}

/// The matcher group Condr installs for one entry.
fn expected_group(target: &HookTarget, agent: AgentKind, entry: &HookEntry) -> Value {
    if matches!(agent, AgentKind::Cursor | AgentKind::Copilot) {
        let mut hook = if agent == AgentKind::Copilot {
            json!({"type": "command", "exec": target.executable(),
                "args": ["agent-hook", agent.id(), entry.event.name()], "timeoutSec": entry.timeout_secs})
        } else {
            json!({"command": target.hook_command(agent, entry.event), "timeout": entry.timeout_secs})
        };
        if let Some(matcher) = entry.matcher {
            hook["matcher"] = json!(matcher);
        }
        return hook;
    }
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
    if agent == AgentKind::Kimi {
        return Ok(HooksState::Unsupported);
    }
    if matches!(agent, AgentKind::Pi | AgentKind::Omp) {
        return owned_state(&path, &extension(target, agent), &marker(agent));
    }
    if agent == AgentKind::Antigravity {
        let Some(config) = read_config(&path)? else {
            return Ok(HooksState::Missing);
        };
        let manifest = read_config(&path.with_file_name("plugin.json"))?;
        return Ok(
            if config == antigravity_hooks(target) && manifest == Some(antigravity_manifest()) {
                HooksState::Installed
            } else if config.to_string().contains(&marker(agent)) {
                HooksState::Outdated
            } else {
                HooksState::Missing
            },
        );
    }
    if agent == AgentKind::OpenCode {
        let (_, config) = opencode_config(target)?;
        let registered = config["plugin"]
            .as_array()
            .expect("checked plugin list")
            .contains(&json!(OPENCODE_PLUGIN));
        return Ok(match std::fs::read_to_string(&path) {
            Ok(contents) if registered && contents == opencode_plugin(target) => {
                HooksState::Installed
            }
            Ok(contents) if contents.contains(OPENCODE_MARKER) => HooksState::Outdated,
            Ok(_) => HooksState::Missing,
            Err(error) if error.kind() == io::ErrorKind::NotFound => HooksState::Missing,
            Err(error) => return Err(error),
        });
    }
    let Some(root) = read_config(&path)? else {
        return Ok(HooksState::Missing);
    };
    let mut found = Vec::new();
    if let Some(events) = root.get("hooks").and_then(Value::as_object) {
        for (native, groups) in events {
            for group in groups.as_array().into_iter().flatten() {
                let hooks = group.get("hooks").and_then(Value::as_array);
                if is_ours(group, agent)
                    || hooks.is_some_and(|hooks| hooks.iter().any(|hook| is_ours(hook, agent)))
                {
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
    let version_ok =
        !matches!(agent, AgentKind::Cursor | AgentKind::Copilot) || root["version"] == 1;
    Ok(if found == expected && version_ok {
        HooksState::Installed
    } else {
        HooksState::Outdated
    })
}

/// Writes this `condr`'s hooks, replacing any earlier Condr entries and leaving the
/// user's alone. Returns the file written.
pub fn install(target: &HookTarget, agent: AgentKind) -> io::Result<PathBuf> {
    let path = target.path(agent);
    if agent == AgentKind::Kimi {
        return Err(io::Error::new(io::ErrorKind::Unsupported, KIMI_NOTE));
    }
    if matches!(agent, AgentKind::Pi | AgentKind::Omp) {
        check_owned(&path, &marker(agent))?;
        write_file(&path, extension(target, agent).as_bytes())?;
        return Ok(path);
    }
    if agent == AgentKind::Antigravity {
        let manifest = path.with_file_name("plugin.json");
        check_owned(&path, &marker(agent))?;
        check_manifest(&manifest)?;
        write_config(&manifest, &antigravity_manifest())?;
        write_config(&path, &antigravity_hooks(target))?;
        return Ok(path);
    }
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
        let (config_path, mut config) = opencode_config(target)?;
        let plugins = config["plugin"]
            .as_array_mut()
            .expect("checked plugin list");
        if !plugins.contains(&json!(OPENCODE_PLUGIN)) {
            plugins.push(json!(OPENCODE_PLUGIN));
        }
        write_file(&path, opencode_plugin(target).as_bytes())?;
        write_config(&config_path, &config)?;
        return Ok(path);
    }
    let mut root = read_config(&path)?.unwrap_or_else(|| json!({}));
    if matches!(agent, AgentKind::Cursor | AgentKind::Copilot) {
        if root.get("version").is_some_and(|version| version != 1) {
            return Err(invalid(&path, "unsupported hooks configuration version"));
        }
        root["version"] = json!(1);
    }
    let hooks = hooks_map(&mut root)?;
    strip_ours(hooks, agent);
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
    if agent == AgentKind::Kimi {
        return Ok(path);
    }
    if matches!(agent, AgentKind::Pi | AgentKind::Omp) {
        check_owned(&path, &marker(agent))?;
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        return Ok(path);
    }
    if agent == AgentKind::Antigravity {
        let manifest = path.with_file_name("plugin.json");
        check_owned(&path, &marker(agent))?;
        check_manifest(&manifest)?;
        for file in [&path, &manifest] {
            if file.exists() {
                std::fs::remove_file(file)?;
            }
        }
        // A disabled/enabled preference still belongs to the user, as do other files.
        return Ok(path);
    }
    if agent == AgentKind::OpenCode {
        let (config_path, mut config) = opencode_config(target)?;
        match std::fs::read_to_string(&path) {
            Ok(contents) if contents.contains(OPENCODE_MARKER) => std::fs::remove_file(&path)?,
            Ok(_) => return Err(invalid(&path, "exists and was not written by condr")),
            Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
            Err(_) => {}
        }
        config["plugin"]
            .as_array_mut()
            .expect("checked plugin list")
            .retain(|plugin| plugin != OPENCODE_PLUGIN);
        if config_path.exists() {
            write_config(&config_path, &config)?;
        }
        return Ok(path);
    }
    let Some(mut root) = read_config(&path)? else {
        return Ok(path);
    };
    if let Some(hooks) = root.get_mut("hooks").and_then(Value::as_object_mut) {
        strip_ours(hooks, agent);
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
fn strip_ours(hooks: &mut serde_json::Map<String, Value>, agent: AgentKind) {
    for groups in hooks.values_mut() {
        let Some(groups) = groups.as_array_mut() else {
            continue;
        };
        for group in groups.iter_mut() {
            if let Some(entries) = group.get_mut("hooks").and_then(Value::as_array_mut) {
                entries.retain(|hook| !is_ours(hook, agent));
            }
        }
        groups.retain(|group| {
            !is_ours(group, agent)
                && group
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
        let target = HookTarget::in_home(&root, "\"/opt/condr bin/condr\"".to_owned());
        (target, root)
    }

    fn read(path: &Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn quoted_hook_paths_follow_the_cli_shell() {
        let (target, _) = target();
        let command = |agent, windows, shell| {
            target.hook_command_for_shell(agent, AgentEventKind::Stop, windows, shell)
        };
        for shell in [None, Some("pwsh"), Some("PowerShell"), Some("unknown")] {
            assert_eq!(
                command(AgentKind::Grok, true, shell),
                "& \"/opt/condr bin/condr\" agent-hook grok stop"
            );
            assert_eq!(
                command(AgentKind::Grok, false, shell),
                "\"/opt/condr bin/condr\" agent-hook grok stop"
            );
        }
        for shell in ["bash", "gitbash", "git-bash", "cmd", " CMD.EXE "] {
            assert_eq!(
                command(AgentKind::Grok, true, Some(shell)),
                "\"/opt/condr bin/condr\" agent-hook grok stop"
            );
        }
        assert_eq!(
            command(AgentKind::Codex, true, Some("bash")),
            "& \"/opt/condr bin/condr\" agent-hook codex stop",
            "GROK_SHELL does not select Codex's shell"
        );
        for agent in [AgentKind::Claude, AgentKind::Cursor, AgentKind::Antigravity] {
            assert_eq!(
                command(agent, true, None),
                format!("\"/opt/condr bin/condr\" agent-hook {} stop", agent.id())
            );
        }
        let target = HookTarget {
            command: "condr".into(),
            ..target
        };
        assert_eq!(
            target.hook_command_for_shell(AgentKind::Grok, AgentEventKind::Stop, true, None),
            "condr agent-hook grok stop"
        );
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
}
