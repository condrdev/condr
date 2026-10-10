//! Installing Condr's hooks into an agent CLI's own configuration (ADR 0014).
//!
//! Every hook runs `condr agent-hook <agent> <event>`. Each fully supported agent's spec
//! names one of four formats (ADR 0035), and this module is the code they share. A JSON
//! hook map or flat list gets Condr's entries merged in beside the user's, recognizable by
//! the command they run, so they can be reported, refreshed and removed without touching
//! anything else. A plugin or a script is a file Condr owns outright.

use super::hook::HookInput;
use super::{AgentEventKind, AgentKind};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[cfg(feature = "runtime")]
mod install;

#[cfg(feature = "runtime")]
pub use install::{install, run, state, uninstall};

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
