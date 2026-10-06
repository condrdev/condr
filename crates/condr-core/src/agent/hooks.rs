//! Installing Condr's hooks into an agent CLI's own configuration (ADR 0014).
//!
//! Every hook runs `condr agent-hook <agent> <event>`. Each fully supported agent's spec
//! names one of four formats (ADR 0035), and this module is the code they share. A JSON
//! hook map or flat list gets Condr's entries merged in beside the user's, recognizable by
//! the command they run, so they can be reported, refreshed and removed without touching
//! anything else. A plugin or a script is a file Condr owns outright.

use super::hook::HookInput;
use super::{AgentEventKind, AgentKind, AgentSupport};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io;
use std::path::{Path, PathBuf};

/// How one agent's hooks are written, and what its hook invocations need that only its
/// own input says (ADR 0035).
#[derive(Debug)]
pub struct HookSpec {
    pub(super) format: HookFormat,
    /// The CLI's configuration directory on `target`'s machine.
    pub(super) dir: fn(&HookTarget) -> PathBuf,
    /// The file Condr's hooks live in, under `dir`.
    pub(super) file: &'static str,
    /// The tool whose start is the agent asking the person a question.
    pub(super) question_tool: Option<&'static str>,
    /// Renames or drops a native event for what only this agent's input says.
    pub(super) adjust: Option<fn(&HookInput, AgentEventKind) -> Option<AgentEventKind>>,
    /// Whether each event names the turn it belongs to, so a late event from an earlier
    /// turn is dropped.
    pub(super) prompt_ids: bool,
    /// What the hook must print on stdout for the agent to accept it.
    pub(super) reply: Option<&'static str>,
    /// A step after writing the hooks without which the CLI ignores them; its error
    /// becomes the report's warning.
    pub(super) enable: Option<fn() -> Result<(), String>>,
    /// Standing advice Settings shows.
    pub(super) note: Option<&'static str>,
}

/// The four ways Condr writes an agent's hooks.
#[derive(Debug)]
pub(super) enum HookFormat {
    /// `{"hooks": {"<Event>": [{"hooks": [{"type": "command", …}], "matcher": …}]}}`,
    /// merged beside the person's own entries. `shell` runs the command on Windows.
    NestedMap {
        events: &'static [HookEntry],
        shell: HookShell,
    },
    /// The same events as a plugin Condr owns, `{"condr": {"<Event>": […]}}` beside its
    /// `plugin.json` manifest: an entry with a matcher is a group, one without a bare
    /// command.
    Plugin { events: &'static [HookEntry] },
    /// A flat list of command entries under `"version": 1`, merged beside the person's
    /// own. `exec` writes the executable and its arguments apart instead of a command line.
    FlatList {
        events: &'static [HookEntry],
        exec: bool,
    },
    /// A script Condr owns outright, from a template that may name
    /// `__CONDR_EXECUTABLE__`, `__CONDR_AGENT__` and `__CONDR_AGENT_MARKER__`. `registry`
    /// is the JSON file in the configuration directory whose `plugin` list must name it.
    Script {
        template: &'static str,
        registry: Option<&'static str>,
    },
}

/// What runs a nested map's command line on Windows, which decides whether a quoted path
/// needs PowerShell's call operator. The shells behind the other formats (Antigravity's
/// cmd, Cursor's) run a quoted path as it is.
#[derive(Debug)]
pub(super) enum HookShell {
    /// Git Bash or cmd.
    Plain,
    PowerShell,
    /// PowerShell, unless this environment variable selects Git Bash or cmd.
    PowerShellUnless(&'static str),
}

/// One native hook event and what it reports as.
#[derive(Debug)]
pub(super) struct HookEntry {
    pub(super) native: &'static str,
    pub(super) event: AgentEventKind,
    /// The native matcher, for events that would otherwise report things that do not
    /// block the user.
    pub(super) matcher: Option<&'static str>,
    /// Seconds the agent waits for the hook; it finishes in milliseconds.
    pub(super) timeout_secs: u64,
}

pub(super) const fn entry(native: &'static str, event: AgentEventKind) -> HookEntry {
    HookEntry {
        native,
        event,
        matcher: None,
        timeout_secs: HOOK_TIMEOUT_SECS,
    }
}

/// Seconds Claude Code and Codex wait for the hook by default; it finishes in milliseconds.
pub(super) const HOOK_TIMEOUT_SECS: u64 = 5;
const MAX_CONFIG_BYTES: u64 = 4 * 1024 * 1024;

/// Where and how hooks are installed on one machine.
#[derive(Clone, Debug)]
pub struct HookTarget {
    /// The home directory each CLI keeps its configuration under by default.
    pub home: PathBuf,
    /// Whether the environment variables that move a CLI's configuration apply, as they
    /// do on this machine; an isolated installation under `home` ignores them.
    pub env: bool,
    /// How a hook command names this `condr`: a bare name when PATH resolves it, otherwise
    /// the quoted absolute path.
    pub command: String,
}

impl HookTarget {
    /// Default locations under a home directory, also used for isolated installations.
    pub fn in_home(home: &Path, command: String) -> Self {
        Self {
            home: home.to_path_buf(),
            env: false,
            command,
        }
    }

