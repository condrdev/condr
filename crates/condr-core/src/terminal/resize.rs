use super::*;

#[derive(Default)]
pub(super) struct ResizeState {
    pending: Option<TerminalSize>,
    stopping: bool,
}

#[derive(Default)]
pub(super) struct ResizeControl {
    state: Mutex<ResizeState>,
    wake: Condvar,
}

impl ResizeControl {
    pub(super) fn request(&self, size: TerminalSize) -> io::Result<()> {
        let mut state = self.state.lock().expect("terminal resize lock poisoned");
        if state.stopping {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "terminal resizer stopped",
            ));
        }
        state.pending = Some(size);
        self.wake.notify_one();
        Ok(())
    }

    pub(super) fn stop(&self) {
        let mut state = self.state.lock().expect("terminal resize lock poisoned");
        state.stopping = true;
        state.pending = None;
        self.wake.notify_all();
    }

    pub(super) fn next(&self) -> Option<TerminalSize> {
        let mut state = self.state.lock().expect("terminal resize lock poisoned");
        while state.pending.is_none() && !state.stopping {
            state = self
                .wake
                .wait(state)
                .expect("terminal resize lock poisoned");
        }
        (!state.stopping).then(|| state.pending.take()).flatten()
    }
}

pub(super) struct ResizeWorkerGuard {
    control: Arc<ResizeControl>,
}

impl Drop for ResizeWorkerGuard {
    fn drop(&mut self) {
        self.control.stop();
    }
}

const PTY_RESIZE_RETRY_DELAY: Duration = Duration::from_millis(50);

pub(super) fn resize_loop(
    control: Arc<ResizeControl>,
    master: Weak<Mutex<Box<dyn MasterPty + Send>>>,
    terminal: Arc<Mutex<Terminal>>,
    current_size: Arc<Mutex<TerminalSize>>,
    revision: Arc<AtomicU64>,
    updates: mpsc::Sender<TerminalUpdate>,
) -> io::Result<()> {
    let _exit = ResizeWorkerGuard {
        control: Arc::clone(&control),
    };
    while let Some(size) = control.next() {
        let master = master
            .upgrade()
            .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "PTY was closed"))?;
        // A failed ioctl must not kill the worker. The VT grid is already at the new
        // size, so retry the PTY once shortly after instead of leaving the two apart.
        let resize_pty = |pty_size: PtySize| {
            master
                .lock()
                .expect("PTY master lock poisoned")
                .resize(pty_size)
                .map_err(other_error)
        };
        if resize_terminal_and_pty(
            &terminal,
            &current_size,
            &revision,
            &updates,
            size,
            |_, pty_size| resize_pty(pty_size),
        )
        .is_err()
        {
            thread::sleep(PTY_RESIZE_RETRY_DELAY);
            let _ = resize_pty(size.into());
        }
    }
    Ok(())
}

pub(super) fn resize_terminal_and_pty(
    terminal: &Arc<Mutex<Terminal>>,
    current_size: &Arc<Mutex<TerminalSize>>,
    revision: &AtomicU64,
    updates: &mpsc::Sender<TerminalUpdate>,
    size: TerminalSize,
    resize_pty: impl FnOnce(&Terminal, PtySize) -> io::Result<()>,
) -> io::Result<()> {
    let mut terminal = terminal.lock().expect("terminal state lock poisoned");
    terminal.resize(size);
    let resize_result = resize_pty(&terminal, size.into());
    *current_size.lock().expect("terminal size lock poisoned") = size;
    drop(terminal);
    publish_view(revision, updates);
    resize_result
}
