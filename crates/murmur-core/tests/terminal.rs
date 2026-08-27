#![cfg(target_os = "linux")]

use std::time::{Duration, Instant};

use murmur_core::{CommandBuilder, TerminalRuntime, TerminalSize};

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
