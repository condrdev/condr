//! Ctrl+C for the CLI's long-running commands (ADR 0061): `port forward` exits 0 on it,
//! and `file` removes its temporary file first. Unix waits for SIGINT and SIGTERM on a
//! thread of its own; Windows takes the console's control handler. Neither needs a crate.

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

type Action = Box<dyn Fn() + Send>;

/// Whether Ctrl+C came, and what to do when it does. Until an action is set, Ctrl+C
/// ends the process with 130, as a shell's interrupted command does.
#[derive(Clone)]
pub(super) struct Interrupt {
    fired: Arc<AtomicBool>,
    action: Arc<Mutex<Option<Action>>>,
}

impl Interrupt {
    /// Must run before the command starts any thread: on Unix the signals are blocked in
    /// this thread, every thread spawned after it inherits that, and one waits for them.
    pub(super) fn install() -> io::Result<Self> {
        let interrupt = Self {
            fired: Arc::default(),
            action: Arc::default(),
        };
        platform::install(interrupt.clone())?;
        Ok(interrupt)
    }

    /// One that no signal reaches, for tests to fire by hand.
    #[cfg(test)]
    pub(super) fn by_hand() -> Self {
        Self {
            fired: Arc::default(),
            action: Arc::default(),
        }
    }

    pub(super) fn on_interrupt(&self, action: impl Fn() + Send + 'static) {
        *self.action.lock().unwrap_or_else(PoisonError::into_inner) = Some(Box::new(action));
    }

    pub(super) fn fired(&self) -> bool {
        self.fired.load(Ordering::Acquire)
    }

    pub(super) fn fire(&self) {
        self.fired.store(true, Ordering::Release);
        match &*self.action.lock().unwrap_or_else(PoisonError::into_inner) {
            Some(action) => action(),
            None => std::process::exit(130),
        }
    }
}

#[cfg(unix)]
mod platform {
    use super::Interrupt;
    use nix::sys::signal::{SigSet, SigmaskHow, Signal, pthread_sigmask};
    use std::io;

    pub(super) fn install(interrupt: Interrupt) -> io::Result<()> {
        let mut signals = SigSet::empty();
        signals.add(Signal::SIGINT);
        signals.add(Signal::SIGTERM);
        pthread_sigmask(SigmaskHow::SIG_BLOCK, Some(&signals), None).map_err(io::Error::from)?;
        std::thread::Builder::new()
            .name("condr-interrupt".into())
            .spawn(move || {
                if signals.wait().is_ok() {
                    interrupt.fire();
                }
            })?;
        Ok(())
    }
}

#[cfg(windows)]
mod platform {
    use super::Interrupt;
    use std::io;
    use std::sync::OnceLock;
    use windows_sys::Win32::Foundation::{BOOL, TRUE};
    use windows_sys::Win32::System::Console::SetConsoleCtrlHandler;

    static INTERRUPT: OnceLock<Interrupt> = OnceLock::new();

    /// Runs on a thread the console starts for the event; returning TRUE keeps the
    /// process alive so the command can finish on its own.
    unsafe extern "system" fn handler(_event: u32) -> BOOL {
        if let Some(interrupt) = INTERRUPT.get() {
            interrupt.fire();
        }
        TRUE
    }

    pub(super) fn install(interrupt: Interrupt) -> io::Result<()> {
        let _ = INTERRUPT.set(interrupt);
        // SAFETY: `handler` is a plain function that lives as long as the process.
        if unsafe { SetConsoleCtrlHandler(Some(handler), TRUE) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}
