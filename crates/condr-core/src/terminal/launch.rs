use super::*;

#[derive(Clone)]
pub struct TerminalLaunchProbe {
    #[cfg(unix)]
    master: Option<Weak<Mutex<Box<dyn MasterPty + Send>>>>,
    process: ProcessProbe,
}

impl TerminalRuntime {
    pub fn launch_probe(&self) -> TerminalLaunchProbe {
        TerminalLaunchProbe {
            #[cfg(unix)]
            master: self.master.as_ref().map(Arc::downgrade),
            process: self.process,
        }
    }

    /// A fresh process check for orchestration; display detection may intentionally lag.
    pub fn running_agent(&self) -> Option<AgentKind> {
        process_snapshot().refreshed_at = None;
        #[cfg(unix)]
        let result = self.process.probe_agent(self.master.as_ref()?);
        #[cfg(windows)]
        let result = self.process.probe_agent();
        match result {
            ProcessProbeResult::Agent(kind) => Some(kind),
            _ => None,
        }
    }
}

impl TerminalLaunchProbe {
    /// OS inspection runs outside the Server lock. The caller validates the Terminal
    /// instance and launch reservation again before submitting the returned command.
    pub fn command(
        &self,
        installation: &crate::agent_discovery::AgentInstallation,
        args: &[String],
    ) -> io::Result<String> {
        let shell = self.idle_shell().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::WouldBlock,
                "Pane is busy or its shell is initializing",
            )
        })?;
        let program = installation.executable.to_str().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "agent executable path is not UTF-8",
            )
        })?;
        let command = shell_command(&shell, program, args)?;
        // ponytail: a shell without a line editor may have a 4096-byte canonical
        // input buffer. Longer launches need a dedicated shell submission protocol.
        if command.len() > 4094 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "encoded agent command exceeds 4094 bytes",
            ));
        }
        Ok(command)
    }

    fn idle_shell(&self) -> Option<String> {
        let shell_pid = Pid::from_u32(self.process.shell_pid?);
        let mut system = System::new();
        system.refresh_processes_specifics(ProcessesToUpdate::All, ProcessRefreshKind::new());
        let shell = system.process(shell_pid)?;
        if Some(shell.start_time()) != self.process.shell_started_at {
            return None;
        }
        #[cfg(unix)]
        {
            let group = self
                .master
                .as_ref()?
                .upgrade()?
                .lock()
                .ok()?
                .process_group_leader()?;
            if u32::try_from(group).ok() != Some(shell_pid.as_u32())
                || system.processes().keys().any(|pid| {
                    *pid != shell_pid
                        && i32::try_from(pid.as_u32())
                            .ok()
                            .and_then(|pid| getpgid(Some(UnixPid::from_raw(pid))).ok())
                            .is_some_and(|pid| pid.as_raw() == group)
                })
            {
                return None;
            }
        }
        #[cfg(windows)]
        if system
            .processes()
            .keys()
            .any(|pid| *pid != shell_pid && descendant_depth(&system, *pid, shell_pid).is_some())
        {
            return None;
        }
        let name = shell.name().to_str()?.to_ascii_lowercase();
        let name = name.trim_start_matches('-').trim_end_matches(".exe");
        matches!(
            name,
            "sh" | "bash" | "dash" | "zsh" | "ksh" | "mksh" | "fish" | "pwsh" | "powershell"
        )
        .then(|| name.to_owned())
    }
}

fn shell_command(shell: &str, program: &str, args: &[String]) -> io::Result<String> {
    if std::iter::once(program)
        .chain(args.iter().map(String::as_str))
        .any(|arg| arg.chars().any(char::is_control))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "agent arguments cannot contain control characters",
        ));
    }
    let powershell = matches!(shell, "pwsh" | "powershell");
    if powershell && cfg!(windows) {
        return windows_command(program, args);
    }
    let quote = |value: &str| {
        if powershell {
            powershell_quote(value)
        } else if shell == "fish" {
            format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'"))
        } else {
            format!("'{}'", value.replace('\'', "'\\''"))
        }
    };
    Ok(format!(
        "{}{}",
        if powershell { "& " } else { "" },
        std::iter::once(program)
            .chain(args.iter().map(String::as_str))
            .map(quote)
            .collect::<Vec<_>>()
            .join(" ")
    ))
}

fn powershell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

// Herdr's Start-Process approach preserves argv in Windows PowerShell's legacy
// native argument mode. Reference: src/platform/windows.rs at 5158adab (Apache-2.0).
fn windows_command(program: &str, args: &[String]) -> io::Result<String> {
    let extension = Path::new(program)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    if extension.eq_ignore_ascii_case("ps1") {
        return Ok(format!(
            "& {} {}",
            powershell_quote(program),
            args.iter()
                .map(|arg| powershell_quote(arg))
                .collect::<Vec<_>>()
                .join(" ")
        ));
    }
    if ["cmd", "bat"]
        .iter()
        .any(|ext| extension.eq_ignore_ascii_case(ext))
        && args
            .iter()
            .any(|arg| arg.contains(['&', '|', '<', '>', '^', '(', ')', '%', '!', '"']))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "batch agent shims cannot preserve shell metacharacters; use an .exe or .ps1 installation",
        ));
    }
    let arguments = args
        .iter()
        .map(|arg| windows_quote(arg))
        .collect::<Vec<_>>()
        .join(" ");
    let argument_list = if args.is_empty() {
        String::new()
    } else {
        format!(" -ArgumentList {}", powershell_quote(&arguments))
    };
    Ok(format!(
        "Start-Process -FilePath {}{argument_list} -NoNewWindow -Wait",
        powershell_quote(program)
    ))
}

fn windows_quote(value: &str) -> String {
    let mut quoted = String::from("\"");
    let mut backslashes = 0;
    for ch in value.chars() {
        if ch == '\\' {
            backslashes += 1;
            continue;
        }
        quoted.push_str(&"\\".repeat(if ch == '"' {
            backslashes * 2 + 1
        } else {
            backslashes
        }));
        quoted.push(ch);
        backslashes = 0;
    }
    quoted.push_str(&"\\".repeat(backslashes * 2));
    quoted.push('"');
    quoted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_arguments_are_literals_and_controls_are_rejected() {
        #[cfg(unix)]
        {
            let args = vec![
                "".into(),
                "one two".into(),
                "a'b\"c".into(),
                "$(touch /unwanted); $HOME".into(),
                "end\\".into(),
            ];
            let script = shell_command(
                "sh",
                "printf",
                &[vec!["%s\\n".into()], args.clone()].concat(),
            )
            .unwrap();
            let output = std::process::Command::new("sh")
                .args(["-c", &script])
                .output()
                .unwrap();
            assert!(output.status.success());
            assert_eq!(
                String::from_utf8(output.stdout).unwrap(),
                format!("{}\n", args.join("\n"))
            );
        }
        assert!(shell_command("sh", "codex", &["a\nb".into()]).is_err());
        assert!(shell_command("pwsh", "codex", &["\x1b[201~".into()]).is_err());
        assert_eq!(windows_quote("a\"b\\"), "\"a\\\"b\\\\\"");
        assert_eq!(windows_quote(""), "\"\"");
        assert!(windows_command("codex.cmd", &["%PATH%".into()]).is_err());
        assert_eq!(
            windows_command("codex.ps1", &["a'b".into()]).unwrap(),
            "& 'codex.ps1' 'a''b'"
        );
    }
}
