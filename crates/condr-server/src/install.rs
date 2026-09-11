//! `condr server install` and `condr server uninstall` (ADR 0016): the running binary
//! puts itself into the user's Condr directory and registers it on PATH; uninstall
//! undoes exactly that. Nothing here downloads, unpacks or verifies anything.

use std::ffi::OsStr;
use std::fs;
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::json;

use crate::{ServerConfig, probe_server};

const BINARY: &str = if cfg!(windows) { "condr.exe" } else { "condr" };
#[cfg_attr(not(unix), allow(dead_code))]
const PROFILE_MARKER: &str = "# condr user bin";
#[cfg_attr(not(unix), allow(dead_code))]
const PROFILE_EXPORT: &str = "export PATH=\"$HOME/.local/bin:$PATH\"";

pub struct InstallOptions {
    pub start: bool,
    pub restart: bool,
    pub yes: bool,
    pub json: bool,
}

pub struct UninstallOptions {
    pub yes: bool,
    pub json: bool,
}

/// One line per step for a person; silent when a program asked for `--json`.
struct Steps {
    json: bool,
}

impl Steps {
    fn say(&self, text: impl AsRef<str>) {
        if !self.json {
            println!("condr-server: {}", text.as_ref());
        }
    }
}

fn detail(headline: impl Into<String>, error: impl std::fmt::Display) -> io::Error {
    io::Error::other(format!("{}\n{error}", headline.into()))
}

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// `CONDR_INSTALL_DIR`, else `<home>/.local/opt/condr` on Unix or
/// `<LOCALAPPDATA>\Programs\Condr` on Windows. `platform_root` is `HOME` or
/// `%LOCALAPPDATA%`; without either there is nowhere to install.
fn resolve_install_dir(
    override_dir: Option<PathBuf>,
    platform_root: Option<PathBuf>,
) -> io::Result<PathBuf> {
    if let Some(dir) = override_dir {
        return Ok(dir);
    }
    let root = platform_root.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            if cfg!(windows) {
                "LOCALAPPDATA is not set; set CONDR_INSTALL_DIR"
            } else {
                "HOME is not set; set CONDR_INSTALL_DIR"
            },
        )
    })?;
    Ok(if cfg!(windows) {
        root.join("Programs").join("Condr")
    } else {
        root.join(".local").join("opt").join("condr")
    })
}

fn install_dir() -> io::Result<PathBuf> {
    let platform_root = env_path(if cfg!(windows) {
        "LOCALAPPDATA"
    } else {
        "HOME"
    });
    resolve_install_dir(env_path("CONDR_INSTALL_DIR"), platform_root)
}

/// The profile text with the marked `export PATH` appended once; `None` when it is
/// already there.
#[cfg_attr(not(unix), allow(dead_code))]
fn profile_with_marker(text: &str) -> Option<String> {
    if text.lines().any(|line| line == PROFILE_MARKER) {
        return None;
    }
    let mut out = text.to_owned();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&format!("\n{PROFILE_MARKER}\n{PROFILE_EXPORT}\n"));
    Some(out)
}

