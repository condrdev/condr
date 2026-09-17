use super::*;

#[test]
fn cleanup_errors_do_not_hide_the_primary_shutdown_failure() {
    let error = combine_cleanup_results::<()>(
        Err(io::Error::other("process cleanup failed")),
        Err(io::Error::other("I/O cleanup failed")),
        "stop terminal I/O",
    )
    .unwrap_err();

    let message = error.to_string();
    assert!(message.contains("process cleanup failed"), "{message}");
    assert!(message.contains("I/O cleanup failed"), "{message}");
}

#[cfg(unix)]
#[test]
fn unix_pty_probe_uses_the_child_as_the_expected_session_leader() {
    let probe = ProcessProbe::new(Some(12_345));
    assert_eq!(probe.session_id, Some(12_345));
}

#[test]
fn process_exit_checks_reject_a_reused_pid_identity() {
    let system = System::new_all();
    let pid = Pid::from_u32(std::process::id());
    let started_at = system.process(pid).unwrap().start_time();
    assert!(!process_tree_exited(
        &system,
        &[OwnedProcess { pid, started_at }],
        None,
        false,
        false
    ));
    assert!(process_tree_exited(
        &system,
        &[OwnedProcess {
            pid,
            started_at: started_at.saturating_add(1),
        }],
        None,
        false,
        false
    ));
}

#[test]
fn process_refresh_keeps_retained_identity_after_shell_disappears() {
    let system = System::new_all();
    let pid = Pid::from_u32(std::process::id());
    let identity = OwnedProcess {
        pid,
        started_at: system.process(pid).unwrap().start_time(),
    };
    let mut owned = vec![identity];

    let system = refresh_owned_processes(ProcessProbe::new(None), &mut owned);

    assert_eq!(owned, [identity]);
    assert!(!process_tree_exited(&system, &owned, None, false, false));
}

#[test]
fn cwd_probe_rejects_a_reused_shell_pid_identity() {
    let probe = ProcessProbe::new(Some(std::process::id()));
    let started_at = probe
        .shell_started_at
        .expect("the current process has a birth identity");
    assert!(probe.cwd().is_some());

    let reused = ProcessProbe {
        shell_started_at: Some(started_at.wrapping_add(1)),
        ..probe
    };
    assert_eq!(reused.cwd(), None);
}

#[test]
fn cwd_probe_keeps_the_last_successful_observation_when_sources_disappear() {
    let directory = std::env::temp_dir().join(format!(
        "condr-terminal-cwd-cache-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let initial = directory.join("initial");
    let observed = directory.join("observed");
    std::fs::create_dir_all(&initial).unwrap();
    std::fs::create_dir_all(&observed).unwrap();
    let reported = Arc::new(Mutex::new(ReportedCwd {
        cwd: Some(observed.clone()),
        generation: 7,
    }));
    let probe = TerminalCwdProbe {
        #[cfg(unix)]
        master: None,
        process: ProcessProbe::new(None),
        last_known_cwd: Arc::new(Mutex::new(Some(initial))),
        reported_cwd: Arc::clone(&reported),
    };

    assert_eq!(probe.observe(), (Some(observed.clone()), 7));
    reported.lock().unwrap().cwd = None;
    assert_eq!(probe.observe(), (Some(observed), 7));

    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn windows_agent_selection_requires_one_candidate_to_own_the_tree() {
    let is_ancestor = |ancestor, mut descendant| {
        while ancestor != descendant {
            descendant = match descendant {
                3 => 2,
                2 => 1,
                _ => return false,
            };
        }
        true
    };
    assert_eq!(
        root_agent(
            &[(2, AgentKind::Claude), (3, AgentKind::Codex)],
            is_ancestor
        ),
        Some(AgentKind::Claude)
    );
    assert_eq!(
        root_agent(
            &[(2, AgentKind::Claude), (4, AgentKind::Codex)],
            is_ancestor
        ),
        None
    );
}
