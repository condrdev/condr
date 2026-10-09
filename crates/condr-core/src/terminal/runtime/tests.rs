use super::*;
use std::cell::RefCell;

thread_local! {
    static FAILURE: RefCell<Option<FailedStart>> = const { RefCell::new(None) };
}

struct FailedStart {
    stage: &'static str,
    resources: Option<StartedResources>,
}

struct StartedResources {
    pid: Pid,
    terminal: Weak<Mutex<Terminal>>,
    writer_stopping: Weak<AtomicBool>,
    resize: Weak<ResizeControl>,
}

pub(super) fn before_thread_start(
    runtime: &TerminalRuntime,
    stage: &'static str,
) -> io::Result<()> {
    FAILURE.with_borrow_mut(|failure| {
        if let Some(failure) = failure
            && failure.stage == stage
        {
            failure.resources = Some(StartedResources {
                pid: Pid::from_u32(runtime.process.shell_pid.expect("a real shell has a PID")),
                terminal: Arc::downgrade(&runtime.terminal),
                writer_stopping: Arc::downgrade(&runtime.writer_stopping),
                resize: Arc::downgrade(&runtime.resize),
            });
            return Err(io::Error::other(format!("injected {stage} start failure")));
        }
        Ok(())
    })
}

#[test]
#[cfg(any(unix, windows))]
fn thread_start_failure_reaps_child_and_releases_io_for_every_stage() {
    for stage in ["writer", "resizer", "reader", "child-watch"] {
        FAILURE.with_borrow_mut(|failure| {
            *failure = Some(FailedStart {
                stage,
                resources: None,
            });
        });
        #[cfg(unix)]
        let command = {
            let mut command = CommandBuilder::new("sh");
            command.args(["-c", "exec sleep 60"]);
            command
        };
        #[cfg(windows)]
        let command = {
            let mut command = CommandBuilder::new("powershell.exe");
            command.args(["-NoProfile", "-Command", "Start-Sleep -Seconds 60"]);
            command
        };
        let result = TerminalRuntime::spawn(command, TerminalSize::new(24, 80));
        let failure = FAILURE.take().expect("failure was configured");
        let resources = failure.resources.expect("the requested stage was reached");
        let error = result.err().expect("thread creation must fail");
        assert_eq!(error.to_string(), format!("injected {stage} start failure"));
        // Windows lists an exited process until its last handle closes, and the
        // pseudoconsole host lets go of the shell's tens of milliseconds after the Job
        // ended it; a slow CI runner saw it still listed (exit code 1) on the first check.
        let deadline = Instant::now() + Duration::from_secs(5);
        while process_table().process(resources.pid).is_some() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }
        assert!(
            process_table().process(resources.pid).is_none(),
            "{stage} failure left the shell running or unreaped"
        );
        assert_eq!(resources.terminal.strong_count(), 0, "{stage}: reader");
        assert_eq!(
            resources.writer_stopping.strong_count(),
            0,
            "{stage}: writer"
        );
        assert_eq!(resources.resize.strong_count(), 0, "{stage}: resizer");
    }
}
