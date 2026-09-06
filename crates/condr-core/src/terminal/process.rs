use super::*;

#[derive(Clone, Copy)]
pub(super) struct ProcessProbe {
    pub(super) shell_pid: Option<u32>,
    pub(super) shell_started_at: Option<u64>,
    #[cfg(unix)]
    pub(super) session_id: Option<i32>,
}

pub(super) struct ProcessSnapshot {
    system: System,
    pub(super) refreshed_at: Option<Instant>,
}

/// Keep partial refreshes out of the shared table: sysinfo 0.31 leaves their
/// `updated` flags set, retaining exited processes through the next full refresh.
fn command_line_processes(pids: &[Pid]) -> System {
    let mut system = System::new();
    if !pids.is_empty() {
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(pids),
            ProcessRefreshKind::new().with_cmd(UpdateKind::Always),
        );
    }
    system
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct OwnedProcess {
    pub(super) pid: Pid,
    pub(super) started_at: u64,
}

#[derive(Default)]
pub(super) struct ProcessShutdownState {
    pub(super) owned: Vec<OwnedProcess>,
    pub(super) status: Option<ExitStatus>,
    child_error: Option<io::Error>,
}

pub(super) fn attempt_process_tree_shutdown(
    process: ProcessProbe,
    child: &mut Option<&mut (dyn Child + Send + Sync)>,
    requires_child_status: bool,
    state: &mut ProcessShutdownState,
) -> io::Result<()> {
    poll_child_exit(child, &mut state.status, &mut state.child_error);
    refresh_owned_processes(process, &mut state.owned);
    if process_tree_exited(
        &state.owned,
        process.shell_pid,
        state.status.is_some(),
        requires_child_status,
    ) {
        return Ok(());
    }

    for signal in [Signal::Hangup, Signal::Term, Signal::Kill]
        .into_iter()
        .filter(|signal| sysinfo::SUPPORTED_SIGNALS.contains(signal))
    {
        let system = refresh_owned_processes(process, &mut state.owned);
        signal_processes(&system, &state.owned, signal);
        if wait_for_process_tree(
            process,
            &mut state.owned,
            child,
            &mut state.status,
            &mut state.child_error,
            requires_child_status,
            PROCESS_SHUTDOWN_GRACE,
        ) {
            return Ok(());
        }
    }

    if state.status.is_none() {
        if let Some(child) = child.as_deref_mut()
            && let Err(error) = child.kill()
        {
            record_process_error(&mut state.child_error, error);
        }
        if wait_for_process_tree(
            process,
            &mut state.owned,
            child,
            &mut state.status,
            &mut state.child_error,
            requires_child_status,
            PROCESS_SHUTDOWN_GRACE,
        ) {
            return Ok(());
        }
    }

    let message = state.child_error.as_ref().map_or_else(
        || "terminal process tree did not exit after forced shutdown".into(),
        |error| {
            format!(
                "terminal process tree did not exit after forced shutdown; child process operation failed: {error}"
            )
        },
    );
    Err(io::Error::new(io::ErrorKind::TimedOut, message))
}

pub(super) fn shutdown_process_tree(
    process: ProcessProbe,
    child: Option<&mut (dyn Child + Send + Sync)>,
) -> io::Result<Option<ExitStatus>> {
    let requires_child_status = child.is_some();
    let mut child = child;
    let mut state = ProcessShutdownState::default();
    attempt_process_tree_shutdown(process, &mut child, requires_child_status, &mut state)?;
    Ok(state.status)
}

pub(super) fn poll_child_exit(
    child: &mut Option<&mut (dyn Child + Send + Sync)>,
    status: &mut Option<ExitStatus>,
    child_error: &mut Option<io::Error>,
) {
    if status.is_some() {
        return;
    }
    let Some(child) = child.as_deref_mut() else {
        return;
    };
    match child.try_wait() {
        Ok(Some(child_status)) => {
            *status = Some(child_status);
            *child_error = None;
        }
        Ok(None) => {}
        Err(error) => record_process_error(child_error, error),
    }
}

pub(super) fn record_process_error(current: &mut Option<io::Error>, error: io::Error) {
    if current.is_none() {
        *current = Some(error);
    }
}

