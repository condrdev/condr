//! Client-side OpenSSH transport. The remote command only forwards the local socket.

mod stream;

use std::fmt;
use std::io::{self, PipeReader, PipeWriter, Read, Write};
#[cfg(unix)]
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};
#[cfg(windows)]
use windows_spawn::{Child, Command, CreationFlags, Job, SpawnOptions, Stdio};

pub use stream::SshStream;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SshEndpoint {
    destination: String,
    port: Option<u16>,
    binary: Option<String>,
}

impl SshEndpoint {
    /// `ssh://[user@]host[:port][?bin=/absolute/path]`, including SSH config aliases
    /// and bracketed IPv6. The optional binary is a URI-encoded remote file path.
    pub fn parse(text: &str) -> io::Result<Self> {
        let invalid = || {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "expected ssh://[user@]host[:port][?bin=/absolute/path]",
            )
        };
        let authority = text.trim().strip_prefix("ssh://").ok_or_else(invalid)?;
        let (authority, binary) = match authority.split_once('?') {
            Some((authority, query)) => {
                let encoded = query
                    .strip_prefix("bin=")
                    .filter(|value| !value.contains(['&', '#']))
                    .ok_or_else(invalid)?;
                let path = condr_core::uri::percent_decode(encoded)
                    .filter(|path| path.starts_with('/') && !path.chars().any(char::is_control))
                    .ok_or_else(|| {
                        io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "bin must be an absolute remote path without control characters",
                        )
                    })?;
                (authority, Some(path))
            }
            None => (authority, None),
        };
        let (user, host_port) = authority
            .rsplit_once('@')
            .map_or((None, authority), |(user, host)| (Some(user), host));
        let valid_name = |name: &str| {
            !name.is_empty()
                && !name.starts_with('-')
                && name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        };
        if user.is_some_and(|user| !valid_name(user)) {
            return Err(invalid());
        }
        let (host, port) = if let Some(ipv6) = host_port.strip_prefix('[') {
            let (ip, suffix) = ipv6.split_once(']').ok_or_else(invalid)?;
            let ip = ip.parse::<std::net::Ipv6Addr>().map_err(|_| invalid())?;
            let port = if suffix.is_empty() {
                None
            } else {
                Some(suffix.strip_prefix(':').ok_or_else(invalid)?)
            };
            (format!("[{ip}]"), port)
        } else {
            let (host, port) = host_port
                .split_once(':')
                .map_or((host_port, None), |(host, port)| (host, Some(port)));
            if !valid_name(host) {
                return Err(invalid());
            }
            (host.to_owned(), port)
        };
        let port = port
            .map(|port| {
                port.parse::<u16>()
                    .ok()
                    .filter(|p| *p != 0)
                    .ok_or_else(invalid)
            })
            .transpose()?;
        Ok(Self {
            destination: user.map_or_else(|| host.clone(), |user| format!("{user}@{host}")),
            port,
            binary,
        })
    }

    pub fn destination(&self) -> &str {
        &self.destination
    }
    pub fn port(&self) -> Option<u16> {
        self.port
    }

    pub fn binary(&self) -> &str {
        self.binary.as_deref().unwrap_or("condr")
    }

    fn remote_command(&self) -> String {
        // The remote login shell parses this string. Only the executable is variable;
        // single-quote it as one POSIX shell word, including embedded quotes.
        format!(
            "exec '{}' server bridge",
            self.binary().replace('\'', "'\"'\"'")
        )
    }

    fn command(&self) -> Command {
        let mut command = Command::new("ssh");
        self.configure(&mut command);
        command
    }

    fn configure(&self, command: &mut Command) {
        command
            .args([
                "-T",
                "-o",
                "BatchMode=yes",
                "-o",
                "RemoteCommand=none",
                "-o",
                "SessionType=default",
                "-o",
                "StdinNull=no",
                "-o",
                "ForkAfterAuthentication=no",
                "-o",
                "ClearAllForwardings=yes",
                "-o",
                "ConnectTimeout=10",
                "-o",
                "ServerAliveInterval=15",
                "-o",
                "ServerAliveCountMax=3",
            ])
            .arg(format!(
                "ssh://{}{}",
                self.destination,
                self.port.map(|port| format!(":{port}")).unwrap_or_default()
            ))
            .arg(self.remote_command());
    }

    pub(crate) fn connect(&self) -> io::Result<SshStream> {
        SshStream::spawn(self.command())
    }
}

