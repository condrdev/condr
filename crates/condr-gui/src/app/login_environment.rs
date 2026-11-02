//! A GUI opened from the Dock, Finder or a desktop menu adopts the environment the
//! user's login shell builds, for itself and what it starts (ADR 0057). Started from a
//! terminal, it keeps that terminal's.

use std::io::IsTerminal as _;

/// Rewrites this process's environment, so it must run before any other thread starts.
#[allow(unsafe_code)] // `set_var` while no other thread can read the environment.
pub(super) fn adopt() -> Result<(), String> {
    if std::io::stdout().is_terminal() {
        return Ok(());
    }
    for (key, value) in condr_server::login_environment::captured()? {
        // SAFETY: `run` calls this before logging or GPUI start a thread; the probe's
        // reader thread only reads a pipe.
        unsafe { std::env::set_var(key, value) };
    }
    Ok(())
}