pub(super) fn wait_for_process_tree(
    process: ProcessProbe,
    owned: &mut Vec<OwnedProcess>,
    child: &mut Option<&mut (dyn Child + Send + Sync)>,
    status: &mut Option<ExitStatus>,
    child_error: &mut Option<io::Error>,
    requires_child_status: bool,
    timeout: Duration,
) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        poll_child_exit(child, status, child_error);
        refresh_owned_processes(process, owned);
        if process_tree_exited(
            owned,
            process.shell_pid,
            status.is_some(),
            requires_child_status,
        ) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(20));
    }
}

pub(super) fn refresh_owned_processes(
    process: ProcessProbe,
    owned: &mut Vec<OwnedProcess>,
) -> System {
    let system = System::new_all();
    let Some(shell_pid) = process.shell_pid.map(Pid::from_u32) else {
        return system;
    };
    let shell_identity_matches = system.process(shell_pid).is_some_and(|shell| {
        process
            .shell_started_at
            .is_some_and(|started_at| shell.start_time() == started_at)
    });
    for (pid, candidate) in system.processes() {
        #[cfg(unix)]
        let belongs = process.session_id.is_some_and(|session_id| {
            i32::try_from(pid.as_u32())
                .ok()
                .and_then(|pid| getsid(Some(UnixPid::from_raw(pid))).ok())
                .is_some_and(|pid| pid.as_raw() == session_id)
        }) || (process.session_id.is_none()
            && shell_identity_matches
            && descendant_depth(&system, *pid, shell_pid).is_some());
        #[cfg(not(unix))]
        let belongs =
            shell_identity_matches && descendant_depth(&system, *pid, shell_pid).is_some();
        let identity = OwnedProcess {
            pid: *pid,
            started_at: candidate.start_time(),
        };
        if belongs && !owned.contains(&identity) {
            owned.push(identity);
        }
    }
    owned.sort_unstable_by_key(|process| (process.pid != shell_pid, process.pid.as_u32()));
    system
}

pub(super) fn signal_processes(system: &System, owned: &[OwnedProcess], signal: Signal) {
    for owned_process in owned {
        let _ = system
            .process(owned_process.pid)
            .filter(|process| process.start_time() == owned_process.started_at)
            .and_then(|process| process.kill_with(signal));
    }
}

pub(super) fn process_tree_exited(
    owned: &[OwnedProcess],
    shell_pid: Option<u32>,
    child_reaped: bool,
    requires_child_status: bool,
) -> bool {
    if requires_child_status && !child_reaped {
        return false;
    }
    let system = System::new_all();
    owned.iter().all(|owned_process| {
        (child_reaped && Some(owned_process.pid.as_u32()) == shell_pid)
            || system
                .process(owned_process.pid)
                .is_none_or(|process| process.start_time() != owned_process.started_at)
    })
}

pub(super) fn resolve_initial_cwd(cwd: PathBuf) -> Option<PathBuf> {
    std::path::absolute(cwd)
        .ok()
        .and_then(existing_absolute_directory)
}

pub(super) fn existing_absolute_directory(cwd: PathBuf) -> Option<PathBuf> {
    (cwd.is_absolute() && cwd.is_dir()).then_some(cwd)
}

impl ProcessProbe {
    pub(super) fn new(shell_pid: Option<u32>) -> Self {
        let shell_started_at = shell_pid.and_then(|pid| {
            System::new_all()
                .process(Pid::from_u32(pid))
                .map(|process| process.start_time())
        });
        Self {
            shell_pid,
            shell_started_at,
            #[cfg(unix)]
            // portable-pty calls setsid(2) before exec, so the spawned child is
            // the session leader. Never adopt an arbitrary observed SID here:
            // the parent may briefly see its own session during spawn.
            session_id: shell_pid.and_then(|pid| i32::try_from(pid).ok()),
        }
    }

    pub(super) fn cwd(&self) -> Option<PathBuf> {
        process_cwd_with_identity(Pid::from_u32(self.shell_pid?), self.shell_started_at?)
    }

