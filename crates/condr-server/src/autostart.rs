//! Autostart (ADR 0057): `condr server start` registered with the user's own service
//! manager, so this machine's Server starts when its user logs in, or when it boots on
//! Linux. A systemd user unit, a LaunchAgent or a `Run` value; whether Autostart is on is
//! read back from it and stored nowhere else.

use std::io;
use std::path::{Path, PathBuf};

/// The `condr` that starts the Server at login, or `None` when Autostart is off.
pub fn registered() -> io::Result<Option<PathBuf>> {
    platform::registered()
}

/// Registers `executable` to run `server start` from the next login on, replacing an
/// earlier registration. It does not start a Server now.
pub fn enable(executable: &Path) -> io::Result<()> {
    let executable = executable.to_str().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} is not a UTF-8 path", executable.display()),
        )
    })?;
    platform::enable(executable)
}

/// Removes the registration, if there is one; whether there was.
pub fn disable() -> io::Result<bool> {
    platform::disable()
}

fn remove_if_present(path: &Path) -> io::Result<bool> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use std::fs;
    use std::io;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};

    use super::{remove_if_present, systemd_exec_path, systemd_unit};

    const UNIT: &str = "condr-server.service";

    /// `$XDG_CONFIG_HOME/systemd/user`, where `systemctl --user enable` puts a unit.
    fn unit_directory() -> io::Result<PathBuf> {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .or_else(|| std::env::home_dir().map(|home| home.join(".config")))
            .map(|config| config.join("systemd").join("user"))
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is not set"))
    }

    /// What `systemctl --user enable` links, and `exists` follows to the unit.
    fn wants_link(directory: &Path) -> PathBuf {
        directory.join("default.target.wants").join(UNIT)
    }

    pub(super) fn registered() -> io::Result<Option<PathBuf>> {
        let directory = unit_directory()?;
        if !wants_link(&directory).exists() {
            return Ok(None);
        }
        let unit = fs::read_to_string(directory.join(UNIT))?;
        systemd_exec_path(&unit).map(Some).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{UNIT} names no program to start"),
            )
        })
    }

    pub(super) fn enable(executable: &str) -> io::Result<()> {
        // What `sd_booted` checks: systemd is this machine's init.
        if !Path::new("/run/systemd/system").is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Autostart needs systemd, and this machine does not run it",
            ));
        }
        enable_linger()?;
        let directory = unit_directory()?;
        let unit = directory.join(UNIT);
        let link = wants_link(&directory);
        let written = (|| {
            fs::create_dir_all(link.parent().expect("the link is in a directory"))?;
            fs::write(&unit, systemd_unit(executable))?;
            remove_if_present(&link)?;
            std::os::unix::fs::symlink(&unit, &link)
        })();
        if let Err(error) = written {
            let _ = remove_if_present(&link);
            let _ = remove_if_present(&unit);
            return Err(error);
        }
        reload();
        Ok(())
    }

    pub(super) fn disable() -> io::Result<bool> {
        let directory = unit_directory()?;
        let enabled = remove_if_present(&wants_link(&directory))?;
        // Linger stays: the person may have enabled it for other services (ADR 0057).
        if remove_if_present(&directory.join(UNIT))? || enabled {
            reload();
        }
        Ok(enabled)
    }

    /// Without lingering, the user's service manager starts at the first login rather than
    /// at boot and stops at the last logout, taking the Server and its socket with it.
    fn enable_linger() -> io::Result<()> {
        use std::os::unix::fs::MetadataExt as _;
        let uid = fs::metadata("/proc/self")?.uid().to_string();
        let lingering = Command::new("loginctl")
            .args(["show-user", &uid, "--property=Linger", "--value"])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output();
        match lingering {
            Ok(output) if output.stdout.trim_ascii() == b"yes" => return Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "Autostart needs systemd's loginctl, which is not installed",
                ));
            }
            _ => {}
        }
        // Inherited stdio: on a terminal, polkit may ask for a password there.
        let enabled = Command::new("loginctl").arg("enable-linger").status()?;
        if enabled.success() {
            return Ok(());
        }
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "could not let your services run without a login (loginctl enable-linger); \
             run `sudo loginctl enable-linger $USER`, then try again",
        ))
    }

    /// Lets a running user manager see the change now; at boot it reads the files anyway.
    fn reload() {
        let _ = Command::new("systemctl")
            .args(["--user", "daemon-reload"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use std::fs;
    use std::io;
    use std::path::PathBuf;

    use super::{launchd_plist, launchd_program, remove_if_present};

    fn plist_path() -> io::Result<PathBuf> {
        std::env::home_dir()
            .map(|home| home.join("Library/LaunchAgents/dev.condr.server.plist"))
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is not set"))
    }

    pub(super) fn registered() -> io::Result<Option<PathBuf>> {
        let plist = match fs::read_to_string(plist_path()?) {
            Ok(plist) => plist,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        launchd_program(&plist).map(Some).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "dev.condr.server.plist names no program to start",
            )
        })
    }

    /// Only the file: `launchctl bootstrap` would run a `RunAtLoad` job at once.
    pub(super) fn enable(executable: &str) -> io::Result<()> {
        let path = plist_path()?;
        fs::create_dir_all(path.parent().expect("the plist is in a directory"))?;
        fs::write(path, launchd_plist(executable))
    }

    /// The job loaded at this login already ran; without the file it is not loaded again.
    pub(super) fn disable() -> io::Result<bool> {
        remove_if_present(&plist_path()?)
    }
}