    /// This machine and this executable.
    pub fn local() -> Option<Self> {
        let home = dirs::home_dir()?;
        let exe = std::env::current_exe().ok()?;
        Some(Self {
            env: true,
            ..Self::in_home(&home, command_name(&exe))
        })
    }

    /// An environment variable naming a directory, when it applies and is not empty.
    pub(super) fn var(&self, name: &str) -> Option<PathBuf> {
        self.env
            .then(|| std::env::var_os(name))
            .flatten()
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    }

    /// `$name`, else `relative` under the home directory.
    pub(super) fn env_dir(&self, name: &str, relative: &str) -> PathBuf {
        self.var(name).unwrap_or_else(|| self.home.join(relative))
    }

    /// The file Condr's hooks for `agent` live in; empty for an agent recognized only,
    /// which installs nothing.
    pub fn path(&self, agent: AgentKind) -> PathBuf {
        agent
            .spec()
            .hooks()
            .map_or_else(PathBuf::new, |hooks| (hooks.dir)(self).join(hooks.file))
    }

    /// The executable as a single shell word, for a runtime that does its own quoting.
    fn executable(&self) -> &str {
        self.command.trim_matches('"')
    }

    fn hook_command(&self, agent: AgentKind, event: AgentEventKind, shell: &HookShell) -> String {
        let choice = match shell {
            HookShell::PowerShellUnless(name) => std::env::var(name).ok(),
            _ => None,
        };
        self.hook_command_for_shell(agent, event, shell, cfg!(windows), choice.as_deref())
    }