/// The profile text without the marker line and the export line after it; `None`
/// when there is nothing to remove.
#[cfg_attr(not(unix), allow(dead_code))]
fn profile_without_marker(text: &str) -> Option<String> {
    if !text.lines().any(|line| line == PROFILE_MARKER) {
        return None;
    }
    let mut out = String::new();
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        if line == PROFILE_MARKER {
            lines.next_if(|next| *next == PROFILE_EXPORT);
            // The blank line install put before the marker is ours too.
            if out.ends_with("\n\n") {
                out.pop();
            }
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    Some(out)
}

fn same_path_entry(a: &str, b: &str) -> bool {
    let trim = |s: &str| s.trim().trim_end_matches(['\\', '/']).to_lowercase();
    trim(a) == trim(b)
}

/// A Windows user `Path` value with `dir` appended once; `None` when it is present.
#[cfg_attr(not(windows), allow(dead_code))]
fn user_path_with(path: &str, dir: &str) -> Option<String> {
    let mut parts: Vec<&str> = path.split(';').filter(|part| !part.is_empty()).collect();
    if parts.iter().any(|part| same_path_entry(part, dir)) {
        return None;
    }
    parts.push(dir);
    Some(parts.join(";"))
}

/// A Windows user `Path` value without `dir`; `None` when it was not there.
#[cfg_attr(not(windows), allow(dead_code))]
fn user_path_without(path: &str, dir: &str) -> Option<String> {
    let parts: Vec<&str> = path.split(';').filter(|part| !part.is_empty()).collect();
    let kept: Vec<&str> = parts
        .iter()
        .copied()
        .filter(|part| !same_path_entry(part, dir))
        .collect();
    (kept.len() != parts.len()).then(|| kept.join(";"))
}

/// Asks on a terminal; without one, only `--yes` answers.
fn confirm(question: &str, yes: bool) -> io::Result<bool> {
    if yes {
        return Ok(true);
    }
    if !io::stdin().is_terminal() {
        return Err(io::Error::other(format!(
            "{question}\nno terminal to ask on; pass --yes to confirm"
        )));
    }
    print!("{question} [y/N] ");
    io::stdout().flush()?;
    let mut answer = String::new();
    io::stdin().lock().read_line(&mut answer)?;
    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

fn unique_suffix() -> u128 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    (u128::from(std::process::id()) << 96) ^ nanos
}

fn is_old_file(name: &OsStr) -> bool {
    Path::new(name).extension() == Some(OsStr::new("old"))
}

/// Deletes `.old` leftovers nothing holds any more; the ones still held are returned.
fn sweep_old_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.file_name().is_some_and(is_old_file))
        .filter(|path| fs::remove_file(path).is_err())
        .collect()
}

/// Copies `source` over `target` atomically for anyone executing it. Returns whether
/// an older file was displaced.
fn place_binary(source: &Path, target: &Path) -> io::Result<bool> {
    let dir = target.parent().expect("install path has a directory");
    fs::create_dir_all(dir)?;
    let suffix = unique_suffix();
    let staged = dir.join(format!("condr-{suffix:x}.new"));
    fs::copy(source, &staged).map_err(|error| {
        let _ = fs::remove_file(&staged);
        detail(format!("could not copy to {}", staged.display()), error)
    })?;
    let replaced = target.exists();
    #[cfg(windows)]
    if replaced {
        // A running executable cannot be overwritten but can be renamed.
        let old = dir.join(format!("condr-{suffix:x}.old"));
        fs::rename(target, &old).map_err(|error| {
            let _ = fs::remove_file(&staged);
            detail(
                format!("could not move the old {} aside", target.display()),
                error,
            )
        })?;
    }
    fs::rename(&staged, target).map_err(|error| {
        let _ = fs::remove_file(&staged);
        detail(format!("could not place {}", target.display()), error)
    })?;
    Ok(replaced)
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

#[cfg(unix)]
fn home() -> io::Result<PathBuf> {
    env_path("HOME").ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is not set"))
}

#[cfg(unix)]
fn profile_path(home: &Path) -> PathBuf {
    env_path("CONDR_PROFILE").unwrap_or_else(|| {
        home.join(if cfg!(target_os = "macos") {
            ".zprofile"
        } else {
            ".profile"
        })
    })
}

#[cfg(unix)]
fn register_path(target: &Path, steps: &Steps) -> io::Result<()> {
    let home = home()?;
    let bin_dir = home.join(".local").join("bin");
    let link = bin_dir.join(BINARY);
    fs::create_dir_all(&bin_dir)?;
    if fs::symlink_metadata(&link).is_ok() {
        fs::remove_file(&link)?;
    }
    std::os::unix::fs::symlink(target, &link)
        .map_err(|error| detail(format!("could not link {}", link.display()), error))?;
    steps.say(format!("linked {} -> {}", link.display(), target.display()));

    let profile = profile_path(&home);
    let text = match fs::read_to_string(&profile) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            return Err(detail(
                format!("could not read {}", profile.display()),
                error,
            ));
        }
    };
    match profile_with_marker(&text) {
        Some(text) => {
            if let Some(parent) = profile.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&profile, text)
                .map_err(|error| detail(format!("could not write {}", profile.display()), error))?;
            steps.say(format!(
                "added ~/.local/bin to PATH in {}",
                profile.display()
            ));
        }
        None => steps.say(format!(
            "{} already puts ~/.local/bin on PATH",
            profile.display()
        )),
    }
    Ok(())
}