#[cfg(windows)]
mod platform {
    use std::io;
    use std::path::{Path, PathBuf};

    use super::{run_value, run_value_program};

    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    /// Where Task Manager's Startup page and Settings › Apps › Startup record an item
    /// turned off, which Windows then skips although its `Run` value stays.
    const APPROVED_KEY: &str =
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
    const VALUE: &str = "Condr Server";
    /// `HRESULT_FROM_WIN32(ERROR_FILE_NOT_FOUND)`: no such key or value.
    const NOT_FOUND: i32 = 0x8007_0002_u32 as i32;

    /// Writable: `open` alone asks for read access, which cannot delete a value.
    fn writable(path: &str) -> windows_registry::Result<windows_registry::Key> {
        windows_registry::CURRENT_USER
            .options()
            .read()
            .write()
            .open(path)
    }

    /// An odd first byte (3, 7) is "disabled"; even (2, 6) is "enabled".
    fn turned_off_in_windows() -> bool {
        windows_registry::CURRENT_USER
            .open(APPROVED_KEY)
            .and_then(|key| key.get_value(VALUE))
            .is_ok_and(|value| value.first().is_some_and(|flag| flag & 1 == 1))
    }

    /// Removes `VALUE` under `path`; whether it was there.
    fn remove(path: &str) -> io::Result<bool> {
        match writable(path).and_then(|key| key.remove_value(VALUE)) {
            Ok(()) => Ok(true),
            Err(error) if error.code().0 == NOT_FOUND => Ok(false),
            Err(error) => Err(error.into()),
        }
    }

    pub(super) fn registered() -> io::Result<Option<PathBuf>> {
        if turned_off_in_windows() {
            return Ok(None);
        }
        let value = windows_registry::CURRENT_USER
            .open(RUN_KEY)
            .and_then(|key| key.get_string(VALUE));
        match value {
            Ok(value) => run_value_program(&value).map(Some).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("the {VALUE} login item names no program to start"),
                )
            }),
            Err(error) if error.code().0 == NOT_FOUND => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    pub(super) fn enable(executable: &str) -> io::Result<()> {
        let launcher = Path::new(executable).with_file_name("condrw.exe");
        if !launcher.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!(
                    "{} is missing, so Condr would open a console window at every login; \
                     reinstall Condr",
                    launcher.display()
                ),
            ));
        }
        windows_registry::CURRENT_USER
            .create(RUN_KEY)
            .and_then(|key| key.set_string(VALUE, run_value(executable)))?;
        // Turned off in Task Manager earlier, it would stay off without this.
        remove(APPROVED_KEY).map(drop)
    }

    pub(super) fn disable() -> io::Result<bool> {
        let enabled = !turned_off_in_windows();
        let removed = remove(RUN_KEY)?;
        remove(APPROVED_KEY)?;
        Ok(removed && enabled)
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
mod platform {
    use std::io;
    use std::path::PathBuf;

    fn unsupported() -> io::Error {
        io::Error::new(
            io::ErrorKind::Unsupported,
            "Autostart is available on Linux with systemd, macOS and Windows",
        )
    }

    pub(super) fn registered() -> io::Result<Option<PathBuf>> {
        Ok(None)
    }

    pub(super) fn enable(_: &str) -> io::Result<()> {
        Err(unsupported())
    }

    pub(super) fn disable() -> io::Result<bool> {
        Ok(false)
    }
}

/// A oneshot unit: systemd runs `start` once at boot and leaves the Server, which the
/// Supervisor of ADR 0058 keeps, to Condr. `RemainAfterExit` keeps the unit, and so the
/// Server in its cgroup, from being cleaned up once `start` returns. The `-` stops a
/// `start` that gave up waiting on a slow boot from failing the unit, which would take
/// the Server down with it.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn systemd_unit(executable: &str) -> String {
    let mut quoted = String::from('"');
    for character in executable.chars() {
        match character {
            '\\' | '"' => quoted.extend(['\\', character]),
            // Specifiers and environment variables, doubled to stay literal.
            '%' | '$' => quoted.extend([character, character]),
            _ => quoted.push(character),
        }
    }
    quoted.push('"');
    format!(
        "# Written by `condr server autostart on` (ADR 0057); `condr server autostart off` removes it.\n\
         [Unit]\n\
         Description=Condr Server\n\
         \n\
         [Service]\n\
         Type=oneshot\n\
         RemainAfterExit=yes\n\
         ExecStart=-{quoted} server start\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n"
    )
}

