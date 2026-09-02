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
    refresh_kind: ProcessRefreshKind,
    refreshed_at: Option<Instant>,
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

    #[cfg(unix)]
    pub(super) fn agent_kind(
        &self,
        master: &Mutex<Box<dyn MasterPty + Send>>,
    ) -> Option<AgentKind> {
        let foreground_group = master
            .lock()
            .expect("PTY master lock poisoned")
            .process_group_leader()
            .map(UnixPid::from_raw)
            .or_else(|| {
                let shell_pid = i32::try_from(self.shell_pid?).ok()?;
                getpgid(Some(UnixPid::from_raw(shell_pid))).ok()
            })?;
        let processes = process_snapshot();
        let system = &processes.system;

        let leader = Pid::from_u32(u32::try_from(foreground_group.as_raw()).ok()?);
        if let Some(kind) = system.process(leader).and_then(|process| {
            identify_process(process.name().to_string_lossy().as_ref(), process.cmd())
        }) {
            return Some(kind);
        }

        system.processes().iter().find_map(|(pid, process)| {
            let pid = i32::try_from(pid.as_u32()).ok()?;
            (getpgid(Some(UnixPid::from_raw(pid))).ok()? == foreground_group).then(|| {
                identify_process(process.name().to_string_lossy().as_ref(), process.cmd())
            })?
        })
    }

    #[cfg(windows)]
    pub(super) fn agent_kind(&self) -> Option<AgentKind> {
        let shell_pid = Pid::from_u32(self.shell_pid?);
        let processes = process_snapshot();
        let system = &processes.system;
        if system.process(shell_pid)?.start_time() != self.shell_started_at? {
            return None;
        }
        let candidates = system
            .processes()
            .iter()
            .filter_map(|(pid, process)| {
                descendant_depth(system, *pid, shell_pid).and_then(|_| {
                    identify_process(process.name().to_string_lossy().as_ref(), process.cmd())
                        .map(|kind| (*pid, kind))
                })
            })
            .collect::<Vec<_>>();
        root_agent(&candidates, |ancestor, descendant| {
            descendant_depth(system, descendant, ancestor).is_some()
        })
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
                // Identification reads name and cmd only; resolving every exe path is the
                // expensive part of a full refresh on Windows.
                refresh_kind: ProcessRefreshKind::new().with_cmd(UpdateKind::Always),
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
        let refresh_kind = snapshot.refresh_kind;
        snapshot
            .system
            .refresh_processes_specifics(ProcessesToUpdate::All, refresh_kind);
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

pub(super) fn identify_process(name: &str, argv: &[std::ffi::OsString]) -> Option<AgentKind> {
    let argv = argv
        .iter()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    identify_agent_process(name, &argv)
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