#[cfg(unix)]
fn unregister_path(target: &Path, steps: &Steps) -> io::Result<()> {
    let home = home()?;
    let link = home.join(".local").join("bin").join(BINARY);
    if fs::read_link(&link).is_ok_and(|to| to == target) {
        fs::remove_file(&link)?;
        steps.say(format!("removed {}", link.display()));
    }
    let profile = profile_path(&home);
    if let Ok(text) = fs::read_to_string(&profile)
        && let Some(text) = profile_without_marker(&text)
    {
        fs::write(&profile, text)
            .map_err(|error| detail(format!("could not write {}", profile.display()), error))?;
        steps.say(format!("removed the PATH lines from {}", profile.display()));
    }
    Ok(())
}

// ponytail: the user Path goes through PowerShell, the way script/install-condr.ps1 did.
// Switch to a registry crate if PowerShell turns out to be missing somewhere that matters.
#[cfg(windows)]
fn powershell(command: &str, value: Option<(&str, &str)>) -> io::Result<String> {
    // PowerShell writes stdout in the console code page by default, which turns every
    // character outside it into `?`; a Path under a non-ASCII user name would come back
    // damaged and be written back that way. Values go in through the environment, which
    // is UTF-16 end to end.
    let command =
        format!("[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false); {command}");
    let mut process = std::process::Command::new("powershell");
    process.args(["-NoProfile", "-NonInteractive", "-Command", &command]);
    if let Some((value, kind)) = value {
        process.env("CONDR_USER_PATH", value);
        process.env("CONDR_USER_PATH_KIND", kind);
    }
    let output = process
        .output()
        .map_err(|error| detail("could not run PowerShell to edit the user Path", error))?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "PowerShell could not edit the user Path\n{}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .trim_end_matches(['\r', '\n'])
        .to_owned())
}

// The registry is read and written raw, keeping the value's kind:
// `[Environment]::GetEnvironmentVariable` expands `%USERPROFILE%`-style entries and
// `Set…` writes them back expanded as REG_SZ, which would rewrite the user's other
// entries. Returns the value and its kind (`String` or `ExpandString`).
#[cfg(windows)]
fn read_user_path() -> io::Result<(String, String)> {
    let output = powershell(
        concat!(
            "$key = Get-Item 'HKCU:\\Environment'; ",
            "if ($key.GetValueNames() -contains 'Path') { $key.GetValueKind('Path') } else { 'ExpandString' }; ",
            "$key.GetValue('Path', '', 'DoNotExpandEnvironmentNames')",
        ),
        None,
    )?;
    let (kind, value) = output.split_once(['\r', '\n']).unwrap_or((&output, ""));
    Ok((
        value.trim_start_matches(['\r', '\n']).to_owned(),
        kind.trim().to_owned(),
    ))
}

#[cfg(windows)]
fn write_user_path(value: &str, kind: &str) -> io::Result<()> {
    powershell(
        concat!(
            "$key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment', $true); ",
            "$key.SetValue('Path', $env:CONDR_USER_PATH, $env:CONDR_USER_PATH_KIND); $key.Close(); ",
            // Setting and clearing a scratch variable broadcasts WM_SETTINGCHANGE, so new
            // sessions see the Path without a logoff; the registry write alone does not.
            "[Environment]::SetEnvironmentVariable('CONDR_PATH_REFRESH', '1', 'User'); ",
            "[Environment]::SetEnvironmentVariable('CONDR_PATH_REFRESH', $null, 'User')",
        ),
        Some((value, kind)),
    )
    .map(drop)
}

#[cfg(windows)]
fn register_path(target: &Path, steps: &Steps) -> io::Result<()> {
    let dir = target.parent().expect("install path has a directory");
    let dir = dir.to_string_lossy();
    let (current, kind) = read_user_path()?;
    match user_path_with(&current, &dir) {
        Some(value) => {
            write_user_path(&value, &kind)?;
            steps.say(format!("added {dir} to the user Path"));
        }
        None => steps.say(format!("{dir} is already on the user Path")),
    }
    Ok(())
}