/// The program [`systemd_unit`] wrote into `ExecStart`.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn systemd_exec_path(unit: &str) -> Option<PathBuf> {
    let quoted = unit
        .lines()
        .find_map(|line| line.strip_prefix("ExecStart=-"))?
        .strip_prefix('"')?;
    let mut path = String::new();
    let mut characters = quoted.chars();
    while let Some(character) = characters.next() {
        match character {
            '"' => return Some(PathBuf::from(path)),
            '\\' => path.push(characters.next()?),
            '%' | '$' => {
                characters.next()?;
                path.push(character);
            }
            _ => path.push(character),
        }
    }
    None
}

/// `RunAtLoad` without `KeepAlive`: launchd runs `start` once at login. No
/// `LimitLoadToSessionType`, so the default, Aqua, keeps it to a login at the Mac.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn launchd_plist(executable: &str) -> String {
    let executable = executable
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<!-- Written by `condr server autostart on` (ADR 0057); `condr server autostart off` removes it. -->
<plist version="1.0">
<dict>
	<key>Label</key>
	<string>dev.condr.server</string>
	<key>ProgramArguments</key>
	<array>
		<string>{executable}</string>
		<string>server</string>
		<string>start</string>
	</array>
	<key>RunAtLoad</key>
	<true/>
</dict>
</plist>
"#
    )
}

/// The program [`launchd_plist`] wrote first among `ProgramArguments`.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn launchd_program(plist: &str) -> Option<PathBuf> {
    let (_, arguments) = plist.split_once("<key>ProgramArguments</key>")?;
    let (_, program) = arguments.split_once("<string>")?;
    let (program, _) = program.split_once("</string>")?;
    let program = program
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&");
    Some(PathBuf::from(program))
}

/// `condrw.exe` beside `executable`, which runs it without a console window. A Windows
/// path cannot hold `"`, so quoting it is all the escaping it needs.
#[cfg_attr(not(windows), allow(dead_code))]
fn run_value(executable: &str) -> String {
    let launcher = Path::new(executable).with_file_name("condrw.exe");
    format!("\"{}\" server start", launcher.display())
}

/// The `condr.exe` beside the `condrw.exe` that [`run_value`] wrote.
#[cfg_attr(not(windows), allow(dead_code))]
fn run_value_program(value: &str) -> Option<PathBuf> {
    let (launcher, _) = value.strip_prefix('"')?.split_once('"')?;
    Some(Path::new(launcher).with_file_name("condr.exe"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_systemd_unit_starts_the_server_once_and_reads_back_its_program() {
        let executable = r#"/home/a b/50%$HOME/"x"\condr"#;
        let unit = systemd_unit(executable);
        assert!(unit.contains(r#"ExecStart=-"/home/a b/50%%$$HOME/\"x\"\\condr" server start"#));
        for line in [
            "Type=oneshot",
            "RemainAfterExit=yes",
            "WantedBy=default.target",
        ] {
            assert!(unit.lines().any(|unit_line| unit_line == line), "{line}");
        }
        assert!(
            !unit.contains("Restart="),
            "the Supervisor restarts, not systemd"
        );
        assert_eq!(systemd_exec_path(&unit), Some(PathBuf::from(executable)));
        assert_eq!(systemd_exec_path("[Service]\nExecStart=condr"), None);
    }

    #[test]
    fn the_launch_agent_runs_at_load_only_and_reads_back_its_program() {
        let executable = "/Users/a&b/<condr>";
        let plist = launchd_plist(executable);
        assert!(plist.contains("<string>/Users/a&amp;b/&lt;condr&gt;</string>"));
        assert!(plist.contains("<key>RunAtLoad</key>\n\t<true/>"));
        assert!(
            !plist.contains("KeepAlive"),
            "the Supervisor restarts, not launchd"
        );
        assert_eq!(launchd_program(&plist), Some(PathBuf::from(executable)));
    }

    #[test]
    fn the_run_value_starts_condrw_beside_condr() {
        let executable = if cfg!(windows) {
            r"C:\Users\a b\AppData\Local\Programs\Condr\condr.exe"
        } else {
            "/opt/a b/condr.exe"
        };
        let value = run_value(executable);
        let launcher = Path::new(executable).with_file_name("condrw.exe");
        assert_eq!(value, format!("\"{}\" server start", launcher.display()));
        assert_eq!(run_value_program(&value), Some(PathBuf::from(executable)));
        assert_eq!(run_value_program("condrw.exe server start"), None);
    }
}
