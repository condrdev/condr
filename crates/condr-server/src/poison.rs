//! A poisoned Server lock ends the Server (ADR 0053): the Session it guards may be
//! half-changed, and carrying on would persist it into the Snapshot. The GUI's reconnect
//! starts a new Server, which comes back from the last Snapshot written.

use std::panic::Location;
use std::process;
use std::sync::LockResult;

pub(crate) trait OrAbort<T> {
    /// What the lock gave, or the end of the process when a panic poisoned it.
    fn or_abort(self) -> T;
}

impl<T> OrAbort<T> for LockResult<T> {
    #[track_caller]
    fn or_abort(self) -> T {
        // Read here: a closure does not inherit `#[track_caller]`.
        let caller = Location::caller();
        self.unwrap_or_else(|_| abort_server(&format!("a lock taken at {caller} was poisoned")))
    }
}

/// The panic that led here is already in the log with its backtrace (ADR 0019).
pub(crate) fn abort_server(reason: &str) -> ! {
    tracing::error!("stopping the Server: {reason} by a panic");
    process::abort()
}