#[cfg(windows)]
fn unregister_path(target: &Path, steps: &Steps) -> io::Result<()> {
    let dir = target.parent().expect("install path has a directory");
    let dir = dir.to_string_lossy();
    let (current, kind) = read_user_path()?;
    if let Some(value) = user_path_without(&current, &dir) {
        write_user_path(&value, &kind)?;
        steps.say(format!("removed {dir} from the user Path"));
    }
    Ok(())
}

/// Puts the running executable in place and, when asked, serves from it. `Ok` carries
/// the exit code.
pub fn install(options: InstallOptions) -> io::Result<i32> {
    let steps = Steps { json: options.json };
    let target = install_dir()?.join(BINARY);
    let source = std::env::current_exe()?;

    let replaced = if same_file(&source, &target) {
        steps.say(format!(
            "{} is already the installed copy",
            target.display()
        ));
        false
    } else {
        let replaced = place_binary(&source, &target)?;
        steps.say(format!(
            "{} {}",
            if replaced { "replaced" } else { "installed" },
            target.display()
        ));
        replaced
    };
    for leftover in sweep_old_files(target.parent().expect("install path has a directory")) {
        steps.say(format!("{} is still in use; left it", leftover.display()));
    }
    register_path(&target, &steps)?;

    let config = ServerConfig::default();
    let endpoint = config.local_endpoint();
    let running = probe_server(&endpoint).is_ok();
    let server = if options.restart {
        if std::env::var_os("CONDR_PANE_ID").is_some() {
            return Err(io::Error::other(
                "run `condr server install --restart` from a terminal outside Condr; shutdown stops every Pane",
            ));
        }
        if running
            && !confirm(
                "Restarting the Server ends every Session and PTY. Restart it now?",
                options.yes,
            )?
        {
            return Err(io::Error::other(
                "not restarted; the running Server keeps serving from its previous binary",
            ));
        }
        crate::restart_server_from(config, &target)
            .map_err(|error| detail("failed to restart the Server", error))?;
        if running { "restarted" } else { "started" }
    } else if options.start {
        if running {
            "running"
        } else {
            crate::ensure_server_from(config, &target)
                .map_err(|error| detail("failed to start the Server", error))?;
            "started"
        }
    } else if running {
        "running"
    } else {
        "none"
    };
    match server {
        "none" => steps.say("no Server is running; start one with `condr server start`"),
        "running" => steps.say(
            "a Server is running from its previous binary; `condr server restart` switches it",
        ),
        _ => steps.say(format!("Server {server} at {endpoint}")),
    }

    if options.json {
        println!(
            "{}",
            json!({
                "install_path": target,
                "version": env!("CARGO_PKG_VERSION"),
                "protocol": condr_core::protocol::PROTOCOL_VERSION,
                "replaced": replaced,
                "path_registered": true,
                "server": server,
            })
        );
    } else {
        steps.say("open a new terminal, then run: condr --help");
    }
    Ok(0)
}