    /// `choice` is the value of the variable a `PowerShellUnless` shell names.
    fn hook_command_for_shell(
        &self,
        agent: AgentKind,
        event: AgentEventKind,
        shell: &HookShell,
        windows: bool,
        choice: Option<&str>,
    ) -> String {
        // PowerShell needs the call operator before a quoted path.
        let powershell = match shell {
            HookShell::Plain => false,
            HookShell::PowerShell => true,
            HookShell::PowerShellUnless(_) => !matches!(
                choice
                    .map(|shell| shell.trim().to_ascii_lowercase())
                    .as_deref(),
                Some("bash" | "gitbash" | "git-bash" | "cmd" | "cmd.exe")
            ),
        };
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
    /// The agent is recognized only (ADR 0035): its hooks cannot report a state worth
    /// trusting, so none are installed.
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
/// `note` is standing advice for the agent, or why it is recognized only; `warning` is a
/// step the install could not finish on the user's behalf.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HooksReport {
    pub agent: AgentKind,
    pub path: PathBuf,
    pub state: HooksState,
    pub note: Option<String>,
    pub warning: Option<String>,
}

/// Performs `action` for `agent` on this machine and reports the resulting state. The
/// CLI runs it locally; the Server runs it for a GUI, whose hooks files live where the
/// agents run.
pub fn run(target: &HookTarget, agent: AgentKind, action: HooksAction) -> io::Result<HooksReport> {
    let mut warning = None;
    let path = match action {
        HooksAction::Install => {
            let path = install(target, agent)?;
            if let Some(enable) = agent.spec().hooks().and_then(|hooks| hooks.enable) {
                warning = enable().err();
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
        note: match &agent.spec().support {
            AgentSupport::Full(hooks) => hooks.note,
            AgentSupport::RecognitionOnly(reason) => Some(*reason),
        }
        .map(str::to_owned),
        warning,
    })
}

pub fn state(target: &HookTarget, agent: AgentKind) -> io::Result<HooksState> {
    let Some(hooks) = agent.spec().hooks() else {
        return Ok(HooksState::Unsupported);
    };
    let path = target.path(agent);
    match &hooks.format {
        HookFormat::NestedMap { .. } | HookFormat::FlatList { .. } => {
            map_state(target, agent, hooks, &path)
        }
        HookFormat::Plugin { .. } => {
            let Some(config) = read_config(&path)? else {
                return Ok(HooksState::Missing);
            };
            let manifest = read_config(&manifest_path(&path))?;
            Ok(
                if config == plugin(target, agent, hooks) && manifest == Some(plugin_manifest()) {
                    HooksState::Installed
                } else if config.to_string().contains(&marker(agent)) {
                    HooksState::Outdated
                } else {
                    HooksState::Missing
                },
            )
        }
        HookFormat::Script { template, registry } => {
            let registered = match registry {
                Some(registry) => registry_config(target, hooks, registry)?.1["plugin"]
                    .as_array()
                    .expect("checked plugin list")
                    .contains(&json!(registry_entry(hooks))),
                None => true,
            };
            Ok(match std::fs::read_to_string(&path) {
                Ok(contents) if registered && contents == script(target, agent, template) => {
                    HooksState::Installed
                }
                Ok(contents) if contents.contains(&marker(agent)) => HooksState::Outdated,
                Ok(_) => HooksState::Missing,
                Err(error) if error.kind() == io::ErrorKind::NotFound => HooksState::Missing,
                Err(error) => return Err(error),
            })
        }
    }
}

/// Writes this `condr`'s hooks, replacing any earlier Condr entries and leaving the
/// user's alone. Returns the file written.
pub fn install(target: &HookTarget, agent: AgentKind) -> io::Result<PathBuf> {
    let hooks = match &agent.spec().support {
        AgentSupport::Full(hooks) => hooks,
        AgentSupport::RecognitionOnly(reason) => {
            return Err(io::Error::new(io::ErrorKind::Unsupported, *reason));
        }
    };
    let path = target.path(agent);
    match &hooks.format {
        HookFormat::NestedMap { events, .. } | HookFormat::FlatList { events, .. } => {
            let mut root = read_config(&path)?.unwrap_or_else(|| json!({}));
            if matches!(hooks.format, HookFormat::FlatList { .. }) {
                if root.get("version").is_some_and(|version| version != 1) {
                    return Err(invalid(&path, "unsupported hooks configuration version"));
                }
                root["version"] = json!(1);
            }
            let map = hooks_map(&mut root)?;
            strip_ours(map, agent);
            for entry in *events {
                let items = map
                    .entry(entry.native)
                    .or_insert_with(|| Value::Array(Vec::new()));
                let Some(items) = items.as_array_mut() else {
                    return Err(invalid(
                        &path,
                        format!("hooks.{} is not an array", entry.native),
                    ));
                };
                items.push(map_item(target, agent, hooks, entry));
            }
            write_config(&path, &root)?;
        }
        HookFormat::Plugin { .. } => {
            let manifest = manifest_path(&path);
            check_owned(&path, &marker(agent))?;
            check_manifest(&manifest)?;
            write_config(&manifest, &plugin_manifest())?;
            write_config(&path, &plugin(target, agent, hooks))?;
        }
        HookFormat::Script { template, registry } => {
            check_owned(&path, &marker(agent))?;
            let registry = registry
                .map(|registry| registry_config(target, hooks, registry))
                .transpose()?;
            write_file(&path, script(target, agent, template).as_bytes())?;
            if let Some((registry_path, mut config)) = registry {
                let plugins = config["plugin"]
                    .as_array_mut()
                    .expect("checked plugin list");
                if !plugins.contains(&json!(registry_entry(hooks))) {
                    plugins.push(json!(registry_entry(hooks)));
                }
                write_config(&registry_path, &config)?;
            }
        }
    }
    Ok(path)
}

/// Removes every Condr entry, leaving the rest of the file as it was.
pub fn uninstall(target: &HookTarget, agent: AgentKind) -> io::Result<PathBuf> {
    let path = target.path(agent);
    let Some(hooks) = agent.spec().hooks() else {
        return Ok(path);
    };
    match &hooks.format {
        HookFormat::NestedMap { .. } | HookFormat::FlatList { .. } => {
            let Some(mut root) = read_config(&path)? else {
                return Ok(path);
            };
            if let Some(map) = root.get_mut("hooks").and_then(Value::as_object_mut) {
                strip_ours(map, agent);
                map.retain(|_, items| items.as_array().is_none_or(|items| !items.is_empty()));
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
        }
        HookFormat::Plugin { .. } => {
            let manifest = manifest_path(&path);
            check_owned(&path, &marker(agent))?;
            check_manifest(&manifest)?;
            // A disabled/enabled preference still belongs to the user, as do other files.
            for file in [&path, &manifest] {
                if file.exists() {
                    std::fs::remove_file(file)?;
                }
            }
        }
        HookFormat::Script { registry, .. } => {
            check_owned(&path, &marker(agent))?;
            let registry = registry
                .map(|registry| registry_config(target, hooks, registry))
                .transpose()?;
            if path.exists() {
                std::fs::remove_file(&path)?;
            }
            if let Some((registry_path, mut config)) = registry {
                config["plugin"]
                    .as_array_mut()
                    .expect("checked plugin list")
                    .retain(|plugin| plugin != registry_entry(hooks).as_str());
                if registry_path.exists() {
                    write_config(&registry_path, &config)?;
                }
            }
        }
    }
    Ok(path)
}

/// What marks a hook command or an owned file as Condr's, whichever `condr` wrote it.
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

/// What Condr installs for one entry of a nested map or flat list.
fn map_item(target: &HookTarget, agent: AgentKind, hooks: &HookSpec, entry: &HookEntry) -> Value {
    let mut item = match hooks.format {
        HookFormat::NestedMap { ref shell, .. } => json!({
            "hooks": [{
                "type": "command",
                "command": target.hook_command(agent, entry.event, shell),
                "timeout": entry.timeout_secs,
            }]
        }),
        HookFormat::FlatList { exec: true, .. } => json!({
            "type": "command",
            "exec": target.executable(),
            "args": ["agent-hook", agent.id(), entry.event.name()],
            "timeoutSec": entry.timeout_secs,
        }),
        HookFormat::FlatList { exec: false, .. } => json!({
            "command": target.hook_command(agent, entry.event, &HookShell::Plain),
            "timeout": entry.timeout_secs,
        }),
        HookFormat::Plugin { .. } | HookFormat::Script { .. } => {
            unreachable!("only a map holds items")
        }
    };
    if let Some(matcher) = entry.matcher {
        item["matcher"] = json!(matcher);
    }
    item
}

fn map_state(
    target: &HookTarget,
    agent: AgentKind,
    hooks: &HookSpec,
    path: &Path,
) -> io::Result<HooksState> {
    let (HookFormat::NestedMap { events, .. } | HookFormat::FlatList { events, .. }) = hooks.format
    else {
        unreachable!("only a map holds items");
    };
    let Some(root) = read_config(path)? else {
        return Ok(HooksState::Missing);
    };
    let mut found = Vec::new();
    if let Some(map) = root.get("hooks").and_then(Value::as_object) {
        for (native, items) in map {
            for item in items.as_array().into_iter().flatten() {
                let nested = item.get("hooks").and_then(Value::as_array);
                if is_ours(item, agent)
                    || nested.is_some_and(|nested| nested.iter().any(|hook| is_ours(hook, agent)))
                {
                    found.push((native.clone(), item.clone()));
                }
            }
        }
    }
    if found.is_empty() {
        return Ok(HooksState::Missing);
    }
    let mut expected: Vec<(String, Value)> = events
        .iter()
        .map(|entry| {
            (
                entry.native.to_owned(),
                map_item(target, agent, hooks, entry),
            )
        })
        .collect();
    expected.sort_by(|a, b| a.0.cmp(&b.0));
    found.sort_by(|a, b| a.0.cmp(&b.0));
    let version_ok = !matches!(hooks.format, HookFormat::FlatList { .. }) || root["version"] == 1;
    Ok(if found == expected && version_ok {
        HooksState::Installed
    } else {
        HooksState::Outdated
    })
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
fn strip_ours(map: &mut serde_json::Map<String, Value>, agent: AgentKind) {
    for items in map.values_mut() {
        let Some(items) = items.as_array_mut() else {
            continue;
        };
        for item in items.iter_mut() {
            if let Some(nested) = item.get_mut("hooks").and_then(Value::as_array_mut) {
                nested.retain(|hook| !is_ours(hook, agent));
            }
        }
        items.retain(|item| {
            !is_ours(item, agent)
                && item
                    .get("hooks")
                    .and_then(Value::as_array)
                    .is_none_or(|nested| !nested.is_empty())
        });
    }
}

fn plugin(target: &HookTarget, agent: AgentKind, hooks: &HookSpec) -> Value {
    let HookFormat::Plugin { events } = hooks.format else {
        unreachable!("only a plugin is built as one");
    };
    let mut map = serde_json::Map::new();
    for entry in events {
        let command = json!({
            "type": "command",
            "command": target.hook_command(agent, entry.event, &HookShell::Plain),
            "timeout": entry.timeout_secs,
        });
        let item = match entry.matcher {
            Some(matcher) => json!({"matcher": matcher, "hooks": [command]}),
            None => command,
        };
        map.entry(entry.native)
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .expect("built as an array")
            .push(item);
    }
    json!({ "condr": map })
}

fn plugin_manifest() -> Value {
    json!({"name": "condr", "description": "Condr agent status hooks (generated by condr)"})
}

fn manifest_path(hooks: &Path) -> PathBuf {
    hooks.with_file_name("plugin.json")
}

fn check_manifest(path: &Path) -> io::Result<()> {
    if read_config(path)?.is_some_and(|value| value != plugin_manifest()) {
        return Err(invalid(path, "exists and was not written by condr"));
    }
    Ok(())
}

fn script(target: &HookTarget, agent: AgentKind, template: &str) -> String {
    template
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

/// How the registry's `plugin` list names the script, beside it in the same directory.
fn registry_entry(hooks: &HookSpec) -> String {
    format!("./{}", hooks.file)
}

fn registry_config(
    target: &HookTarget,
    hooks: &HookSpec,
    registry: &str,
) -> io::Result<(PathBuf, Value)> {
    let path = (hooks.dir)(target).join(registry);
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

/// Someone else's file at our name is theirs to keep.
fn check_owned(path: &Path, marker: &str) -> io::Result<()> {
    match std::fs::read_to_string(path) {
        Ok(contents) if !contents.contains(marker) => {
            Err(invalid(path, "exists and was not written by condr"))
        }
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
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

    /// The shell a nested map's command runs under; every other format's is plain.
    fn shell(agent: AgentKind) -> &'static HookShell {
        match agent.spec().hooks() {
            Some(HookSpec {
                format: HookFormat::NestedMap { shell, .. },
                ..
            }) => shell,
            _ => &HookShell::Plain,
        }
    }

    #[test]
    fn quoted_hook_paths_follow_the_cli_shell() {
        let (target, _) = target();
        let command = |agent, windows, choice| {
            target.hook_command_for_shell(
                agent,
                AgentEventKind::Stop,
                shell(agent),
                windows,
                choice,
            )
        };
        for choice in [None, Some("pwsh"), Some("PowerShell"), Some("unknown")] {
            assert_eq!(
                command(AgentKind::Grok, true, choice),
                "& \"/opt/condr bin/condr\" agent-hook grok stop"
            );
            assert_eq!(
                command(AgentKind::Grok, false, choice),
                "\"/opt/condr bin/condr\" agent-hook grok stop"
            );
        }
        for choice in ["bash", "gitbash", "git-bash", "cmd", " CMD.EXE "] {
            assert_eq!(
                command(AgentKind::Grok, true, Some(choice)),
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
            target.hook_command_for_shell(
                AgentKind::Grok,
                AgentEventKind::Stop,
                shell(AgentKind::Grok),
                true,
                None
            ),
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
        let Some(HookSpec {
            format: HookFormat::NestedMap { events, .. },
            ..
        }) = AgentKind::Claude.spec().hooks()
        else {
            unreachable!("Claude's hooks are a nested map");
        };
        assert_eq!(
            once["hooks"].as_object().unwrap().len(),
            events
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
