//! Client-side OpenSSH transport. The remote command only forwards the local socket.

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

/// Native pipes keep terminal bytes out of an extra application queue. All clones
/// share the child; shutdown interrupts both directions and the last drop reaps it.
pub struct SshStream {
    reader: PipeReader,
    writer: PipeWriter,
    process: Arc<SshProcess>,
}

#[derive(Default)]
struct Deadline {
    idle: Option<Duration>,
    expires: Option<Instant>,
    stopped: bool,
}

struct SshProcess {
    // Take once before reaping, so a later shutdown never signals a reused PID.
    child: Mutex<Option<Child>>,
    #[cfg(windows)]
    job: Job,
    stderr: Arc<Mutex<Vec<u8>>>,
    stderr_done: Mutex<mpsc::Receiver<()>>,
    timed_out: AtomicBool,
    deadline: Arc<(Mutex<Deadline>, Condvar)>,
}

impl SshProcess {
    fn shutdown(&self) -> io::Result<()> {
        self.deadline.0.lock().unwrap().stopped = true;
        self.deadline.1.notify_one();
        let mut owned = self.child.lock().unwrap();
        let Some(mut child) = owned.take() else {
            return Ok(());
        };
        // ProxyCommand/ProxyJump helpers may inherit our pipes. Terminate the owned
        // process group/Job, without touching a pre-existing shared SSH master.
        #[cfg(unix)]
        {
            use nix::{
                sys::signal::{Signal, killpg},
                unistd::Pid,
            };
            let _ = killpg(Pid::from_raw(child.id() as i32), Signal::SIGKILL);
        }
        #[cfg(windows)]
        self.job.terminate(1)?;
        child.wait().map(|_| ())
    }

    fn error(&self, fallback: &str) -> io::Error {
        let _ = self.shutdown();
        // An external helper can retain stderr even after SSH exits. Give the drain
        // a bounded chance to finish; EOF is never a prerequisite for reporting failure.
        let _ = self
            .stderr_done
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_millis(50));
        let stderr = self.stderr.lock().unwrap();
        let detail = String::from_utf8_lossy(&stderr);
        let kind = if self.timed_out.load(Ordering::Acquire) {
            io::ErrorKind::TimedOut
        } else {
            io::ErrorKind::Other
        };
        io::Error::new(
            kind,
            if detail.trim().is_empty() {
                fallback.to_owned()
            } else {
                format!("{fallback}: {}", detail.trim())
            },
        )
    }

    fn progress(&self) {
        let mut deadline = self.deadline.0.lock().unwrap();
        deadline.expires = deadline.idle.map(|idle| Instant::now() + idle);
    }
}

