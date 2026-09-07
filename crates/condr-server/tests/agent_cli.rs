#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use condr_core::protocol::{
    AgentCommand, AgentResponse, ClientMessage, ServerMessage, read_message, write_message,
};
use condr_server::{ClientConnection, Endpoint};
use serde_json::Value;

struct Server {
    child: Child,
    root: PathBuf,
    endpoint: Endpoint,
}

impl Server {
    fn start() -> Self {
        let root = std::env::temp_dir().join(format!("condr-agent-cli-{}", uuid::Uuid::new_v4()));
        let bin = root.join("bin directory");
        std::fs::create_dir_all(&bin).unwrap();
        let config = root.join("config/condr");
        std::fs::create_dir_all(&config).unwrap();
        let shell = if cfg!(windows) {
            "powershell.exe"
        } else {
            "/bin/sh"
        };
        std::fs::write(
            config.join("config.toml"),
            format!("[server.terminal]\nshell = '{shell}'\n"),
        )
        .unwrap();
        let executable = bin.join(if cfg!(windows) { "codex.exe" } else { "codex" });
        #[cfg(windows)]
        std::fs::copy(std::env::current_exe().unwrap(), &executable).unwrap();
        #[cfg(unix)]
        {
            std::fs::write(
                &executable,
                r#"#!/bin/sh
printf 'started\n' >> "$CONDR_TEST_STARTS"
printf '%s\n' "$@" > "$CONDR_TEST_ARGS"
report() { printf '{"session_id":"fixture","source":"startup"}' | condr agent-hook codex "$1"; }
case "$1" in
  --exit) exit 0 ;;
  --blocked) report session-start; report permission-request ;;
  --working) report prompt-submit ;;
  *) report session-start ;;
esac
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$CONDR_TEST_PROMPTS"
  [ "$line" = quit ] && exit 0
  report prompt-submit
  sleep 1
  report stop
