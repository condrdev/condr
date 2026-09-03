use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[cfg(target_os = "linux")]
use condr_core::{
    AgentKind, AgentState, TerminalModifiers, TerminalMouseButton, TerminalMouseEvent,
    TerminalMousePosition, TerminalMouseTracking, TerminalMouseWheel, TerminalPosition,
    TerminalScroll, TerminalSide, TerminalUpdate,
};
use condr_core::{CommandBuilder, TerminalCommand, TerminalRuntime, TerminalSize};
#[cfg(target_os = "windows")]
use sysinfo::{Pid, System};

#[test]
fn shell_spawn_rejects_a_missing_working_directory() {
    let missing = std::env::temp_dir().join(format!(
        "condr-missing-cwd-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    assert!(!missing.exists());

    let error = match TerminalRuntime::spawn_shell(&missing, TerminalSize::new(24, 80), None) {
        Ok(_) => panic!("a missing cwd must not start a shell elsewhere"),
        Err(error) => error,
    };
    assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn runtime_cwd_follows_a_real_shell_directory_change() {
    let root = std::env::temp_dir().join(format!(
        "condr-terminal-cwd-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let target = root.join("next");
    std::fs::create_dir_all(&target).unwrap();
    let expected_root = root.canonicalize().unwrap();
    let expected_target = target.canonicalize().unwrap();
    let mut runtime = TerminalRuntime::spawn_shell(&root, TerminalSize::new(8, 60), None).unwrap();
    let cwd_probe = runtime.cwd_probe();

    wait_for_cwd(|| cwd_probe.cwd(), &expected_root);
    #[cfg(target_os = "windows")]
    wait_for_terminal_output(&runtime);
    runtime
        .execute(TerminalCommand::Text("cd next\r".into()))
        .unwrap();
    wait_for_cwd(|| cwd_probe.cwd(), &expected_target);

    runtime.shutdown().unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn bash_shell_reports_the_final_cwd_before_normal_exit() {
    let default = CommandBuilder::new_default_prog();
    let shell = default.get_shell();
    if std::path::Path::new(&shell)
        .file_name()
        .and_then(|name| name.to_str())
        != Some("bash")
    {
        return;
    }

    let root = std::env::temp_dir().join(format!(
        "condr-terminal-exit-cwd-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let target = root.join("final");
    std::fs::create_dir_all(&target).unwrap();
    let expected_target = target.canonicalize().unwrap();
    let mut runtime = TerminalRuntime::spawn_shell(&root, TerminalSize::new(8, 60), None).unwrap();
    let cwd_probe = runtime.cwd_probe();
    runtime
        .execute(TerminalCommand::Text(format!(
            "cd '{}'; exit 7\r",
            target.to_string_lossy().replace('\'', "'\\''")
        )))
        .unwrap();

    let status = runtime.wait().unwrap();
    assert_eq!(status.exit_code(), 7);
    let (observed_cwd, reported_generation) = cwd_probe.observe();
    assert_eq!(observed_cwd.as_deref(), Some(expected_target.as_path()));
    assert!(reported_generation > 0);
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn runtime_cwd_uses_a_foreground_group_member_when_the_leader_matches_the_shell() {
    let root = std::env::temp_dir().join(format!(
        "condr-terminal-member-cwd-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let target = root.join("member");
    std::fs::create_dir_all(&target).unwrap();
    let expected_target = target.canonicalize().unwrap();
    let mut command = CommandBuilder::new("/bin/bash");
    command.args(["-c", "true | (cd member && sleep 30)"]);
    command.cwd(&root);
    let mut runtime = TerminalRuntime::spawn(command, TerminalSize::new(5, 20)).unwrap();

    wait_for_cwd(|| runtime.cwd_probe().cwd(), &expected_target);

    runtime.shutdown().unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn shell_round_trip_updates_vt_replies_and_resizes() {
    let cwd = std::env::temp_dir().canonicalize().unwrap();
    let mut command = CommandBuilder::new("/bin/sh");
    command.arg("-c");
    command.arg(
        r#"printf '\033[31mready \344\270\226\347\225\214 e\314\201\033[0m\r\n'
stty raw -echo
printf '\033[6n'
reply=$(dd bs=1 count=6 2>/dev/null | od -An -tx1 | tr -d ' \n')
printf '\033[14t'
area=$(dd bs=1 count=12 2>/dev/null | od -An -tx1 | tr -d ' \n')
stty sane
printf '\r\nreply=%s area=%s env=%s/%s\r\ncwd=%s\r\n' "$reply" "$area" "$TERM" "$COLORTERM" "$PWD"
IFS= read -r line
printf 'input=%s\r\n' "$line"
stty size"#,
    );
    command.cwd(&cwd);

    let mut runtime =
        TerminalRuntime::spawn(command, TerminalSize::new(24, 80).with_cell_size(8, 16)).unwrap();
    wait_for_text(&runtime, "reply=1b5b323b3152");
    wait_for_text(&runtime, "area=1b5b343b3338343b36343074");
    let initial_text = runtime.visible_text();
    assert!(
        initial_text.contains("ready \u{4e16}\u{754c} e\u{301}"),
        "{initial_text:?}"
    );
    assert!(
        initial_text.contains("env=xterm-256color/truecolor"),
        "{initial_text:?}"
    );
    assert!(
        initial_text.contains(&format!("cwd={}", cwd.display())),
        "{initial_text:?}"
    );

    runtime.request_resize(TerminalSize::new(40, 100)).unwrap();
    runtime.request_resize(TerminalSize::new(24, 80)).unwrap();
    runtime.write(b"hello\r".to_vec()).unwrap();
    wait_for_text(&runtime, "24 80");

    let status = runtime.wait().unwrap();
    assert!(status.success());
    assert!(runtime.wait().is_err());
    assert!(runtime.agent_probe().is_none());
    assert!(runtime.agent_snapshot(None).is_none());
    assert!(runtime.revision() > 0);
    let text = runtime.visible_text();
    assert!(text.contains("input=hello"), "{text:?}");
    assert!(text.contains("24 80"), "{text:?}");

    let mut command = CommandBuilder::new("/bin/sh");
    command.args(["-c", "sleep 30"]);
    let mut runtime = TerminalRuntime::spawn(command, TerminalSize::new(24, 80)).unwrap();
    assert!(!runtime.shutdown().unwrap().success());
    assert!(runtime.shutdown().is_err());
}

#[cfg(target_os = "linux")]
#[test]
fn terminal_modes_route_mouse_focus_and_alternate_scroll() {
    let mut command = CommandBuilder::new("/bin/sh");
    command.arg("-c");
    command.arg(
        r#"stty raw -echo
for i in $(seq 1 30); do printf 'history-%02d\r\n' "$i"; done
printf 'mouse-ready\r\n\033[?1003h\033[?1006h'
history_mouse=$(timeout 0.3 dd bs=1 count=64 2>/dev/null | od -An -tx1 | tr -d ' \n')
printf 'live-mouse-ready\r\n'
mouse_press=$(dd bs=1 count=9 2>/dev/null | od -An -tx1 | tr -d ' \n')
printf 'release-ready\r\n'
mouse_release=$(dd bs=1 count=9 2>/dev/null | od -An -tx1 | tr -d ' \n')
printf '\r\nfocus-ready\r\n\033[?1004h'
focus=$(dd bs=1 count=24 2>/dev/null | od -An -tx1 | tr -d ' \n')
printf '\033[?1003l\033[?1006l\033[?1004l\033[?1049h\033[?1007hshift-scroll-ready\r\n'
shift_scroll=$(timeout 0.3 dd bs=1 count=3 2>/dev/null | od -An -tx1 | tr -d ' \n')
printf 'scroll-ready\r\n'
scroll=$(dd bs=1 count=3 2>/dev/null | od -An -tx1 | tr -d ' \n')
printf '\033[?1007l\033[?1049l'
stty sane
printf '\r\nhistory-mouse=%s mouse=%s%s focus=%s shift-scroll=%s scroll=%s\r\n' "$history_mouse" "$mouse_press" "$mouse_release" "$focus" "$shift_scroll" "$scroll""#,
    );
    let mut runtime = TerminalRuntime::spawn(command, TerminalSize::new(8, 240)).unwrap();
    let position = TerminalMousePosition { row: 3, column: 7 };
    let history_position = TerminalMousePosition { row: 0, column: 7 };

    wait_for_text(&runtime, "mouse-ready");
    assert_eq!(runtime.view().mouse_tracking, TerminalMouseTracking::Motion);
    runtime.scroll(TerminalScroll::Bottom);
    runtime
        .execute(TerminalCommand::Mouse(TerminalMouseEvent::Wheel {
            direction: TerminalMouseWheel::Up,
            amount: 2,
            position,
            modifiers: TerminalModifiers {
                shift: true,
                ..TerminalModifiers::default()
            },
        }))
        .unwrap();
    assert!(runtime.view().display_offset > 0);
    for event in [
        TerminalMouseEvent::Button {
            button: TerminalMouseButton::Right,
            pressed: true,
            position: history_position,
            modifiers: TerminalModifiers::default(),
        },
        TerminalMouseEvent::Motion {
            button: None,
            position: history_position,
            modifiers: TerminalModifiers::default(),
        },
        TerminalMouseEvent::Wheel {
            direction: TerminalMouseWheel::Up,
            amount: 1,
            position: history_position,
            modifiers: TerminalModifiers::default(),
        },
    ] {
        runtime.execute(TerminalCommand::Mouse(event)).unwrap();
        assert!(runtime.view().display_offset > 0);
    }
    runtime.scroll(TerminalScroll::Bottom);
    wait_for_text(&runtime, "live-mouse-ready");

    runtime
        .execute(TerminalCommand::Mouse(TerminalMouseEvent::Wheel {
            direction: TerminalMouseWheel::Up,
            amount: 2,
            position,
            modifiers: TerminalModifiers {
                shift: true,
                ..TerminalModifiers::default()
            },
        }))
        .unwrap();
    let scrolled_offset = runtime.view().display_offset;
    assert!(scrolled_offset > 0);
    runtime
        .execute(TerminalCommand::Mouse(TerminalMouseEvent::Button {
            button: TerminalMouseButton::Right,
            pressed: true,
            position,
            modifiers: TerminalModifiers::default(),
        }))
        .unwrap();
    assert_eq!(runtime.view().display_offset, scrolled_offset);
    runtime.scroll(TerminalScroll::Bottom);
    wait_for_text(&runtime, "release-ready");
    runtime.scroll(TerminalScroll::Top);
    runtime
        .execute(TerminalCommand::Mouse(TerminalMouseEvent::Button {
            button: TerminalMouseButton::Right,
            pressed: false,
            position: history_position,
            modifiers: TerminalModifiers::default(),
        }))
        .unwrap();
    assert!(runtime.view().display_offset > 0);
    runtime.scroll(TerminalScroll::Bottom);

    wait_for_text(&runtime, "focus-ready");
    runtime
        .execute(TerminalCommand::Mouse(TerminalMouseEvent::Button {
            button: TerminalMouseButton::Right,
            pressed: true,
            position,
            modifiers: TerminalModifiers::default(),
        }))
        .unwrap();
    runtime.execute(TerminalCommand::Focus(false)).unwrap();
    runtime.execute(TerminalCommand::Focus(true)).unwrap();

    wait_for_text(&runtime, "shift-scroll-ready");
    runtime
        .execute(TerminalCommand::Mouse(TerminalMouseEvent::Wheel {
            direction: TerminalMouseWheel::Up,
            amount: 1,
            position,
            modifiers: TerminalModifiers {
                shift: true,
                ..TerminalModifiers::default()
            },
        }))
        .unwrap();
    wait_for_text(&runtime, "scroll-ready");
    runtime
        .execute(TerminalCommand::Mouse(TerminalMouseEvent::Wheel {
            direction: TerminalMouseWheel::Up,
            amount: 1,
            position,
            modifiers: TerminalModifiers::default(),
        }))
        .unwrap();

    wait_for_text(&runtime, "mouse=1b5b3c323b383b324d1b5b3c323b383b326d");
    let text = runtime.visible_text();
    assert!(text.contains("history-mouse= mouse="), "{text:?}");
    assert!(
        text.contains("focus=1b5b3c323b383b344d1b5b3c323b383b346d1b5b4f1b5b49"),
        "{text:?}"
    );
    assert!(text.contains("shift-scroll= scroll=1b5b41"), "{text:?}");
    assert!(text.contains("scroll=1b5b41"), "{text:?}");
    assert!(runtime.wait().unwrap().success());
}

#[cfg(target_os = "linux")]
#[test]
fn mouse_release_bypasses_user_input_backpressure() {
    let mut command = CommandBuilder::new("/bin/sh");
    command.args([
        "-c",
        "stty raw -echo; printf 'release-ready\\r\\n\\033[?1000h\\033[?1006h'; sleep 30",
    ]);
    let mut runtime = TerminalRuntime::spawn(command, TerminalSize::new(5, 40)).unwrap();
    wait_for_text(&runtime, "release-ready");
    assert_eq!(runtime.view().mouse_tracking, TerminalMouseTracking::Click);

    let position = TerminalMousePosition { row: 1, column: 1 };
    runtime
        .execute(TerminalCommand::Mouse(TerminalMouseEvent::Button {
            button: TerminalMouseButton::Left,
            pressed: true,
            position,
            modifiers: TerminalModifiers::default(),
        }))
        .unwrap();

    let payload = vec![b'x'; 1024 * 1024];
    loop {
        match runtime.write(payload.clone()) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(error) => panic!("unexpected terminal input failure: {error}"),
        }
    }
    runtime
        .execute(TerminalCommand::Mouse(TerminalMouseEvent::Button {
            button: TerminalMouseButton::Left,
            pressed: false,
            position,
            modifiers: TerminalModifiers::default(),
        }))
        .unwrap();

    runtime.shutdown().unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn close_cancels_a_reader_when_a_descendant_keeps_the_slave_open() {
    let mut command = CommandBuilder::new("/bin/sh");
    command.args(["-c", "trap '' HUP; sleep 2 & wait"]);
    let mut runtime = TerminalRuntime::spawn(command, TerminalSize::new(5, 20)).unwrap();

    let started = Instant::now();
    runtime.close().unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "Terminal close waited for a detached descendant"
    );
    runtime.close().unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn close_keeps_reading_until_a_delayed_hangup_tail_is_published() {
    let mut command = CommandBuilder::new("/bin/sh");
    command.args([
        "-c",
        r#"trap 'sleep 0.1; printf "shutdown-tail\r\n"; exit 0' HUP; printf 'ready\r\n'; while :; do sleep 5; done"#,
    ]);
    let mut runtime = TerminalRuntime::spawn(command, TerminalSize::new(5, 40)).unwrap();
    wait_for_text(&runtime, "ready");

    runtime.close().unwrap();

    assert!(
        runtime.visible_text().contains("shutdown-tail"),
        "shutdown output was not merged into the final Terminal view: {:?}",
        runtime.visible_text()
    );
}

#[cfg(target_os = "linux")]
#[test]
fn close_terminates_the_entire_pty_session() {
    let pid_file = std::env::temp_dir().join(format!(
        "condr-terminal-descendant-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let mut command = CommandBuilder::new("/bin/sh");
    command.args([
        "-c",
        &format!(
            "trap '' HUP TERM; /bin/sh -c 'trap \"\" HUP TERM; while :; do sleep 1; done' & printf '%s' \"$!\" > '{}'; wait",
            pid_file.display()
        ),
    ]);
    let mut runtime = TerminalRuntime::spawn(command, TerminalSize::new(5, 20)).unwrap();
    let descendant = wait_for_pid_file(&pid_file);
    assert!(std::path::Path::new(&format!("/proc/{descendant}")).exists());

    runtime.close().unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while std::path::Path::new(&format!("/proc/{descendant}")).exists() && Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        !std::path::Path::new(&format!("/proc/{descendant}")).exists(),
        "PTY descendant {descendant} survived Terminal close"
    );
    let _ = std::fs::remove_file(pid_file);
}

#[cfg(target_os = "linux")]
#[test]
fn closing_one_pty_session_does_not_signal_another() {
    let mut first_command = CommandBuilder::new("/bin/sh");
    first_command.args(["-c", "trap '' HUP TERM; sleep 30"]);
    let mut first = TerminalRuntime::spawn(first_command, TerminalSize::new(5, 20)).unwrap();

    let mut second_command = CommandBuilder::new("/bin/sh");
    second_command.args([
        "-c",
        "trap '' HUP TERM; IFS= read -r line; printf 'second=%s\\r\\n' \"$line\"; sleep 30",
    ]);
    let mut second = TerminalRuntime::spawn(second_command, TerminalSize::new(5, 40)).unwrap();

    first.close().unwrap();
    second.write(b"alive\r".to_vec()).unwrap();
    wait_for_text(&second, "second=alive");
    second.close().unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn close_cancels_a_blocked_terminal_writer() {
    let mut command = CommandBuilder::new("/bin/sh");
    command.args(["-c", "stty raw -echo; trap '' HUP; sleep 2"]);
    let mut runtime = TerminalRuntime::spawn(command, TerminalSize::new(5, 20)).unwrap();
    runtime.write(vec![b'x'; 4 * 1024 * 1024]).unwrap();
    std::thread::sleep(Duration::from_millis(50));

    let started = Instant::now();
    runtime.close().unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "Terminal close waited for a blocked PTY writer"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn blocked_terminal_input_applies_bounded_backpressure() {
    let mut command = CommandBuilder::new("/bin/sh");
    command.args(["-c", "stty raw -echo; trap '' HUP TERM; sleep 30"]);
    let mut runtime = TerminalRuntime::spawn(command, TerminalSize::new(5, 20)).unwrap();

    let mut accepted = 0;
    let error = loop {
        match runtime.write(vec![b'x'; 1024 * 1024]) {
            Ok(()) => accepted += 1,
            Err(error) => break error,
        }
        assert!(
            accepted <= 8,
            "Terminal accepted more than its input budget"
        );
    };
    assert_eq!(error.kind(), std::io::ErrorKind::WouldBlock);
    runtime.close().unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn final_resize_bypasses_pty_input_backpressure() {
    let initial_size = TerminalSize::new(5, 20);
    let mut command = CommandBuilder::new("/bin/sh");
    command.args([
        "-c",
        "stty raw -echo; sleep 2; dd of=/dev/null bs=65536 2>/dev/null",
    ]);
    let mut runtime = TerminalRuntime::spawn(command, initial_size).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    runtime.write(vec![b'x'; 4 * 1024 * 1024]).unwrap();
    std::thread::sleep(Duration::from_millis(50));
    let revision = runtime.revision();

    runtime.request_resize(TerminalSize::new(10, 40)).unwrap();
    let final_size = TerminalSize::new(12, 42);
    runtime.request_resize(final_size).unwrap();

    let deadline = Instant::now() + Duration::from_secs(1);
    while runtime.size() != final_size && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        runtime.revision() > revision,
        "the final resize must be applied while PTY input is blocked"
    );
    assert_eq!(runtime.size(), final_size);
    runtime.close().unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn view_scrollback_selection_and_final_update_follow_the_vt_state() {
    let mut command = CommandBuilder::new("/bin/sh");
    command.args([
        "-c",
        r#"printf '\033[31malpha \344\270\226\347\225\214 e\314\201\033[0m\r\n'; sleep 30"#,
    ]);
    let mut runtime = TerminalRuntime::spawn(command, TerminalSize::new(5, 20)).unwrap();
    wait_for_text(&runtime, "alpha");

    let view = runtime.view();
    assert_eq!(view.cells.len(), 100);
    assert_eq!(view.cell(0, 0).unwrap().text, "a");
    assert_ne!(
        view.cell(0, 0).unwrap().foreground,
        view.cell(0, 0).unwrap().background
    );
    let revision = runtime.revision();
    assert_eq!(
        runtime
            .execute(TerminalCommand::Copy {
                selection: condr_core::TerminalSelection {
                    start: TerminalPosition {
                        row: 0,
                        column: 0,
                        side: TerminalSide::Left,
                    },
                    end: TerminalPosition {
                        row: 0,
                        column: 4,
                        side: TerminalSide::Right,
                    },
                    display_offset: view.display_offset,
                },
            })
            .unwrap(),
        Some("alpha".into())
    );
    assert_eq!(runtime.revision(), revision);
    let _ = runtime.shutdown().unwrap();

    let mut command = CommandBuilder::new("/bin/sh");
    command.args([
        "-c",
        "i=1; while [ $i -le 30 ]; do printf 'line-%02d\\r\\n' $i; i=$((i+1)); done; sleep 1; printf 'tail\\r\\n'; sleep 30",
    ]);
    let mut runtime = TerminalRuntime::spawn(command, TerminalSize::new(5, 20)).unwrap();
    wait_for_text(&runtime, "line-30");
    runtime.scroll(TerminalScroll::Top);
    let before = runtime.view();
    assert!(before.display_offset > 0);
    wait_for_revision_after(&runtime, before.revision);
    let after = runtime.view();
    assert!(after.display_offset > 0);
    assert_eq!(visible_rows(&before), visible_rows(&after));
    runtime.scroll(TerminalScroll::Bottom);
    wait_for_text(&runtime, "tail");
    let _ = runtime.shutdown().unwrap();

    let mut command = CommandBuilder::new("/bin/sh");
    command.args(["-c", "printf final"]);
    let mut runtime = TerminalRuntime::spawn(command, TerminalSize::new(5, 20)).unwrap();
    let updates = runtime.take_updates().unwrap();
    let mut saw_view = false;
    while let TerminalUpdate::View(_) = updates.recv_timeout(Duration::from_secs(5)).unwrap() {
        saw_view = true;
    }
    assert!(saw_view);
    assert!(runtime.wait().unwrap().success());
    assert!(runtime.visible_text().contains("final"));
}

#[cfg(target_os = "linux")]
#[test]
fn detection_text_uses_the_live_bottom_while_the_viewport_is_scrolled() {
    let mut command = CommandBuilder::new("/bin/sh");
    command.args([
        "-c",
        "for i in $(seq 1 40); do printf 'line-%02d\\n' \"$i\"; done; sleep 1",
    ]);
    let mut runtime = TerminalRuntime::spawn(command, TerminalSize::new(5, 40)).unwrap();
    wait_for_text(&runtime, "line-40");
    let bottom = runtime.bottom_text();
    assert!(bottom.contains("line-40"));

    runtime.scroll(TerminalScroll::Top);
    assert!(!runtime.visible_text().contains("line-40"));
    assert_eq!(runtime.bottom_text(), bottom);
    runtime.shutdown().unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn foreground_agent_process_and_terminal_text_produce_a_snapshot() {
    let mut command = CommandBuilder::new("/bin/bash");
    command.args([
        "-c",
        "printf '◦ Working (1s - esc to interrupt)\\n'; bash -c 'exec -a codex sleep 30' & wait",
    ]);
    let mut runtime = TerminalRuntime::spawn(command, TerminalSize::new(5, 50)).unwrap();
    wait_for_text(&runtime, "esc to interrupt");

    let deadline = Instant::now() + Duration::from_secs(5);
    let snapshot = loop {
        if let Some(snapshot) = runtime.agent_snapshot(None) {
            break snapshot;
        }
        assert!(Instant::now() < deadline, "agent process was not detected");
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(snapshot.kind, AgentKind::Codex);
    assert_eq!(snapshot.state, AgentState::Working);
    runtime.shutdown().unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn foreground_agent_survives_its_process_group_leader_exiting() {
    let mut command = CommandBuilder::new("/bin/bash");
    command.args(["--noprofile", "--norc", "-i"]);
    let mut runtime = TerminalRuntime::spawn(command, TerminalSize::new(8, 60)).unwrap();
    runtime
        .write(
            "printf '◦ Working (1s - esc to interrupt)\\n'; true | bash -c 'exec -a codex sleep 30'\r"
                .as_bytes()
                .to_vec(),
        )
        .unwrap();
    wait_for_text(&runtime, "esc to interrupt");

    let deadline = Instant::now() + Duration::from_secs(5);
    let snapshot = loop {
        if let Some(snapshot) = runtime.agent_snapshot(None) {
            break snapshot;
        }
        assert!(
            Instant::now() < deadline,
            "agent process was not detected after the pipeline leader exited"
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(snapshot.kind, AgentKind::Codex);
    assert_eq!(snapshot.state, AgentState::Working);

    runtime.write(vec![3]).unwrap();
    std::thread::sleep(Duration::from_millis(50));
    runtime.shutdown().unwrap();
}

#[cfg(target_os = "windows")]
#[test]
fn conpty_round_trip_resizes_unicode_and_eof() {
    let mut command = CommandBuilder::new("pwsh.exe");
    command.args(["-NoLogo", "-NoProfile"]);
    command.cwd(std::env::temp_dir());
    let mut runtime =
        TerminalRuntime::spawn(command, TerminalSize::new(24, 80)).expect("spawn pwsh in ConPTY");

    runtime
        .execute(TerminalCommand::Text(
            "Write-Output 'ready \u{4e16}\u{754c}'\r".into(),
        ))
        .unwrap();
    wait_for_text(&runtime, "ready \u{4e16}\u{754c}");

    runtime.request_resize(TerminalSize::new(40, 100)).unwrap();
    runtime.request_resize(TerminalSize::new(24, 80)).unwrap();
    runtime
        .execute(TerminalCommand::Text(
            "$s=$Host.UI.RawUI.WindowSize; Write-Output \"size=$($s.Height)x$($s.Width)\"\r".into(),
        ))
        .unwrap();
    wait_for_text(&runtime, "size=24x80");

    runtime
        .execute(TerminalCommand::Text(
            "Write-Output 'final-before-exit'; exit\r".into(),
        ))
        .unwrap();
    wait_for_text(&runtime, "final-before-exit");
    assert!(runtime.wait().unwrap().success());
    assert!(runtime.agent_probe().is_none());
    assert!(runtime.agent_snapshot(None).is_none());
    assert!(runtime.visible_text().contains("final-before-exit"));
}

#[cfg(target_os = "windows")]
#[test]
fn conpty_close_terminates_descendant_processes() {
    let pid_file = std::env::temp_dir().join(format!(
        "condr-conpty-descendant-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let escaped_pid_file = pid_file.display().to_string().replace('\'', "''");
    let script = format!(
        "$child = Start-Process pwsh.exe -WindowStyle Hidden -ArgumentList @('-NoLogo','-NoProfile','-Command','Start-Sleep -Seconds 30') -PassThru; Set-Content -NoNewline -LiteralPath '{escaped_pid_file}' -Value $child.Id; Start-Sleep -Seconds 30"
    );
    let mut command = CommandBuilder::new("pwsh.exe");
    command.args(["-NoLogo", "-NoProfile", "-Command", &script]);
    let mut runtime = TerminalRuntime::spawn(command, TerminalSize::new(5, 40)).unwrap();
    let descendant = wait_for_pid_file(&pid_file);
    assert!(windows_process_exists(descendant));

    runtime.close().unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while windows_process_exists(descendant) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        !windows_process_exists(descendant),
        "ConPTY descendant {descendant} survived Terminal close"
    );
    let _ = std::fs::remove_file(pid_file);
}

fn wait_for_text(runtime: &TerminalRuntime, needle: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if runtime.visible_text().contains(needle) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!(
        "terminal never displayed {needle:?}: {:?}",
        runtime.visible_text()
    );
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn wait_for_pid_file(path: &std::path::Path) -> u32 {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Ok(contents) = std::fs::read_to_string(path)
            && let Ok(pid) = contents.parse()
        {
            return pid;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("Terminal never reported its descendant PID");
}

#[cfg(target_os = "windows")]
fn windows_process_exists(pid: u32) -> bool {
    System::new_all().process(Pid::from_u32(pid)).is_some()
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn wait_for_cwd(cwd: impl Fn() -> Option<std::path::PathBuf>, expected: &std::path::Path) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while Instant::now() < deadline {
        if cwd().and_then(|cwd| cwd.canonicalize().ok()).as_deref() == Some(expected) {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!(
        "terminal cwd never became {}: {:?}",
        expected.display(),
        cwd()
    );
}

#[cfg(target_os = "windows")]
fn wait_for_terminal_output(runtime: &TerminalRuntime) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while Instant::now() < deadline {
        if runtime.revision() > 0 && !runtime.visible_text().is_empty() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("PowerShell did not publish its initial prompt");
}

#[cfg(target_os = "linux")]
fn wait_for_revision_after(runtime: &TerminalRuntime, revision: u64) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if runtime.revision() > revision {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("terminal revision never advanced past {revision}");
}

#[cfg(target_os = "linux")]
fn visible_rows(view: &condr_core::TerminalView) -> Vec<String> {
    (0..view.size.rows)
        .map(|row| {
            (0..view.size.columns)
                .filter_map(|column| view.cell(row, column))
                .map(|cell| cell.text.as_str())
                .collect::<String>()
        })
        .collect()
}
