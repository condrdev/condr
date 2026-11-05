//! `condr --device <name> file download|upload` (ADR 0061): one regular file between this
//! machine and a Workspace of a remote Device, over a Stream connection that rides
//! Condr's SSH master like every other `--device` call. The target is written beside
//! itself and placed only once complete; an existing one needs `--replace`.

use super::interrupt::Interrupt;
use super::port::stream_connect_error;
use super::*;
use condr_client::{StreamConnection, StreamFailure};
use condr_core::protocol::{RelativePathBuf, StreamError, StreamFile};
use std::io::{IsTerminal as _, Write as _};
use std::time::{Duration, Instant};

#[derive(Subcommand)]
pub(crate) enum FileCommand {
    /// Download one file of a Workspace on a remote Device (needs --device)
    Download {
        /// Relative to the Workspace's root directory
        path: String,
        /// Where to save it; defaults to the file's name in the current directory, and
        /// a directory takes the file's name inside it
        dest: Option<PathBuf>,
        #[arg(long)]
        workspace: u64,
        /// Overwrite a destination that exists
        #[arg(long)]
        replace: bool,
    },
    /// Upload one file into a Workspace on a remote Device (needs --device)
    Upload {
        file: PathBuf,
        /// Relative to the Workspace's root directory
        path: String,
        #[arg(long)]
        workspace: u64,
        /// Overwrite a file that exists there
        #[arg(long)]
        replace: bool,
    },
}

pub(crate) fn run_file(device: Option<&str>, command: FileCommand) -> i32 {
    match file(device, command) {
        Ok(value) => {
            println!("{value}");
            0
        }
        Err(error) => {
            eprintln!("{}", json!({ "error": error }));
            1
        }
    }
}

fn file(device: Option<&str>, command: FileCommand) -> Result<Value, CliError> {
    let device = device.ok_or_else(|| {
        CliError::new(
            "device_required",
            "file needs --device <name> (or CONDR_DEVICE); on this machine, copy the file",
        )
    })?;
    let endpoint = device::device_endpoint(&device::saved_devices()?, device)?;
    let interrupt = Interrupt::install()?;
    transfer(&endpoint, command, &interrupt)
}

fn relative(text: &str) -> Result<RelativePathBuf, CliError> {
    if text.is_empty() || text.starts_with('/') || text.contains('\\') {
        return Err(CliError::new(
            "invalid_path",
            format!("{text:?} is not a path relative to the Workspace's root directory"),
        ));
    }
    Ok(RelativePathBuf::from(text))
}

/// Runs one transfer and prints its JSON result's value.
pub(super) fn transfer(
    endpoint: &Endpoint,
    command: FileCommand,
    interrupt: &Interrupt,
) -> Result<Value, CliError> {
    let (connection, _) = StreamConnection::connect(endpoint, "condr-cli")
        .map_err(|error| stream_connect_error(endpoint, &error))?;
    let connection = std::sync::Arc::new(connection);
    {
        let connection = std::sync::Arc::clone(&connection);
        interrupt.on_interrupt(move || connection.close());
    }
    let failed = |failure: StreamFailure| {
        if interrupt.fired() {
            CliError::new("cancelled", "cancelled; nothing was left behind")
        } else {
            transfer_error(failure)
        }
    };
    match command {
        FileCommand::Download {
            path,
            dest,
            workspace,
            replace,
        } => {
            let path = relative(&path)?;
            let name = path.file_name().unwrap_or_default().to_owned();
            let destination = match dest {
                Some(dest) if dest.is_dir() => dest.join(local_file_name(&name)?),
                Some(dest) => dest,
                None => std::env::current_dir()?.join(local_file_name(&name)?),
            };
            let mut meter = Meter::new(&name);
            let bytes = condr_client::download(
                connection.as_ref(),
                StreamFile {
                    workspace: WorkspaceId::from_u64(workspace),
                    path: path.clone(),
                },
                &destination,
                replace,
                |done, total| {
                    meter.show(done, total);
                    true
                },
            )
            .map_err(failed)?;
            meter.end();
            let destination = std::path::absolute(&destination).unwrap_or(destination);
            Ok(json!({ "download": {
                "path": path.as_str(),
                "dest": destination.display().to_string(),
                "bytes": bytes,
            }}))
        }
        FileCommand::Upload {
            file,
            path,
            workspace,
            replace,
        } => {
            let path = relative(&path)?;
            let mut meter = Meter::new(path.file_name().unwrap_or_default());
            let bytes = condr_client::upload(
                connection.as_ref(),
                &file,
                StreamFile {
                    workspace: WorkspaceId::from_u64(workspace),
                    path: path.clone(),
                },
                replace,
                |done, total| {
                    meter.show(done, total);
                    true
                },
            )
            .map_err(failed)?;
            meter.end();
            Ok(json!({ "upload": {
                "source": file.display().to_string(),
                "path": path.as_str(),
                "bytes": bytes,
            }}))
        }
    }
}

/// The name a download takes here, which must be one this machine can hold.
fn local_file_name(name: &str) -> Result<String, CliError> {
    if condr_core::valid_local_name(name) {
        Ok(name.to_owned())
    } else {
        Err(CliError::new(
            "invalid_name",
            format!("{name:?} cannot be a file name on this machine; give a destination"),
        ))
    }
}