done
"#,
            )
            .unwrap();
            std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        #[cfg(windows)]
        let path = std::env::join_paths(
            std::iter::once(bin).chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
        )
        .unwrap();
        #[cfg(unix)]
        let path =
            std::env::join_paths([bin, PathBuf::from("/usr/bin"), PathBuf::from("/bin")]).unwrap();
        let endpoint = Endpoint::local(root.join("server.sock"));
        let child = Command::new(env!("CARGO_BIN_EXE_condr"))
            .args(["server", "run", "--endpoint"])
            .arg(endpoint.as_local_path().unwrap())
            .arg("--snapshot")
            .arg(root.join("session.bin"))
            .env("CONDR_CONFIG_DIR", config)
            .env("PATH", path)
            .env("CONDR_TEST_STARTS", root.join("starts"))
            .env("CONDR_TEST_ARGS", root.join("args"))
            .env("CONDR_TEST_PROMPTS", root.join("prompts"))
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let server = Self {
            child,
            root,
            endpoint,
        };
        let deadline = Instant::now() + Duration::from_secs(5);
        while server.endpoint.connect().is_err() {
            assert!(Instant::now() < deadline, "Server failed to bind");
            thread::sleep(Duration::from_millis(20));
        }
        server
    }

    fn command(&self, args: &[&str]) -> Command {
        // The Windows fixture is this test executable renamed to codex.exe. Exact
        // test filters after `--` carry arbitrary native arguments to the helper.
        #[cfg(windows)]
        let args = if args.starts_with(&["agent", "start"]) {
            let split = args
                .iter()
                .position(|arg| *arg == "--")
                .unwrap_or(args.len());
            let mut launch = args[..split].to_vec();
            launch.extend([
                "--",
                "--exact",
                "windows_agent_fixture",
                "--nocapture",
                "--test-threads=1",
                "--",
            ]);
            launch.extend(args.get(split + 1..).unwrap_or_default());
            launch
        } else {
            args.to_vec()
        };
        let mut command = Command::new(env!("CARGO_BIN_EXE_condr"));
        command
            .args(args)
            .env("CONDR_SOCKET_PATH", self.endpoint.as_local_path().unwrap())
            .env_remove("CONDR_PANE_ID");
        command
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command(args).output().unwrap()
    }

    fn ok(&self, args: &[&str]) -> Value {
        let output = self.run(args);
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }

    fn err(&self, args: &[&str]) -> String {
        let output = self.run(args);
        assert_eq!(output.status.code(), Some(1), "{args:?}: {output:?}");
        serde_json::from_slice::<Value>(&output.stderr).unwrap()["error"]["code"]
            .as_str()
            .unwrap()
            .into()
    }

    fn pane(&self) -> String {
        self.ok(&["workspace", "create", "--cwd", self.root.to_str().unwrap()])["root_pane"]["pane_id"].as_u64().unwrap().to_string()
    }

    fn connect(&self) -> ClientConnection {
        ClientConnection::connect_overview(&self.endpoint, "agent-test").unwrap()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = condr_server::stop_server(&self.endpoint);
        let deadline = Instant::now() + Duration::from_secs(10);
        while self.child.try_wait().ok().flatten().is_none() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[cfg(windows)]
#[test]
fn windows_agent_fixture() {
    use std::io::{BufRead as _, Write as _};

    if std::env::current_exe().unwrap().file_name().unwrap() != "codex.exe" {
        return;
    }
    let append = |key: &str, text: &str| {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(std::env::var_os(key).unwrap())
            .unwrap();
        writeln!(file, "{text}").unwrap();
    };
    let report = |event: &str| {
        use std::process::{Command, Stdio};
        let mut hook = Command::new("condr")
            .args(["agent-hook", "codex", event])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        hook.stdin
            .take()
            .unwrap()
            .write_all(br#"{"session_id":"fixture","source":"startup"}"#)
            .unwrap();
        hook.wait().unwrap();
    };
    let args = std::env::args()
        .skip_while(|arg| arg != "--")
        .skip(1)
        .collect::<Vec<_>>();
    append("CONDR_TEST_STARTS", "started");
    std::fs::write(
        std::env::var_os("CONDR_TEST_ARGS").unwrap(),
        format!("{}\n", args.join("\n")),
    )
    .unwrap();
    match args.first().map(String::as_str) {
        Some("--exit") => return,
        Some("--blocked") => {
            report("session-start");
            report("permission-request");
        }
        Some("--working") => report("prompt-submit"),
        _ => report("session-start"),
    }
    for line in std::io::stdin().lock().lines() {
        let line = line.unwrap();
        append("CONDR_TEST_PROMPTS", &line);
        if line == "quit" {
            return;
        }
        report("prompt-submit");
        thread::sleep(Duration::from_secs(1));
        report("stop");
    }
}

#[test]
fn pane_cli_survives_a_child_shell_path_reset() {
    let server = Server::start();
    let pane = server.pane();
    let command = if cfg!(windows) {
        r#"$env:PATH = $env:SystemRoot; & $env:CONDR_BIN_PATH pane current | Out-File -Encoding ascii pane-current.json"#
    } else {
        r#"/bin/sh -lc 'PATH=/usr/bin:/bin; export PATH; "$CONDR_BIN_PATH" pane current > pane-current.json'"#
    };
    server.ok(&["pane", "run", &pane, command]);
    let deadline = Instant::now() + Duration::from_secs(5);
    let current = loop {
        if let Some(current) = std::fs::read(server.root.join("pane-current.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        {
            break current;
        }
        assert!(
            Instant::now() < deadline,
            "Pane CLI failed after PATH reset: {}",
            String::from_utf8_lossy(&server.run(&["pane", "read", &pane]).stdout)
        );
        thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(current["pane"]["pane_id"], pane.parse::<u64>().unwrap());
}

#[test]
fn discovers_on_server_starts_once_and_prompts_by_name() {
    let server = Server::start();
    let agents = server.ok(&["agent", "available"]);
    let codex = agents["agents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|agent| agent["agent"] == "codex")
        .unwrap();
    assert_eq!(
        codex["executable"],
        server
            .root
            .join("bin directory")
            .join(if cfg!(windows) { "codex.exe" } else { "codex" })
            .to_str()
            .unwrap()
    );
    let pane = server.pane();
    let args = [
        "",
        "one two",
        "a'b\"c",
        "$(touch injected); $HOME",
        "trailing\\",
    ];
    let mut launch = vec![
        "agent", "start", "worker", "--kind", "codex", "--pane", &pane, "--",
    ];
    launch.extend(args);
    let pending = server
        .command(&launch)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let listed = server.ok(&["agent", "list"]);
        if !listed["agents"].as_array().unwrap().is_empty() {
            assert_eq!(listed["agents"][0]["launch_pending"], true);
            break;
        }
        assert!(Instant::now() < deadline, "launch name was not reserved");
        thread::sleep(Duration::from_millis(20));
    }
    let rejected = server
        .connect()
        .agent(AgentCommand::Start {
            name: "competitor".into(),
            kind: condr_core::AgentKind::Codex,
            pane_id: condr_core::PaneId::from_u64(pane.parse().unwrap()),
            args: Vec::new(),
            timeout_ms: 30_000,
        })
        .unwrap()
        .unwrap_err();
    assert_eq!(rejected.code, "pane_busy");
    let output = pending.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let started: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(started["agent"]["name"], "worker");
    assert_eq!(started["agent"]["agent_status"], "idle");
    assert_eq!(
        std::fs::read_to_string(server.root.join("args")).unwrap(),
        format!("{}\n", args.join("\n"))
    );
    assert!(!server.root.join("injected").exists());
    // New CLI connections retain the same runtime name, and never start twice.
    assert_eq!(server.ok(&["agent", "list"])["agents"][0]["name"], "worker");
    assert_eq!(
        server.err(&[
            "agent", "start", "worker", "--kind", "codex", "--pane", &pane
        ]),
        "agent_name_in_use"
    );
    assert_eq!(
        server.err(&[
            "agent", "start", "second", "--kind", "codex", "--pane", &pane
        ]),
        "pane_busy"
    );
    assert_eq!(
        std::fs::read_to_string(server.root.join("starts")).unwrap(),
        "started\n"
    );
    assert_eq!(
        server.err(&[
            "agent",
            "prompt",
            "worker",
            "do not send",
            "--wait",
            "--until",
            "typo"
        ]),
        "invalid_agent_state"
    );
    assert!(!server.root.join("prompts").exists());
    let prompted = server.ok(&[
        "agent",
        "prompt",
        "worker",
        "hello",
        "--wait",
        "--timeout",
        "8000",
    ]);
    assert_eq!(prompted["agent"]["agent_status"], "idle");
    assert_eq!(
        std::fs::read_to_string(server.root.join("prompts")).unwrap(),
        "hello\n"
    );
    assert_eq!(
        server.ok(&["agent", "wait", &pane, "--timeout", "1000"])["agent"]["name"],
        "worker"
    );
    assert_eq!(
        server.err(&["agent", "wait", "codex", "--timeout", "1"]),
        "agent_not_found"
    );
    assert_eq!(
        server.err(&[
            "agent",
            "wait",
            "worker",
            "--until",
            "blocked",
            "--timeout",
            "50"
        ]),
        "agent_timeout"
    );
    assert_eq!(
        server.err(&[
            "agent",
            "prompt",
            "worker",
            "quit",
            "--wait",
            "--timeout",
            "8000"
        ]),
        "agent_not_running",
        "{}",
        String::from_utf8_lossy(&server.run(&["pane", "read", &pane]).stdout)
    );
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        if server.ok(&["agent", "list"])["agents"]
            .as_array()
            .unwrap()
            .is_empty()
        {
            break;
        }
        assert!(Instant::now() < deadline, "exited agent retained its name");
        thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(server.ok(&launch)["agent"]["name"], "worker");
}

#[test]
fn blocked_timeout_exit_and_disconnect_do_not_report_false_readiness() {
    let server = Server::start();
    let pane = server.pane();
    assert_eq!(
        server.err(&[
            "agent",
            "start",
            "blocked",
            "--kind",
            "codex",
            "--pane",
            &pane,
            "--",
            "--blocked"
        ]),
        "agent_not_ready"
    );
    assert_eq!(
        server.ok(&["agent", "list"])["agents"][0]["name"],
        "blocked"
    );
    assert_eq!(
        server.err(&["agent", "prompt", "blocked", "do not send"]),
        "agent_not_ready"
    );
    assert!(!server.root.join("prompts").exists());

    let client = server.connect();
    let overview = client.overview().clone();
    let mut stream = client.into_stream();
    stream
        .set_handshake_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    write_message(
        &mut stream,
        &ClientMessage::Agent {
            server_id: overview.server_id,
            session_id: overview.session_id,
            command: AgentCommand::Wait {
                target: "blocked".into(),
                until: vec![condr_core::AgentState::Idle],
                timeout_ms: 300_000,
            },
        },
    )
    .unwrap();
    // The reader is free to service other messages while a wait is pending.
    write_message(
        &mut stream,
        &ClientMessage::Ping {
            server_id: overview.server_id,
            nonce: 987,
        },
    )
    .unwrap();
    assert!(matches!(
        read_message::<_, ServerMessage>(&mut stream).unwrap(),
        ServerMessage::Pong { nonce: 987, .. }
    ));
    drop(stream);
    assert!(matches!(
        server.connect().agent(AgentCommand::List).unwrap().unwrap(),
        AgentResponse::List(_)
    ));

    let working = server.pane();
    assert_eq!(
        server.err(&[
            "agent",
            "start",
            "working",
            "--kind",
            "codex",
            "--pane",
            &working,
            "--timeout",
            "4500",
            "--",
            "--working"
        ]),
        "agent_timeout"
    );
    let exiting = server.pane();
    let failure = server.err(&[
        "agent",
        "start",
        "exiting",
        "--kind",
        "codex",
        "--pane",
        &exiting,
        "--timeout",
        "4500",
        "--",
        "--exit",
    ]);
    assert!(["agent_timeout", "agent_not_running"].contains(&failure.as_str()));
    assert!(
        server.ok(&["agent", "list"])["agents"]
            .as_array()
            .unwrap()
            .iter()
            .all(|agent| agent["name"] != "exiting")
    );
}