    #[cfg(unix)]
    pub(super) fn foreground_cwd(
        &self,
        master: &Mutex<Box<dyn MasterPty + Send>>,
    ) -> Option<PathBuf> {
        let foreground_group = master
            .lock()
            .expect("PTY master lock poisoned")
            .process_group_leader()
            .map(UnixPid::from_raw)?;
        let leader = u32::try_from(foreground_group.as_raw()).ok()?;
        let shell_cwd = self.cwd();
        let leader_cwd = process_cwd(Pid::from_u32(leader));
        if leader_cwd.as_ref() != shell_cwd.as_ref() && leader_cwd.is_some() {
            return leader_cwd;
        }
        let members = {
            let processes = process_snapshot();
            processes
                .system
                .processes()
                .keys()
                .filter_map(|pid| {
                    let raw_pid = i32::try_from(pid.as_u32()).ok()?;
                    (raw_pid != foreground_group.as_raw()
                        && getpgid(Some(UnixPid::from_raw(raw_pid))).ok()? == foreground_group)
                        .then_some(*pid)
                })
                .collect::<Vec<_>>()
        };
        members
            .into_iter()
            .filter_map(process_cwd)
            .find(|cwd| Some(cwd) != shell_cwd.as_ref())
            .or(leader_cwd)
    }

    /// The foreground process group of the PTY, or the shell's own group.
    #[cfg(unix)]
    pub(super) fn foreground_group(
        &self,
        master: &Mutex<Box<dyn MasterPty + Send>>,
    ) -> Option<UnixPid> {
        master
            .lock()
            .expect("PTY master lock poisoned")
            .process_group_leader()
            .map(UnixPid::from_raw)
            .or_else(|| {
                let shell_pid = i32::try_from(self.shell_pid?).ok()?;
                getpgid(Some(UnixPid::from_raw(shell_pid))).ok()
            })
    }

    /// Identifies the agent in the PTY's foreground job, the way herdr does: the group
    /// leader first, then the best-scoring member. The shell's own process takes part
    /// (`exec agent` replaces it in place); only a job that contains the shell and no
    /// agent means whatever ran here has exited.
    #[cfg(unix)]
    pub(super) fn probe_agent(
        &self,
        master: &Mutex<Box<dyn MasterPty + Send>>,
    ) -> ProcessProbeResult {
        let Some(foreground_group) = self.foreground_group(master) else {
            return ProcessProbeResult::Unidentified;
        };
        let processes = process_snapshot();
        // Phase one is the cheap table (no command lines); phase two reads command lines for
        // the foreground group only, leader first.
        let Some(leader) = u32::try_from(foreground_group.as_raw())
            .ok()
            .map(Pid::from_u32)
        else {
            return ProcessProbeResult::Unidentified;
        };
        let mut members = vec![leader];
        members.extend(processes.system.processes().keys().copied().filter(|pid| {
            *pid != leader
                && i32::try_from(pid.as_u32())
                    .ok()
                    .and_then(|pid| getpgid(Some(UnixPid::from_raw(pid))).ok())
                    == Some(foreground_group)
        }));
        drop(processes);
        let system = command_line_processes(&members);
        if let Some(agent) = system.process(leader).and_then(identify_process) {
            return ProcessProbeResult::Agent(agent);
        }
        let candidates = members
            .iter()
            .filter_map(|pid| system.process(*pid))
            .map(CandidateProcess::new)
            .collect::<Vec<_>>();
        if let Some(agent) =
            crate::agent::identify_agent_among(candidates.iter().map(CandidateProcess::info))
        {
            return ProcessProbeResult::Agent(agent);
        }
        let shell_in_job = self
            .shell_pid
            .map(Pid::from_u32)
            .is_some_and(|shell| members.contains(&shell));
        if shell_in_job {
            ProcessProbeResult::ShellOnly
        } else {
            ProcessProbeResult::Unidentified
        }
    }