fn transfer_error(failure: StreamFailure) -> CliError {
    let code = match failure.error {
        StreamError::AlreadyExists => "already_exists",
        StreamError::NotFound => "not_found",
        StreamError::NotAFile => "not_a_file",
        StreamError::InvalidPath => "invalid_path",
        StreamError::WorkspaceGone => "workspace_not_found",
        StreamError::Unsupported => "feature_unavailable",
        StreamError::Cancelled => "cancelled",
        StreamError::Io => "io",
        StreamError::ConnectionRefused
        | StreamError::InvalidPort
        | StreamError::TooManyStreams
        | StreamError::Other => "transfer_failed",
    };
    CliError::new(code, failure.to_string())
}

/// One line of progress on stderr while it is a terminal, redrawn at most ten times a
/// second; nothing when it is not.
struct Meter {
    name: String,
    started: Instant,
    drawn: Option<Instant>,
    terminal: bool,
}

impl Meter {
    fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            started: Instant::now(),
            drawn: None,
            terminal: io::stderr().is_terminal(),
        }
    }

    fn show(&mut self, done: u64, total: Option<u64>) {
        if !self.terminal
            || self
                .drawn
                .is_some_and(|drawn| drawn.elapsed() < Duration::from_millis(100))
        {
            return;
        }
        self.drawn = Some(Instant::now());
        let seconds = self.started.elapsed().as_secs_f64().max(0.001);
        let rate = done as f64 / seconds;
        let total = total.map_or_else(String::new, |total| format!(" / {}", bytes(total as f64)));
        let mut stderr = io::stderr();
        let _ = write!(
            stderr,
            "\r{}  {}{total}  {}/s\x1b[K",
            self.name,
            bytes(done as f64),
            bytes(rate)
        );
        let _ = stderr.flush();
    }

    fn end(&self) {
        if self.terminal && self.drawn.is_some() {
            eprintln!();
        }
    }
}

fn bytes(value: f64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut value = value;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{value:.0} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use condr_server::{BoundServer, ServerConfig};

    /// Downloads and uploads against a real Server with a Workspace on disk: an existing
    /// target needs `--replace` on both sides, a directory is refused, and the uploaded
    /// file arrives whole under its name.
    #[test]
    fn files_move_both_ways_and_an_existing_target_needs_replace() {
        let root = std::env::temp_dir().join(format!(
            "condr-cli-file-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let workspace_root = root.join("workspace");
        let local = root.join("local");
        std::fs::create_dir_all(workspace_root.join("docs")).unwrap();
        std::fs::create_dir_all(&local).unwrap();
        let payload: Vec<u8> = (0..3_000_000u32).map(|i| i as u8).collect();
        std::fs::write(workspace_root.join("docs/report.bin"), &payload).unwrap();

        let endpoint = Endpoint::local(root.join("server.sock"));
        let server =
            BoundServer::bind(ServerConfig::ephemeral(endpoint.as_local_path().unwrap())).unwrap();
        let handle = server.handle();
        let runtime = std::thread::spawn(move || server.run());
        let mut client = ClientConnection::connect_overview(&endpoint, "test").unwrap();
        let created = client
            .layout(LayoutCommand::CreateWorkspace {
                root_directory: workspace_root.clone(),
                name: None,
            })
            .unwrap()
            .unwrap();
        let LayoutResult::WorkspaceCreated { workspace_id, .. } = created else {
            panic!("expected the new Workspace, got {created:?}");
        };
        let workspace = workspace_id.as_u64();

        let run = |command| transfer(&endpoint, command, &Interrupt::by_hand());
        let saved = local.join("report.bin");
        let result = run(FileCommand::Download {
            path: "docs/report.bin".into(),
            dest: Some(local.clone()),
            workspace,
            replace: false,
        })
        .unwrap();
        assert_eq!(result["download"]["bytes"], payload.len());
        assert_eq!(std::fs::read(&saved).unwrap(), payload);
        let again = run(FileCommand::Download {
            path: "docs/report.bin".into(),
            dest: Some(saved.clone()),
            workspace,
            replace: false,
        });
        assert_eq!(again.unwrap_err().code, "already_exists");
        run(FileCommand::Download {
            path: "docs/report.bin".into(),
            dest: Some(saved.clone()),
            workspace,
            replace: true,
        })
        .unwrap();
        let directory = run(FileCommand::Download {
            path: "docs".into(),
            dest: Some(local.join("docs")),
            workspace,
            replace: false,
        });
        assert_eq!(directory.unwrap_err().code, "not_a_file");

        let upload = |path: &str, replace| {
            run(FileCommand::Upload {
                file: saved.clone(),
                path: path.into(),
                workspace,
                replace,
            })
        };
        assert_eq!(
            upload("docs/copy.bin", false).unwrap()["upload"]["bytes"],
            payload.len()
        );
        assert_eq!(
            std::fs::read(workspace_root.join("docs/copy.bin")).unwrap(),
            payload
        );
        assert_eq!(
            upload("docs/copy.bin", false).unwrap_err().code,
            "already_exists"
        );
        upload("docs/copy.bin", true).unwrap();
        assert_eq!(
            upload(".git/config", false).unwrap_err().code,
            "invalid_path"
        );
        // Nothing half-written is left beside the targets.
        let leftovers: Vec<_> = std::fs::read_dir(workspace_root.join("docs"))
            .unwrap()
            .flatten()
            .filter(|entry| condr_core::is_transfer_temp(&entry.file_name().to_string_lossy()))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");

        handle.stop();
        runtime.join().unwrap().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}
