use super::*;

/// What the process table knows about one candidate.
#[derive(Clone, Copy, Debug)]
pub struct ProcessInfo<'a> {
    pub name: &'a str,
    pub argv: Option<&'a [String]>,
}

/// The agent a single process is, if any. A process named after the agent (or whose
/// argv[0] is) matches directly; a runtime or shell (node, bun, python, sh, cmd, pwsh)
/// is looked through to the script or command it runs.
pub fn identify_agent_process(process: ProcessInfo<'_>) -> Option<AgentKind> {
    let argv0 = process
        .argv
        .and_then(|argv| argv.first())
        .map(String::as_str);
    if let Some(agent) =
        AgentKind::parse_label(process.name).or_else(|| argv0.and_then(AgentKind::parse_label))
    {
        return Some(agent);
    }
    let argv = process.argv?;
    if !is_runtime_or_shell(argv0.unwrap_or(process.name)) {
        return None;
    }
    wrapped_command(argv).and_then(agent_from_path_token)
}

/// The best agent among several processes of one job: a process named after the agent
/// beats one only found through a wrapper.
pub fn identify_agent_among<'a>(
    processes: impl IntoIterator<Item = ProcessInfo<'a>>,
) -> Option<AgentKind> {
    let mut wrapped = None;
    for process in processes {
        let Some(agent) = identify_agent_process(process) else {
            continue;
        };
        if AgentKind::parse_label(process.name).is_some() {
            return Some(agent);
        }
        wrapped.get_or_insert(agent);
    }
    wrapped
}

/// The first thing a runtime or shell was asked to run: its script argument, or the
/// first token of a `-c` / `/c` / `-Command` string. Flags are skipped; an inline
/// program (`-e`, `-m`) is nothing Condr can name.
fn wrapped_command(argv: &[String]) -> Option<&str> {
    let mut args = argv.iter().skip(1).map(String::as_str);
    while let Some(arg) = args.next() {
        let flag = arg.trim_matches('"').to_ascii_lowercase();
        match flag.as_str() {
            "-c" | "/c" | "/k" | "-command" | "/command" => {
                return args.next().and_then(|command| {
                    command
                        .trim_start()
                        .trim_start_matches(['&', '.'])
                        .split_whitespace()
                        .next()
                });
            }
            "-e" | "--eval" | "-p" | "--print" | "-m" | "-encodedcommand" | "-enc" => {
                return None;
            }
            "-file" | "-f" => return args.next(),
            "--" => return args.next(),
            // A `cmd`/`pwsh` switch, not an absolute Unix path.
            _ if flag.starts_with('-') || (flag.starts_with('/') && !flag[1..].contains('/')) => {}
            _ => return Some(arg),
        }
    }
    None
}

fn agent_from_path_token(token: &str) -> Option<AgentKind> {
    let path = token.trim_matches(['"', '\'']);
    if path.is_empty() {
        return None;
    }
    if let Some(agent) = AgentKind::parse_label(path) {
        return Some(agent);
    }
    let lower = path.replace('\\', "/").to_ascii_lowercase();
    if let Some(agent) = AgentKind::ALL.into_iter().find(|agent| {
        agent
            .package_paths()
            .iter()
            .any(|package| lower.contains(&format!("/node_modules/{package}/")))
    }) {
        return Some(agent);
    }
    // A launcher (a symlink in `~/.local/bin`, say) may resolve to the real agent.
    let resolved = std::fs::canonicalize(path).ok()?;
    AgentKind::parse_label(resolved.file_name()?.to_str()?)
}

fn is_runtime_or_shell(name: &str) -> bool {
    let normalized = normalized_lookup_name(path_basename(name));
    normalized.starts_with("python")
        || matches!(normalized.as_str(), "node" | "bun")
        || is_shell(name)
}

/// A shell wrapping a command, as opposed to a runtime that *is* the agent's process.
pub(crate) fn is_shell(name: &str) -> bool {
    crate::terminal::KNOWN_SHELLS.contains(&normalized_lookup_name(path_basename(name)).as_str())
}

pub(super) fn normalized_lookup_name(name: &str) -> String {
    let mut name = name.trim().to_lowercase();
    for suffix in [".exe", ".cmd", ".bat", ".ps1", ".js"] {
        if name.ends_with(suffix) {
            name.truncate(name.len() - suffix.len());
            break;
        }
    }
    name
}

pub(super) fn path_basename(path: &str) -> &str {
    path.rsplit(['/', '\\'])
        .find(|component| !component.is_empty())
        .unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shells_are_told_apart_from_runtimes() {
        assert!(is_shell("/bin/bash"));
        assert!(is_shell("pwsh.exe"));
        assert!(is_shell("nu"));
        assert!(is_shell(r"C:\Windows\System32\cmd.exe"));
        assert!(!is_shell("node"));
        assert!(!is_shell("claude"));
    }
}
