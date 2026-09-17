//! Logging for the Server and the GUI (ADR 0019).
//!
//! Both processes call [`init`] once from `main`. Events go through `tracing`; records
//! that `alacritty_terminal`, `portable-pty`, `notify` and gpui emit through `log` are
//! bridged into the same pipeline. The file layer writes off the calling thread, so a
//! PTY or render thread never waits on disk.
//!
//! Level policy: per-chunk and per-frame paths (PTY reads, VT parsing, frame
//! publication, `prepaint`/`paint`) carry `debug!` and `trace!` only; `info!` and above
//! belong to lifecycle, Agent state, recovery, configuration and errors.

use std::io::{self, IsTerminal as _};
use std::path::PathBuf;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;
use tracing_subscriber::{EnvFilter, Layer as _, fmt};

/// Holds an `EnvFilter` directive string. It replaces the default; it does not extend it.
pub const ENV_VAR: &str = "CONDR_LOG";
const DEFAULT_DIRECTIVES: &str = "warn,condr_core=info,condr_server=info,condr_gui=info";
const KEPT_FILES: usize = 7;

/// Keeps the file writer's thread alive. `main` holds it until it returns, so a normal
/// exit and an unwound panic flush the queue; never store it in a static.
#[must_use = "dropping the guard stops the log file writer"]
pub struct Guard {
    _file: Option<WorkerGuard>,
}

/// Installs the global subscriber and the panic hook.
///
/// The file layer writes `<log dir>/<file_stem>.<date>.log`, rolled daily, seven files
/// kept. Stderr gets everything when it is a terminal and only `error` otherwise: a
/// detached Server's stderr is the file its parent opened, a desktop-launched GUI's is
/// the session journal or nothing at all, and a write to a closed handle is discarded.
///
/// A second call, as tests may make, keeps the first subscriber and returns a guard
/// for nothing.
pub fn init(file_stem: &str) -> Guard {
    let (filter, filter_error) = env_filter(std::env::var(ENV_VAR).ok().as_deref());
    let stderr_layer = if io::stderr().is_terminal() {
        fmt::layer().with_writer(io::stderr).boxed()
    } else {
        fmt::layer()
            .with_writer(io::stderr)
            .with_ansi(false)
            .with_filter(LevelFilter::ERROR)
            .boxed()
    };
    let (file_layer, file_guard, file_error) = match open_log_file(file_stem) {
        Ok((writer, guard)) => (
            Some(fmt::layer().with_writer(writer).with_ansi(false)),
            Some(guard),
            None,
        ),
        Err(error) => (None, None, Some(error)),
    };
    let installed = tracing_subscriber::registry()
        .with(filter)
        .with(file_layer)
        .with(stderr_layer)
        .try_init()
        .is_ok();
    if !installed {
        return Guard { _file: None };
    }
    install_panic_hook();
    if let Some(error) = filter_error {
        tracing::warn!("{ENV_VAR} is not a valid filter, using the default: {error}");
    }
    if let Some(error) = file_error {
        tracing::warn!("logging to a file is unavailable: {error}");
    }
    Guard { _file: file_guard }
}

/// The filter from `CONDR_LOG`, or the default with the parse error when it is invalid.
fn env_filter(directives: Option<&str>) -> (EnvFilter, Option<String>) {
    match directives.map(str::trim).filter(|text| !text.is_empty()) {
        None => (EnvFilter::new(DEFAULT_DIRECTIVES), None),
        Some(text) => match EnvFilter::try_new(text) {
            Ok(filter) => (filter, None),
            Err(error) => (EnvFilter::new(DEFAULT_DIRECTIVES), Some(error.to_string())),
        },
    }
}

fn open_log_file(
    file_stem: &str,
) -> io::Result<(tracing_appender::non_blocking::NonBlocking, WorkerGuard)> {
    let directory = log_directory()?;
    let appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix(file_stem)
        .filename_suffix("log")
        .max_log_files(KEPT_FILES)
        .build(&directory)
        .map_err(io::Error::other)?;
    // Lossy: a full queue drops lines rather than blocking the caller.
    Ok(tracing_appender::non_blocking(appender))
}

fn log_directory() -> io::Result<PathBuf> {
    condr_core::log_directory().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "no platform log directory is available",
        )
    })
}

/// Logs the panic and its backtrace, then lets the previous hook print as before.
fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let backtrace = std::backtrace::Backtrace::force_capture();
        tracing::error!("{info}\n{backtrace}");
        previous(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_filter_falls_back_to_the_default_when_unset_blank_or_invalid() {
        // `EnvFilter` renders its directives in its own order.
        let default = EnvFilter::new(DEFAULT_DIRECTIVES).to_string();
        for text in [None, Some(""), Some("   ")] {
            let (filter, error) = env_filter(text);
            assert!(error.is_none(), "{text:?}");
            assert_eq!(filter.to_string(), default);
        }
        let (filter, error) = env_filter(Some("condr_server=loud"));
        assert!(error.is_some());
        assert_eq!(filter.to_string(), default);
    }

    #[test]
    fn env_filter_replaces_the_default() {
        let (filter, error) = env_filter(Some("debug"));
        assert!(error.is_none());
        assert_eq!(filter.to_string(), "debug");
    }
}