impl Drop for SshProcess {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

impl SshStream {
    fn spawn(mut command: Command) -> io::Result<Self> {
        let (reader, stdout) = io::pipe()?;
        let (stdin, writer) = io::pipe()?;
        #[cfg(unix)]
        let spawned = {
            use std::os::unix::process::CommandExt;
            command
                .process_group(0)
                .stdin(stdin)
                .stdout(stdout)
                .stderr(Stdio::piped())
                .spawn()
        };
        #[cfg(windows)]
        let job = Job::create()?;
        #[cfg(windows)]
        let spawned = {
            use std::os::windows::io::OwnedHandle;
            job.set_kill_on_close(true)?;
            command
                .stdin(OwnedHandle::from(stdin))
                .stdout(OwnedHandle::from(stdout))
                .stderr(Stdio::piped())
                .spawn_with(
                    SpawnOptions::new()
                        .job(&job)
                        .creation_flags(CreationFlags::NO_WINDOW),
                )
        };
        let mut child = spawned.map_err(|error| {
            io::Error::new(error.kind(), format!("could not start SSH: {error}"))
        })?;
        // Command retains its configured pipe ends after spawn; close them for EOF.
        drop(command);
        let mut stderr = child.stderr.take().expect("SSH stderr is piped");
        let (stderr_finished, stderr_done) = mpsc::channel();
        let process = Arc::new(SshProcess {
            child: Mutex::new(Some(child)),
            #[cfg(windows)]
            job,
            stderr: Arc::default(),
            stderr_done: Mutex::new(stderr_done),
            timed_out: AtomicBool::new(false),
            deadline: Arc::default(),
        });
        let diagnostics = Arc::clone(&process.stderr);
        thread::Builder::new()
            .name("condr-ssh-stderr".into())
            .spawn(move || {
                let mut chunk = [0; 4096];
                while let Ok(count) = stderr.read(&mut chunk) {
                    if count == 0 {
                        break;
                    }
                    let mut text = diagnostics.lock().unwrap();
                    text.extend_from_slice(&chunk[..count]);
                    let excess = text.len().saturating_sub(8192);
                    text.drain(..excess);
                }
                let _ = stderr_finished.send(());
            })?;
        let weak = Arc::downgrade(&process);
        let deadline = Arc::clone(&process.deadline);
        // Native pipes have no portable timeout. Watch inactivity during bootstrap;
        // progress updates one deadline instead of queuing per-read wakeups.
        thread::Builder::new()
            .name("condr-ssh-handshake".into())
            .spawn(move || {
                let (state, changed) = &*deadline;
                let mut state = state.lock().unwrap();
                while !state.stopped {
                    match state.expires {
                        Some(expires) if expires <= Instant::now() => {
                            state.stopped = true;
                            if let Some(process) = weak.upgrade() {
                                process.timed_out.store(true, Ordering::Release);
                                drop(state);
                                let _ = process.shutdown();
                            }
                            return;
                        }
                        Some(expires) => {
                            state = changed
                                .wait_timeout(
                                    state,
                                    expires.saturating_duration_since(Instant::now()),
                                )
                                .unwrap()
                                .0;
                        }
                        None => state = changed.wait(state).unwrap(),
                    }
                }
            })?;
        Ok(Self {
            reader,
            writer,
            process,
        })
    }

    pub(crate) fn try_clone(&self) -> io::Result<Self> {
        Ok(Self {
            reader: self.reader.try_clone()?,
            writer: self.writer.try_clone()?,
            process: Arc::clone(&self.process),
        })
    }

    pub(crate) fn shutdown(&self) -> io::Result<()> {
        self.process.shutdown()
    }

    pub(crate) fn set_handshake_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        let mut deadline = self.process.deadline.0.lock().unwrap();
        if deadline.stopped {
            drop(deadline);
            return Err(self.process.error("SSH connection closed"));
        }
        deadline.idle = timeout;
        deadline.expires = timeout.map(|idle| Instant::now() + idle);
        self.process.deadline.1.notify_one();
        Ok(())
    }
}

impl Read for SshStream {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        match self.reader.read(buffer) {
            Ok(0) => Err(self.process.error("SSH connection closed")),
            Err(error) => Err(self.process.error(&format!("SSH read failed: {error}"))),
            Ok(count) => {
                self.process.progress();
                Ok(count)
            }
        }
    }
}

impl Write for SshStream {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.writer
            .write(buffer)
            .map_err(|error| self.process.error(&format!("SSH write failed: {error}")))
    }
    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}