impl fmt::Display for SshEndpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ssh://{}", self.destination)?;
        if let Some(port) = self.port {
            write!(f, ":{port}")?;
        }
        if let Some(binary) = &self.binary {
            f.write_str("?bin=")?;
            for byte in binary.bytes() {
                if byte.is_ascii_alphanumeric() || b"/._-~".contains(&byte) {
                    write!(f, "{}", char::from(byte))?;
                } else {
                    write!(f, "%{byte:02X}")?;
                }
            }
        }
        Ok(())
    }
}

/// Run only in the bridge CLI process: once either direction ends, `main` exits and
/// closes the other direction too, even if it is blocked on stdin or a local pipe.
pub fn bridge(path: &std::path::Path) -> io::Result<()> {
    // On the Server's own socket the bridge behaves like the GUI on its own machine:
    // nobody listening means start the Server, not fail. Any other path was chosen on
    // purpose and is only connected to.
    let endpoint = if path == crate::default_socket_path() {
        crate::ensure_local_server()?
    } else {
        crate::Endpoint::local(path)
    };
    let mut reader = endpoint.connect().map_err(|error| {
        io::Error::other(format!(
            "{}; start the remote Server with `condr server start`",
            endpoint.describe_connect_error(&error)
        ))
    })?;
    let mut writer = reader.try_clone()?;
    let (done, result) = mpsc::channel();
    let input_done = done.clone();
    thread::Builder::new()
        .name("condr-bridge-stdin".into())
        .spawn(move || {
            let _ = input_done.send(io::copy(&mut io::stdin().lock(), &mut writer).map(|_| ()));
        })?;
    thread::Builder::new()
        .name("condr-bridge-stdout".into())
        .spawn(move || {
            let _ = done.send((|| {
                let mut stdout = io::stdout().lock();
                let mut buffer = [0; 64 * 1024];
                loop {
                    let count = reader.read(&mut buffer)?;
                    if count == 0 {
                        return Ok(());
                    }
                    stdout.write_all(&buffer[..count])?;
                    stdout.flush()?;
                }
            })());
        })?;
    result.recv().map_err(io::Error::other)?
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn binary_is_one_literal_shell_word() {
        use std::os::unix::fs::PermissionsExt;
        let directory = std::env::temp_dir().join(format!("condr-ssh-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let binary = directory.join("a 'quoted' \"$(echo injected)\"; binary");
        std::fs::write(&binary, "#!/bin/sh\nprintf '%s\\n' \"$0\" \"$@\"\n").unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        let endpoint = SshEndpoint::parse(&format!("ssh://host?bin={}", binary.display())).unwrap();
        let output = Command::new("sh")
            .args(["-c", &endpoint.remote_command()])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            format!("{}\nserver\nbridge\n", binary.display())
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn ssh_config_preserves_routes_but_cannot_replace_the_bridge_session() {
        let path = std::env::temp_dir().join(format!("condr-ssh-config-{}", uuid::Uuid::new_v4()));
        std::fs::write(&path, "Host review-target\n HostName example.invalid\n User configured-user\n Port 2222\n IdentityFile /configured/key\n ProxyJump jump-host\n StrictHostKeyChecking yes\n RemoteCommand tmux\n SessionType none\n StdinNull yes\n ForkAfterAuthentication yes\n LocalForward 8080 localhost:80\n RemoteForward 9090 localhost:90\n DynamicForward 1080\n ControlMaster auto\n ControlPersist 60\n").unwrap();
        let mut command = Command::new("ssh");
        command.arg("-G").arg("-F").arg(&path);
        SshEndpoint::parse("ssh://review-target")
            .unwrap()
            .configure(&mut command);
        let output = command
            .output()
            .expect("system OpenSSH is required for this check");
        std::fs::remove_file(path).unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let config = String::from_utf8(output.stdout).unwrap();
        for expected in [
            "hostname example.invalid",
            "user configured-user",
            "port 2222",
            "identityfile /configured/key",
            "proxyjump jump-host",
            "stricthostkeychecking true",
            "sessiontype default",
            "stdinnull no",
            "forkafterauthentication no",
            "clearallforwardings yes",
            "requesttty false",
            "controlmaster auto",
            "controlpersist 60",
        ] {
            assert!(
                config.lines().any(|line| line == expected),
                "missing {expected}: {config}"
            );
        }
        assert!(!config.lines().any(|line| line.starts_with("localforward ")
            || line.starts_with("remoteforward ")
            || line.starts_with("dynamicforward ")
            || line == "remotecommand tmux"));
    }
}
