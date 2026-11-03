use super::*;

/// How long a shell Condr gave integration has to announce itself before the process
/// probe decides alone (ADR 0059): a `/etc/zshenv` that sets `ZDOTDIR` skips Condr's.
const ANNOUNCE_GRACE: Duration = Duration::from_secs(2);
/// How long after its first prompt mark a line editor has to turn on bracketed paste,
/// which it does once it reads; the person's own prompt hooks still run after the mark.
const PASTE_GRACE: Duration = Duration::from_secs(1);

#[derive(Clone)]
pub struct TerminalLaunchProbe {
    #[cfg(unix)]
    master: Option<Weak<Mutex<Box<dyn MasterPty + Send>>>>,
    process: ProcessProbe,
    terminal: Weak<Mutex<Terminal>>,
    notices: SharedTerminalNotices,
    started: Instant,
    announces: bool,
    prompt_reports_cwd: bool,
}

/// Whether a line typed into the Pane now would reach its shell's prompt (ADR 0059).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ShellReadiness {
    /// The shell has not shown its first prompt yet; ask again.
    Starting,
    /// A program runs in the Pane, or nothing says its shell waits at a prompt.
    Busy,
    /// At its prompt: the shell's lower-cased name, to quote a line for.
    Ready(String),
}

impl TerminalRuntime {
    pub fn launch_probe(&self) -> TerminalLaunchProbe {
        TerminalLaunchProbe {
            #[cfg(unix)]
            master: self.master.as_ref().map(Arc::downgrade),
            process: self.process.clone(),
            terminal: Arc::downgrade(&self.terminal),
            notices: Arc::clone(&self.notices),
            started: self.started,
            announces: self.announces,
            prompt_reports_cwd: self.prompt_reports_cwd,
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
    /// Whether the Pane's shell waits at its prompt and is one Condr can type into.
    pub fn is_ready(&self) -> bool {
        matches!(self.readiness(), ShellReadiness::Ready(shell) if KNOWN_SHELLS.contains(&shell.as_str()))
    }

    /// A shell whose integration announced itself is ready at its first prompt mark, and
    /// a Windows PowerShell once its prompt first reports the directory. Anything else is
    /// left to the process probe, which takes a shell running only builtins, still in its
    /// startup files, for one at its prompt (ADR 0059).
    pub fn readiness(&self) -> ShellReadiness {
        let Ok(notices) = self.notices.lock() else {
            return ShellReadiness::Busy;
        };
        let (announced, first_prompt, first_cwd_report, running) = (
            notices.shell_announced,
            notices.first_prompt,
            notices.first_cwd_report,
            notices.command.is_some(),
        );
        drop(notices);
        if announced {
            let Some(prompt) = first_prompt else {
                return ShellReadiness::Starting;
            };
            if running {
                return ShellReadiness::Busy;
            }
            let bracketed = self
                .terminal
                .upgrade()
                .is_some_and(|terminal| lock_pane(&terminal).is_ok_and(|vt| vt.mode(2004)));
            if !bracketed && prompt.elapsed() < PASTE_GRACE {
                return ShellReadiness::Starting;
            }
            return self
                .current_shell()
                .map_or(ShellReadiness::Busy, ShellReadiness::Ready);
        }
        if self.prompt_reports_cwd && first_cwd_report.is_none() {
            return ShellReadiness::Starting;
        }
        if self.announces && self.started.elapsed() < ANNOUNCE_GRACE {
            return ShellReadiness::Starting;
        }
        self.idle_shell()
            .map_or(ShellReadiness::Busy, ShellReadiness::Ready)
    }

    /// OS inspection runs outside the Server lock. The caller validates the Terminal
    /// instance and launch reservation again before submitting the returned command.
    pub fn command(
        &self,
        installation: &crate::agent_discovery::AgentInstallation,
        args: &[String],
    ) -> io::Result<String> {
        let ShellReadiness::Ready(shell) = self.readiness() else {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "Pane is busy or its shell is initializing",
            ));
        };
        if !KNOWN_SHELLS.contains(&shell.as_str()) {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                format!(
                    "unsupported shell '{shell}'; set [server.terminal] shell to one of {}",
                    KNOWN_SHELLS.join(", ")
                ),
            ));
        }
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

    /// The shell's name from its process row alone, with no look at what else runs.
    fn current_shell(&self) -> Option<String> {
        let shell_pid = Pid::from_u32(self.process.shell_pid?);
        let mut system = System::new();
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[shell_pid]),
            ProcessRefreshKind::new(),
        );
        let shell = system.process(shell_pid)?;
        if Some(shell.start_time()) != self.process.shell_started_at {
            return None;
        }
        process_shell_name(shell)
    }

    fn idle_shell(&self) -> Option<String> {
        let shell_pid = Pid::from_u32(self.process.shell_pid?);
        let mut system = System::new();
        // Unix looks for other members of the foreground group among every process;
        // Windows asks the shell's Job, so it needs the shell's own row only.
        #[cfg(unix)]
        system.refresh_processes_specifics(ProcessesToUpdate::All, ProcessRefreshKind::new());
        #[cfg(windows)]
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[shell_pid]),
            ProcessRefreshKind::new(),
        );
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
        // Any other process in the Job keeps the shell busy (ADR 0030).
        #[cfg(windows)]
        if self
            .process
            .job
            .as_ref()?
            .process_ids()
            .ok()?
            .iter()
            .any(|pid| *pid != shell_pid.as_u32())
        {
            return None;
        }
        process_shell_name(shell)
    }
}

