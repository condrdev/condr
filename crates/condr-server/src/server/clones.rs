//! The `git clone` a Clone Repository types into its new Workspace's first Pane once the
//! shell reaches its prompt (ADR 0059). Nothing about it is in the Snapshot.

use super::*;

/// How long a new shell has to reach its prompt before the Pane gets the command to run
/// instead.
const CLONE_PROMPT_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone)]
pub(super) struct PendingClone {
    url: String,
    root: PathBuf,
    deadline: Instant,
}

impl RuntimeState {
    pub(super) fn queue_clone(&mut self, pane_id: PaneId, url: String, root: PathBuf) {
        self.pending_clones.insert(
            pane_id,
            PendingClone {
                url,
                root,
                deadline: Instant::now() + CLONE_PROMPT_TIMEOUT,
            },
        );
    }
}

/// Types the Pane's pending clone once its shell is ready, outside the Session lock like
/// a resume (ADR 0005), and says whether one is still pending. Keys the person typed
/// first, a shell that never reaches its prompt, or one Condr cannot type into leave the
/// command printed in the Pane instead.
pub(super) fn type_pending_clone(
    state: &Arc<Mutex<RuntimeState>>,
    pane_id: PaneId,
    instance_id: u64,
) -> bool {
    let (clone, probe, typed) = {
        let mut state = state.lock().or_abort();
        if !state.terminal_is_current(pane_id, instance_id)
            || state.closing_terminals.contains(&pane_id)
        {
            state.pending_clones.remove(&pane_id);
            return false;
        }
        let Some(clone) = state.pending_clones.get(&pane_id).cloned() else {
            return false;
        };
        let terminal = &state.terminals[&pane_id];
        (clone, terminal.launch_probe(), terminal.typed())
    };
    let line = if typed {
        Err(io::Error::other("keys reached the Pane first"))
    } else if Instant::now() >= clone.deadline {
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "the shell did not reach its prompt in 30 seconds",
        ))
    } else {
        match probe.readiness() {
            ShellReadiness::Ready(shell) => {
                condr_core::clone_command(&shell, &clone.url, &clone.root)
            }
            ShellReadiness::Starting | ShellReadiness::Busy => return true,
        }
    };
    let mut state = state.lock().or_abort();
    if !state.terminal_is_current(pane_id, instance_id)
        || state.pending_clones.remove(&pane_id).is_none()
    {
        return false;
    }
    let terminal = &state.terminals[&pane_id];
    match line.and_then(|line| terminal.submit(&line)) {
        Ok(()) => false,
        // The input queue is full for a moment; the next tick tries again.
        Err(error)
            if error.kind() == io::ErrorKind::WouldBlock && Instant::now() < clone.deadline =>
        {
            state.pending_clones.insert(pane_id, clone);
            true
        }
        Err(error) => {
            let message = format!(
                "Could not start the clone: {error}. Run: {}",
                plain_clone_command(&clone.url, &clone.root)
            );
            tracing::info!("Pane {}: {message}", pane_id.as_u64());
            terminal.print_notice(&message);
            false
        }
    }
}

/// The command as a person would type it: single quotes everywhere but Windows, whose
/// cmd takes only double ones and whose PowerShell takes both.
fn plain_clone_command(url: &str, root: &std::path::Path) -> String {
    let quote = if cfg!(windows) { '"' } else { '\'' };
    format!(
        "git clone -- {quote}{url}{quote} {quote}{}{quote}",
        root.display()
    )
}
