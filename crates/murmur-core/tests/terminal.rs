#![cfg(target_os = "linux")]

use std::time::{Duration, Instant};

use murmur_core::{
    CommandBuilder, TerminalCommand, TerminalPosition, TerminalRuntime, TerminalScroll,
    TerminalSide, TerminalSize, TerminalUpdate,
};

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
    runtime
        .execute(TerminalCommand::Select {
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
        })
        .unwrap();
    assert!(runtime.view().cell(0, 0).unwrap().selected);
    assert_eq!(
        runtime.execute(TerminalCommand::Copy).unwrap(),
        Some("alpha".into())
    );
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
