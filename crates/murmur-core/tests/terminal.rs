use std::time::{Duration, Instant};

#[cfg(target_os = "linux")]
use murmur_core::{
    AgentKind, AgentState, TerminalPosition, TerminalScroll, TerminalSide, TerminalUpdate,
};
use murmur_core::{CommandBuilder, TerminalCommand, TerminalRuntime, TerminalSize};

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
printf '\r\nreply=%s area=%s env=%s/%s cwd=%s\r\n' "$reply" "$area" "$TERM" "$COLORTERM" "$PWD"
IFS= read -r line
printf 'input=%s\r\n' "$line"
stty size"#,
    );
    command.cwd(&cwd);

    let mut runtime =
        TerminalRuntime::spawn(command, TerminalSize::new(24, 80).with_cell_size(8, 16)).unwrap();
    wait_for_text(&runtime, "reply=1b5b323b3152");
    wait_for_text(&runtime, "area=1b5b343b3338343b36343074");

    runtime.resize(TerminalSize::new(40, 100)).unwrap();
    runtime.write(b"hello\r".to_vec()).unwrap();
    wait_for_text(&runtime, "40 100");

    let status = runtime.wait().unwrap();
    assert!(status.success());
    assert!(runtime.wait().is_err());
    assert!(runtime.revision() > 0);
    let text = runtime.visible_text();
    assert!(text.contains("ready \u{4e16}\u{754c} e\u{301}"), "{text:?}");
    assert!(text.contains("env=xterm-256color/truecolor"), "{text:?}");
    assert!(text.contains(&format!("cwd={}", cwd.display())), "{text:?}");
    assert!(text.contains("input=hello"), "{text:?}");
    assert!(text.contains("40 100"), "{text:?}");

    let mut command = CommandBuilder::new("/bin/sh");
    command.args(["-c", "sleep 30"]);
    let mut runtime = TerminalRuntime::spawn(command, TerminalSize::new(24, 80)).unwrap();
    assert!(!runtime.shutdown().unwrap().success());
    assert!(runtime.shutdown().is_err());
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
                selection: murmur_core::TerminalSelection {
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
        "printf 'Working (1s - esc to interrupt)\\n'; bash -c 'exec -a codex sleep 30' & wait",
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

    runtime.resize(TerminalSize::new(40, 100)).unwrap();
    runtime
        .execute(TerminalCommand::Text(
            "$s=$Host.UI.RawUI.WindowSize; Write-Output \"size=$($s.Height)x$($s.Width)\"\r".into(),
        ))
        .unwrap();
    wait_for_text(&runtime, "size=40x100");

    runtime
        .execute(TerminalCommand::Text(
            "Write-Output 'final-before-exit'; exit\r".into(),
        ))
        .unwrap();
    wait_for_text(&runtime, "final-before-exit");
    assert!(runtime.wait().unwrap().success());
    assert!(runtime.visible_text().contains("final-before-exit"));
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
fn visible_rows(view: &murmur_core::TerminalView) -> Vec<String> {
    (0..view.size.rows)
        .map(|row| {
            (0..view.size.columns)
                .filter_map(|column| view.cell(row, column))
                .map(|cell| cell.text.as_str())
                .collect::<String>()
        })
        .collect()
}
