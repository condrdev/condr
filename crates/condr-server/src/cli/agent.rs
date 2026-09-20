use super::*;

const DEFAULT_AGENT_WAIT_MS: u64 = 120_000;

#[derive(Subcommand)]
pub(crate) enum AgentCommand {
    /// List native agent CLIs available on the Server's PATH
    Available,
    /// List agents currently detected in Panes
    List,
    /// Start a named agent in an existing idle shell and wait until it is ready
    Start {
        name: String,
        #[arg(long)]
        kind: String,
        #[arg(long)]
        pane: u64,
        #[arg(long, default_value_t = 30_000)]
        timeout: u64,
        #[arg(last = true)]
        args: Vec<String>,
    },
    /// Send a prompt to an agent; optionally wait for a settled state
    Prompt {
        target: String,
        text: String,
        #[arg(long)]
        wait: bool,
        #[arg(long = "until", requires = "wait")]
        until: Vec<String>,
        #[arg(long, requires = "wait")]
        timeout: Option<u64>,
    },
    /// Wait for an agent to reach a state
    Wait {
        target: String,
        #[arg(long = "until")]
        until: Vec<String>,
        #[arg(long, default_value_t = DEFAULT_AGENT_WAIT_MS)]
        timeout: u64,
    },
    /// Install, inspect or remove the hooks that report an agent's status to Condr; edits
    /// that agent's own configuration on this machine
    Hooks {
        #[arg(value_enum)]
        action: HooksAction,
        /// Agent kind (see `condr agent available`)
        agent: String,
    },
}

#[derive(Clone, Copy, clap::ValueEnum)]
pub(crate) enum HooksAction {
    Install,
    Uninstall,
    Status,
}

pub(crate) fn run_agent(device: Option<&str>, command: AgentCommand) -> i32 {
    if let AgentCommand::Hooks { action, agent } = command {
        return match agent_hooks(action, &agent) {
            Ok(value) => {
                println!("{value}");
                0
            }
            Err(error) => {
                eprintln!("{}", json!({ "error": error }));
                1
            }
        };
    }
    run(device, |client| agent(client, command))
}

/// Local only: the hooks live in the agent's configuration on this machine, and the
/// Server is not involved until an agent runs them.
fn agent_hooks(action: HooksAction, agent: &str) -> Result<Value, CliError> {
    use condr_core::agent_hooks as hooks;
    let kind = AgentKind::parse_label(agent).ok_or_else(|| {
        CliError::new("unknown_agent_kind", format!("unknown agent kind {agent}"))
    })?;
    let target = hooks::HookTarget::local()
        .ok_or_else(|| CliError::new("no_home_directory", "cannot locate the home directory"))?;
    let action = match action {
        HooksAction::Install => hooks::HooksAction::Install,
        HooksAction::Uninstall => hooks::HooksAction::Uninstall,
        HooksAction::Status => hooks::HooksAction::Status,
    };
    let report = hooks::run(&target, kind, action).map_err(|error| {
        CliError::new(
            match error.kind() {
                io::ErrorKind::InvalidData => "hooks_config_invalid",
                _ => "io",
            },
            error.to_string(),
        )
    })?;
    serde_json::to_value(report).map_err(|error| CliError::new("io", error.to_string()))
}

fn agent(client: &mut ClientConnection, command: AgentCommand) -> Result<Value, CliError> {
    use condr_core::protocol::{AgentCommand as Request, AgentResponse};
    let request = match command {
        AgentCommand::Available => Request::Available,
        AgentCommand::List => Request::List,
        AgentCommand::Start {
            name,
            kind,
            pane,
            timeout,
            args,
        } => Request::Start {
            name,
            kind: AgentKind::parse_label(&kind).ok_or_else(|| {
                CliError::new("unknown_agent_kind", format!("unknown agent kind {kind}"))
            })?,
            pane_id: PaneId::from_u64(pane),
            args,
            timeout_ms: timeout,
        },
        AgentCommand::Prompt {
            target,
            text,
            wait,
            until,
            timeout,
        } => Request::Prompt {
            target,
            text,
            until: wait.then(|| parse_until(&until)).transpose()?,
            timeout_ms: timeout.unwrap_or(DEFAULT_AGENT_WAIT_MS),
        },
        AgentCommand::Wait {
            target,
            until,
            timeout,
        } => Request::Wait {
            target,
            until: parse_until(&until)?,
            timeout_ms: timeout,
        },
        AgentCommand::Hooks { .. } => unreachable!("handled locally by run_agent"),
    };
    let retry_until = std::time::Instant::now() + std::time::Duration::from_secs(2);
    let response = loop {
        let response = client.agent(request.clone())?;
        // A new shell can still be initializing. Retrying is safe only when the
        // Server explicitly rejected the launch before enqueueing any input.
        if matches!(request, Request::Start { .. })
            && response
                .as_ref()
                .is_err_and(|error| error.code == "pane_busy")
            && std::time::Instant::now() < retry_until
        {
            std::thread::sleep(std::time::Duration::from_millis(100));
            continue;
        }
        break response;
    }
    .map_err(|error| CliError {
        code: error.code,
        message: error.message,
    })?;
    match response {
        AgentResponse::Available(agents) => Ok(json!({
            "agents": agents.into_iter().map(|entry| json!({
                "agent": entry.kind.id(),
                "label": entry.kind.label(),
                "command": entry.kind.executable(),
                "executable": entry.executable,
            })).collect::<Vec<_>>()
        })),
        AgentResponse::List(agents) => Ok(json!({
            "agents": agents.into_iter().map(agent_info).collect::<Vec<_>>()
        })),
        AgentResponse::Ready(agent) => Ok(json!({ "agent": agent_info(agent) })),
        // The CLI never asks a Server for hooks; `agent hooks` runs them locally.
        AgentResponse::Hooks(report) => {
            serde_json::to_value(report).map_err(|error| CliError::new("io", error.to_string()))
        }
    }
}

pub(super) fn agent_info(info: condr_core::protocol::AgentInfo) -> Value {
    json!({
        "pane_id": info.pane_id.as_u64(),
        "name": info.name,
        "agent": info.agent.kind.id(),
        "agent_status": agent_state_name(info.agent.state),
        "session_id": info.agent.session_id,
        "launch_pending": info.launch_pending,
    })
}

pub(super) fn agent_state_name(state: AgentState) -> &'static str {
    match state {
        AgentState::Unknown => "unknown",
        AgentState::Idle => "idle",
        AgentState::Working => "working",
        AgentState::Blocked => "blocked",
    }
}

fn parse_until(values: &[String]) -> Result<Vec<AgentState>, CliError> {
    if values.is_empty() {
        return Ok(vec![AgentState::Idle, AgentState::Blocked]);
    }
    values
        .iter()
        .map(|value| match value.as_str() {
            "unknown" => Ok(AgentState::Unknown),
            "idle" => Ok(AgentState::Idle),
            "working" => Ok(AgentState::Working),
            "blocked" => Ok(AgentState::Blocked),
            _ => Err(CliError::new(
                "invalid_agent_state",
                format!("unknown agent state {value}"),
            )),
        })
        .collect()
}