/// Removes the installed `condr` and its PATH registration; the user's data stays.
pub fn uninstall(options: UninstallOptions) -> io::Result<i32> {
    let steps = Steps { json: options.json };
    let dir = install_dir()?;
    let target = dir.join(BINARY);

    let config = ServerConfig::default();
    let endpoint = config.local_endpoint();
    let server = if probe_server(&endpoint).is_ok() {
        if std::env::var_os("CONDR_PANE_ID").is_some() {
            return Err(io::Error::other(
                "run `condr server uninstall` from a terminal outside Condr; shutdown stops every Pane, this one included",
            ));
        }
        if !confirm(
            "Stopping the Server ends every Session and PTY. Stop it now?",
            options.yes,
        )? {
            return Err(io::Error::other(
                "not uninstalled; the Server keeps running",
            ));
        }
        crate::stop_server(&endpoint)
            .map_err(|error| detail("failed to stop the Server", error))?;
        crate::wait_for_shutdown(&config.socket_path)?;
        steps.say("stopped the Server");
        "stopped"
    } else {
        "none"
    };

    let mut leftovers = Vec::new();
    match fs::remove_file(&target) {
        Ok(()) => steps.say(format!("removed {}", target.display())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            steps.say(format!("{} was not installed", target.display()));
        }
        Err(error) if cfg!(windows) => {
            // The running condr.exe cannot delete itself; a later install sweeps it.
            let old = dir.join(format!("condr-{:x}.old", unique_suffix()));
            fs::rename(&target, &old)
                .map_err(|_| detail(format!("could not remove {}", target.display()), error))?;
            steps.say(format!(
                "{} is in use; renamed to {}",
                target.display(),
                old.display()
            ));
            leftovers.push(old);
        }
        Err(error) => {
            return Err(detail(
                format!("could not remove {}", target.display()),
                error,
            ));
        }
    }
    leftovers.extend(sweep_old_files(&dir));
    if fs::remove_dir(&dir).is_ok() {
        steps.say(format!("removed {}", dir.display()));
    } else if dir.exists() {
        steps.say(format!("kept {}; other files are in it", dir.display()));
    }
    unregister_path(&target, &steps)?;

    if options.json {
        println!(
            "{}",
            json!({
                "removed_path": target,
                "path_unregistered": true,
                "server": server,
                "leftovers": leftovers,
            })
        );
    } else {
        let mut kept: Vec<PathBuf> = [
            condr_core::config_directory(),
            condr_core::state_directory(),
            condr_core::log_directory(),
        ]
        .into_iter()
        .flatten()
        .collect();
        kept.dedup();
        for path in kept {
            steps.say(format!("kept your data in {}", path.display()));
        }
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_dir_prefers_the_override_then_the_platform_root() {
        assert_eq!(
            resolve_install_dir(Some("/opt/x".into()), Some("/home/u".into())).unwrap(),
            PathBuf::from("/opt/x")
        );
        let expected = if cfg!(windows) {
            PathBuf::from("/home/u").join("Programs").join("Condr")
        } else {
            PathBuf::from("/home/u/.local/opt/condr")
        };
        assert_eq!(
            resolve_install_dir(None, Some("/home/u".into())).unwrap(),
            expected
        );
        assert_eq!(
            resolve_install_dir(None, None).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
    }

    #[test]
    fn profile_marker_is_added_once_and_removed_cleanly() {
        let added = profile_with_marker("# mine\n").unwrap();
        assert_eq!(
            added,
            "# mine\n\n# condr user bin\nexport PATH=\"$HOME/.local/bin:$PATH\"\n"
        );
        assert_eq!(profile_with_marker(&added), None);
        assert_eq!(profile_without_marker(&added).unwrap(), "# mine\n");
        assert_eq!(profile_without_marker("# mine\n"), None);
        // No trailing newline on the existing file, marker in the middle.
        let added = profile_with_marker("a").unwrap();
        assert!(added.starts_with("a\n\n# condr user bin\n"));
        let middle = format!("{added}b\n");
        assert_eq!(profile_without_marker(&middle).unwrap(), "a\nb\n");
        // The export line is only removed when it follows the marker.
        assert_eq!(
            profile_without_marker("# condr user bin\nexport OTHER=1\n").unwrap(),
            "export OTHER=1\n"
        );
    }

    #[test]
    fn user_path_entries_are_added_once_and_removed_case_insensitively() {
        let dir = r"C:\Users\u\AppData\Local\Programs\Condr";
        assert_eq!(
            user_path_with(r"C:\a;C:\b", dir).unwrap(),
            format!(r"C:\a;C:\b;{dir}")
        );
        assert_eq!(user_path_with("", dir).unwrap(), dir);
        assert_eq!(user_path_with(&format!(r"C:\a;{dir}\;"), dir), None);
        assert_eq!(user_path_with(&dir.to_lowercase(), dir), None);
        assert_eq!(
            user_path_without(&format!(r"C:\a;{dir};C:\b"), dir).unwrap(),
            r"C:\a;C:\b"
        );
        assert_eq!(user_path_without(dir, dir).unwrap(), "");
        assert_eq!(user_path_without(r"C:\a;C:\b", dir), None);
    }
}