/// `git clone -- <url> <path>` as one line for `shell` (ADR 0059): POSIX or fish words, a
/// plain `& git …` in PowerShell, since the URL holds no quote that would need
/// `Start-Process`, and that line run through `-EncodedCommand` from cmd, which would
/// expand `%` inside its own quotes.
pub fn clone_command(shell: &str, url: &str, path: &Path) -> io::Result<String> {
    if !KNOWN_SHELLS.contains(&shell) {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!("Condr cannot type into '{shell}'"),
        ));
    }
    let path = path
        .to_str()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "the path is not UTF-8"))?;
    let args = ["clone", "--", url, path].map(String::from);
    let powershell = matches!(shell, "pwsh" | "powershell");
    let line = if cfg!(windows) && (powershell || shell == "cmd") {
        let script = format!(
            "& git {}",
            args.iter()
                .map(|arg| powershell_quote(arg))
                .collect::<Vec<_>>()
                .join(" ")
        );
        if powershell {
            script
        } else {
            encoded_powershell_command(&script)
        }
    } else {
        shell_command(shell, "git", &args)?
    };
    if line.len() > 4094 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the clone command exceeds 4094 bytes",
        ));
    }
    Ok(line)
}

/// The root process by lower-cased name, whatever it is; callers decide whether Condr
/// knows how to type a command into it.
fn process_shell_name(shell: &sysinfo::Process) -> Option<String> {
    let name = shell.name().to_str()?.to_ascii_lowercase();
    Some(
        name.trim_start_matches('-')
            .trim_end_matches(".exe")
            .to_owned(),
    )
}

/// The launch as one line typed at the idle shell's prompt. Words are quoted only when
/// they need it, in POSIX or PowerShell single quotes; on Windows the PowerShell forms
/// go through `windows_command` and a cmd.exe Pane runs the same script through
/// `powershell.exe -EncodedCommand`, which no cmd quoting can break.
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
    if cfg!(windows) && (powershell || shell == "cmd") {
        let script = windows_command(program, args)?;
        return Ok(if powershell {
            script
        } else {
            encoded_powershell_command(&script)
        });
    }
    // ponytail: every other shell gets the POSIX form. It holds for bare words and for
    // single-quoted words without an embedded `'` in nu, elvish and xonsh too; only an
    // argument containing `'` would need a per-shell escape there.
    let quote = match shell {
        _ if powershell => powershell_quote,
        "fish" => fish_quote,
        _ => posix_quote,
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

fn is_bare_word(value: &str, extra: &[char]) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || extra.contains(&ch))
}