/// Run only in the bridge CLI process: once either direction ends, `main` exits and
/// closes the other direction too, even if it is blocked on stdin or a local pipe.
pub fn bridge(path: &std::path::Path) -> io::Result<()> {
    let endpoint = crate::Endpoint::local(path);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_round_trip_and_reject_options_commands_and_invalid_paths() {
        for address in [
            "ssh://build-box",
            "ssh://alice@host:2222",
            "ssh://user@[::1]:22",
            "ssh://host?bin=/opt/example/condr",
            "ssh://host?bin=/opt/a%20b%27%24%28x%29/condr",
            "ssh://host?bin=/opt/%E4%B8%AD%E6%96%87/condr",
        ] {
            let endpoint = SshEndpoint::parse(address).unwrap();
            assert_eq!(endpoint.to_string(), address);
            assert_eq!(SshEndpoint::parse(&endpoint.to_string()).unwrap(), endpoint);
            assert!(
                crate::Endpoint::parse(address, None)
                    .unwrap()
                    .as_local_path()
                    .is_none()
            );
        }
        assert_eq!(SshEndpoint::parse("ssh://host").unwrap().binary(), "condr");
        assert_eq!(
            SshEndpoint::parse("ssh://host?bin=/opt/a+b/condr")
                .unwrap()
                .binary(),
            "/opt/a+b/condr"
        );
        for address in [
            "host",
            "tcp://host",
            "ssh://",
            "ssh://-oProxyCommand=x",
            "ssh://user@-host",
            "ssh://-user@host",
            "ssh://user@host;touch",
            "ssh://user@host/path",
            "ssh://a@b@host",
            "ssh://host:0",
            "ssh://host:65536",
            "ssh://[broken]",
            "ssh://::1",
            "ssh://host?bin=",
            "ssh://host?bin=relative",
            "ssh://host?bin=/a&bin=/b",
            "ssh://host?command=id",
            "ssh://host?bin=/a#fragment",
            "ssh://host?bin=/a%",
            "ssh://host?bin=/a%FF",
            "ssh://host?bin=/a%00",
            "ssh://host?bin=/a%0A",
            "ssh://host\ncommand",
        ] {
            assert!(SshEndpoint::parse(address).is_err(), "{address}");
        }
        let key = crate::StaticKey::from_private([7; 32]);
        let tcp = format!("tcp://{}@localhost:4242", key.public());
        assert!(crate::Endpoint::parse(&tcp, None).is_err());
        assert!(matches!(
            crate::Endpoint::parse(&tcp, Some(&key)).unwrap(),
            crate::Endpoint::Tcp(_)
        ));
        assert!(crate::Endpoint::parse("https://host", Some(&key)).is_err());
    }

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

    #[cfg(unix)]
    #[test]
    fn timeout_reaps_proxy_helpers_without_waiting_for_inherited_stderr() {
        let mut command = Command::new("ssh");
        command.args([
            "-F",
            "/dev/null",
            "-o",
            "ProxyCommand=sh -c 'printf proxy-ready >&2; sleep 30'",
        ]);
        SshEndpoint::parse("ssh://review-target")
            .unwrap()
            .configure(&mut command);
        let mut stream = SshStream::spawn(command).unwrap();
        let process = Arc::clone(&stream.process);
        let ready_deadline = Instant::now() + Duration::from_secs(2);
        while !String::from_utf8_lossy(&process.stderr.lock().unwrap()).contains("proxy-ready") {
            assert!(
                Instant::now() < ready_deadline,
                "ProxyCommand did not start"
            );
            thread::sleep(Duration::from_millis(5));
        }
        stream
            .set_handshake_timeout(Some(Duration::from_millis(100)))
            .unwrap();
        let (sent, received) = mpsc::channel();
        let read = thread::spawn(move || sent.send(stream.read(&mut [0]).unwrap_err()).unwrap());
        let error = received
            .recv_timeout(Duration::from_secs(2))
            .expect("timeout must not join a live stderr reader");
        read.join().unwrap();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(error.to_string().contains("proxy-ready"));
        assert!(process.child.lock().unwrap().is_none());
        assert!(
            matches!(
                process.stderr_done.lock().unwrap().try_recv(),
                Err(mpsc::TryRecvError::Disconnected)
            ),
            "owned proxy helpers must release stderr"
        );
    }

    #[cfg(unix)]
    #[test]
    fn progress_renews_the_idle_timeout_and_cancellation_interrupts_blocked_io() {
        use crate::{ConnectionCancellation, EndpointStream};
        let mut stream = SshStream::spawn(Command::new("cat")).unwrap();
        stream
            .set_handshake_timeout(Some(Duration::from_millis(300)))
            .unwrap();
        for _ in 0..8 {
            thread::sleep(Duration::from_millis(70));
            stream.write_all(b"x").unwrap();
            stream.read_exact(&mut [0]).unwrap();
        }
        stream.shutdown().unwrap();

        // A cancelled attempt cannot attach a child that finished spawning later.
        let cancellation = ConnectionCancellation::default();
        cancellation.cancel();
        let late = EndpointStream::Ssh(SshStream::spawn(Command::new("cat")).unwrap());
        assert_eq!(
            cancellation.attach(&late).unwrap_err().kind(),
            io::ErrorKind::Interrupted
        );
        let EndpointStream::Ssh(late) = late else {
            unreachable!()
        };
        assert!(late.process.child.lock().unwrap().is_none());

        // The child keeps stdin open but never drains it. Cancelling must bypass
        // write_all and interrupt both the writer and an in-flight Welcome read.
        let mut command = Command::new("sh");
        command.args(["-c", "printf ready; exec sleep 30"]);
        let mut stream = EndpointStream::Ssh(SshStream::spawn(command).unwrap());
        stream.read_exact(&mut [0; 5]).unwrap();
        let cancellation = ConnectionCancellation::default();
        cancellation.attach(&stream).unwrap();
        let mut writer = stream.try_clone().unwrap();
        let (sent, received) = mpsc::channel();
        let writer_done = sent.clone();
        let write = thread::spawn(move || {
            writer_done
                .send(writer.write_all(&vec![0; 16 * 1024 * 1024]).is_err())
                .unwrap()
        });
        let read = thread::spawn(move || sent.send(stream.read(&mut [0]).is_err()).unwrap());
        assert!(received.recv_timeout(Duration::from_millis(100)).is_err());
        cancellation.cancel();
        for _ in 0..2 {
            assert!(
                received
                    .recv_timeout(Duration::from_secs(2))
                    .expect("cancelled I/O must end")
            );
        }
        write.join().unwrap();
        read.join().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn pipes_are_duplex_clonable_cancellable_and_report_stderr() {
        let mut stream = SshStream::spawn(Command::new("cat")).unwrap();
        let process = Arc::clone(&stream.process);
        stream
            .set_handshake_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let bytes: Vec<u8> = (0..256 * 1024).map(|i| i as u8).collect();
        let mut writer = stream.try_clone().unwrap();
        let sent = bytes.clone();
        let write = thread::spawn(move || writer.write_all(&sent).unwrap());
        let mut received = vec![0; bytes.len()];
        stream.read_exact(&mut received).unwrap();
        write.join().unwrap();
        assert_eq!(received, bytes);
        stream
            .set_handshake_timeout(Some(Duration::from_millis(20)))
            .unwrap();
        stream.set_handshake_timeout(None).unwrap();
        thread::sleep(Duration::from_millis(40));
        stream.write_all(b"alive").unwrap();
        let mut alive = [0; 5];
        stream.read_exact(&mut alive).unwrap();
        assert_eq!(&alive, b"alive");
        let cancel = stream.try_clone().unwrap();
        let read = thread::spawn(move || stream.read(&mut [0]).unwrap_err());
        cancel.shutdown().unwrap();
        assert!(process.child.lock().unwrap().is_none());
        assert!(read.join().unwrap().to_string().contains("SSH"));

        let mut stalled = SshStream::spawn(Command::new("cat")).unwrap();
        stalled
            .set_handshake_timeout(Some(Duration::from_millis(20)))
            .unwrap();
        assert_eq!(
            stalled.read(&mut [0]).unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );

        let mut command = Command::new("sh");
        command.args(["-c", "i=0; while [ $i -lt 2000 ]; do printf 'diagnostic '; i=$((i+1)); done >&2; printf 'Permission denied' >&2; exit 255"]);
        let mut failed = SshStream::spawn(command).unwrap();
        let error = failed.read(&mut [0]).unwrap_err().to_string();
        assert!(error.contains("Permission denied"), "{error}");
        assert!(error.len() < 8300);

        let dropped = SshStream::spawn(Command::new("cat")).unwrap();
        let pid = dropped.process.child.lock().unwrap().as_ref().unwrap().id();
        drop(dropped);
        assert_eq!(
            nix::sys::wait::waitpid(
                nix::unistd::Pid::from_raw(pid as i32),
                Some(nix::sys::wait::WaitPidFlag::WNOHANG)
            )
            .unwrap_err(),
            nix::errno::Errno::ECHILD
        );
    }
}