    /// Identifies the agent among the shell's descendants. Windows has no foreground
    /// group, so the topmost identified process in the tree stands in for the leader;
    /// a shell with no descendants at all means the agent has exited.
    #[cfg(windows)]
    pub(super) fn probe_agent(&self) -> ProcessProbeResult {
        let (Some(shell_pid), Some(shell_started_at)) = (self.shell_pid, self.shell_started_at)
        else {
            return ProcessProbeResult::Unidentified;
        };
        let shell_pid = Pid::from_u32(shell_pid);
        let processes = process_snapshot();
        if processes
            .system
            .process(shell_pid)
            .is_none_or(|shell| shell.start_time() != shell_started_at)
        {
            return ProcessProbeResult::Unidentified;
        }
        // Phase one is the cheap table (names and parents); command lines, the expensive part
        // on Windows, are read only for the shell's descendants.
        let descendants = processes
            .system
            .processes()
            .keys()
            .copied()
            .filter(|pid| {
                *pid != shell_pid && descendant_depth(&processes.system, *pid, shell_pid).is_some()
            })
            .collect::<Vec<_>>();
        if descendants.is_empty() {
            return ProcessProbeResult::ShellOnly;
        }
        drop(processes);
        let system = command_line_processes(&descendants);
        let candidates = descendants
            .iter()
            .filter_map(|pid| {
                let process = system.process(*pid)?;
                identify_process(process).map(|kind| (*pid, kind))
            })
            .collect::<Vec<_>>();
        root_agent(&candidates, |ancestor, descendant| {
            descendant_depth(&system, descendant, ancestor).is_some()
        })
        .map_or(ProcessProbeResult::Unidentified, ProcessProbeResult::Agent)
    }
}

#[cfg(any(windows, test))]
pub(super) fn root_agent<T: Copy + Eq>(
    candidates: &[(T, AgentKind)],
    is_ancestor: impl Fn(T, T) -> bool,
) -> Option<AgentKind> {
    let mut roots = candidates.iter().filter(|(candidate, _)| {
        candidates
            .iter()
            .all(|(other, _)| is_ancestor(*candidate, *other))
    });
    let (_, kind) = roots.next()?;
    roots.next().is_none().then_some(*kind)
}

pub(super) fn process_snapshot() -> std::sync::MutexGuard<'static, ProcessSnapshot> {
    static SNAPSHOT: OnceLock<Mutex<ProcessSnapshot>> = OnceLock::new();
    let mut snapshot = SNAPSHOT
        .get_or_init(|| {
            Mutex::new(ProcessSnapshot {
                system: System::new(),
                refreshed_at: None,
            })
        })
        .lock()
        .expect("process snapshot lock poisoned");
    let now = Instant::now();
    if snapshot
        .refreshed_at
        .is_none_or(|refreshed| now.duration_since(refreshed) >= PROCESS_REFRESH_INTERVAL)
    {
        // Names, parents, and start times only; see `command_line_processes`.
        snapshot
            .system
            .refresh_processes_specifics(ProcessesToUpdate::All, ProcessRefreshKind::new());
        snapshot.refreshed_at = Some(now);
    }
    snapshot
}

#[cfg(unix)]
pub(super) fn process_cwd(pid: Pid) -> Option<PathBuf> {
    let mut system = System::new();
    let pids = [pid];
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&pids),
        ProcessRefreshKind::new().with_cwd(UpdateKind::Always),
    );
    system
        .process(pid)?
        .cwd()
        .map(Path::to_path_buf)
        .and_then(existing_absolute_directory)
}

pub(super) fn process_cwd_with_identity(pid: Pid, started_at: u64) -> Option<PathBuf> {
    let mut system = System::new();
    let pids = [pid];
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&pids),
        ProcessRefreshKind::new().with_cwd(UpdateKind::Always),
    );
    let process = system.process(pid)?;
    if process.start_time() != started_at {
        return None;
    }
    process
        .cwd()
        .map(Path::to_path_buf)
        .and_then(existing_absolute_directory)
}

/// A process table row in the shape the identifier reads.
pub(super) struct CandidateProcess {
    name: String,
    argv: Vec<String>,
}

impl CandidateProcess {
    fn new(process: &sysinfo::Process) -> Self {
        Self {
            name: process.name().to_string_lossy().into_owned(),
            argv: process
                .cmd()
                .iter()
                .map(|argument| argument.to_string_lossy().into_owned())
                .collect(),
        }
    }

    fn info(&self) -> ProcessInfo<'_> {
        ProcessInfo {
            name: &self.name,
            argv: (!self.argv.is_empty()).then_some(self.argv.as_slice()),
        }
    }
}

pub(super) fn identify_process(process: &sysinfo::Process) -> Option<AgentKind> {
    identify_agent_process(CandidateProcess::new(process).info())
}

pub(super) fn descendant_depth(system: &System, mut pid: Pid, ancestor: Pid) -> Option<usize> {
    for depth in 0..32 {
        if pid == ancestor {
            return Some(depth);
        }
        pid = system.process(pid)?.parent()?;
    }
    None
}