fn posix_quote(value: &str) -> String {
    if is_bare_word(value, &['@', '%', '_', '+', '=', ':', ',', '.', '/', '-']) {
        return value.to_owned();
    }
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// fish reads `\\` and `\'` as escapes inside single quotes, so the POSIX form could end
/// the quote early there.
fn fish_quote(value: &str) -> String {
    if is_bare_word(value, &['@', '%', '_', '+', '=', ':', ',', '.', '/', '-']) {
        return value.to_owned();
    }
    format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'"))
}

/// A leading `-` is quoted so a `.ps1` cannot bind it as one of its own parameters.
/// PowerShell ends a single-quoted string at U+2018 to U+201B as well as at `'`, and
/// reads any of them doubled as the character itself.
fn powershell_quote(value: &str) -> String {
    if !value.starts_with('-') && is_bare_word(value, &['_', '-', '.', '/', ':', '+', '=']) {
        return value.to_owned();
    }
    let mut quoted = String::from("'");
    for ch in value.chars() {
        if matches!(ch, '\'' | '\u{2018}'..='\u{201B}') {
            quoted.push(ch);
        }
        quoted.push(ch);
    }
    quoted.push('\'');
    quoted
}

/// `powershell.exe -EncodedCommand` takes the script as base64 of UTF-16LE, so cmd.exe
/// never parses a character of it. Windows PowerShell 5.1 ships with every Windows.
fn encoded_powershell_command(script: &str) -> String {
    use base64::Engine as _;
    let utf16 = script
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect::<Vec<_>>();
    format!(
        "powershell.exe -NoLogo -NoProfile -EncodedCommand {}",
        base64::engine::general_purpose::STANDARD.encode(utf16)
    )
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
            "& codex.ps1 'a''b'"
        );
    }

    #[test]
    fn words_are_quoted_only_when_needed() {
        let args: Vec<String> = ["", "two words", "a'b", "$HOME", "semi;colon", "@options"]
            .map(String::from)
            .to_vec();
        assert_eq!(
            shell_command("bash", "pi", &args).unwrap(),
            "pi '' 'two words' 'a'\\''b' '$HOME' 'semi;colon' @options"
        );
        assert_eq!(
            shell_command("nu", "/opt/pi", &["--resume".into(), "abc-1".into()]).unwrap(),
            "/opt/pi --resume abc-1"
        );
        if !cfg!(windows) {
            assert_eq!(
                shell_command("pwsh", "pi", &args).unwrap(),
                "& pi '' 'two words' 'a''b' '$HOME' 'semi;colon' '@options'"
            );
        }
    }

    #[test]
    fn fish_and_powershell_quotes_cannot_be_closed_early() {
        // fish: `\'` and `\\` are escapes inside single quotes.
        assert_eq!(
            shell_command("fish", "pi", &[r"x\';id;\'".into(), r"end\".into()]).unwrap(),
            r"pi 'x\\\';id;\\\'' 'end\\'"
        );
        // PowerShell: U+2018 to U+201B close a single-quoted string too.
        if !cfg!(windows) {
            assert_eq!(
                shell_command(
                    "pwsh",
                    "pi",
                    &["x\u{2019};calc;\u{2018}a'\u{201A}\u{201B}".into()]
                )
                .unwrap(),
                "& pi 'x\u{2019}\u{2019};calc;\u{2018}\u{2018}a''\u{201A}\u{201A}\u{201B}\u{201B}'"
            );
        }
        assert_eq!(
            windows_command("codex.ps1", &["\u{2019};calc".into()]).unwrap(),
            "& codex.ps1 '\u{2019}\u{2019};calc'"
        );
    }

    #[test]
    fn a_clone_is_one_line_per_shell() {
        let path = Path::new("/home/me/src/my repo");
        assert_eq!(
            clone_command("zsh", "https://host/r.git", path).unwrap(),
            "git clone -- https://host/r.git '/home/me/src/my repo'"
        );
        assert_eq!(
            clone_command("fish", r"/srv/a\b", path).unwrap(),
            r"git clone -- '/srv/a\\b' '/home/me/src/my repo'"
        );
        if !cfg!(windows) {
            assert_eq!(
                clone_command("pwsh", "https://host/r.git", path).unwrap(),
                "& git clone '--' https://host/r.git '/home/me/src/my repo'"
            );
        }
        assert!(clone_command("tmux", "https://host/r.git", path).is_err());
        #[cfg(unix)]
        {
            let script = clone_command("sh", "/srv/x y", path).unwrap();
            let output = std::process::Command::new("sh")
                .args(["-c", &format!("git() {{ printf '%s|' \"$@\"; }}; {script}")])
                .output()
                .unwrap();
            assert_eq!(
                String::from_utf8(output.stdout).unwrap(),
                "clone|--|/srv/x y|/home/me/src/my repo|"
            );
        }
    }

    #[test]
    fn a_cmd_pane_runs_the_powershell_script_encoded() {
        use base64::Engine as _;
        let script = windows_command(r"C:\Tools\codex.exe", &["a b".into()]).unwrap();
        assert_eq!(
            script,
            r#"Start-Process -FilePath 'C:\Tools\codex.exe' -ArgumentList '"a b"' -NoNewWindow -Wait"#
        );
        let encoded = encoded_powershell_command(&script);
        let payload = encoded
            .strip_prefix("powershell.exe -NoLogo -NoProfile -EncodedCommand ")
            .unwrap();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(payload)
            .unwrap();
        let units = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes(*pair))
            .collect::<Vec<_>>();
        assert_eq!(String::from_utf16(&units).unwrap(), script);
        if cfg!(windows) {
            assert_eq!(
                shell_command("cmd", r"C:\Tools\codex.exe", &["a b".into()]).unwrap(),
                encoded
            );
        }
    }
}
