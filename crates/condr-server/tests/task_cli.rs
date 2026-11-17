//! `condr task …` against a real Server (ADR 0068), the way a program in a Pane runs it:
//! JSON on stdout, `.error.code` on stderr with exit 1. Every Server keeps its Tasks in a
//! temporary data directory.

use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use condr_server::{BoundServer, Endpoint, ServerConfig};
use serde_json::Value;

mod common;
use common::unique_suffix;

/// `condr server run` in a temporary directory, with a `claude` on its PATH that writes
/// the arguments it was started with to `<root>/args`.
struct Server {
    child: Child,
    root: PathBuf,
    endpoint: Endpoint,
}

impl Server {
    fn start() -> Self {
        let root = std::env::temp_dir().join(format!("condr-task-cli-{}", unique_suffix()));
        let bin = root.join("bin");
        let config = root.join("config");
        std::fs::create_dir_all(&bin).unwrap();
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
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let claude = bin.join("claude");
            std::fs::write(
                &claude,
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$CONDR_TEST_ARGS\"\n",
            )
            .unwrap();
            std::fs::set_permissions(&claude, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        #[cfg(unix)]
        let path = std::env::join_paths([bin, "/usr/bin".into(), "/bin".into()]).unwrap();
        #[cfg(windows)]
        let path = std::env::join_paths(
            std::iter::once(bin).chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
        )
        .unwrap();
        let endpoint = Endpoint::local(root.join("server.sock"));
        let child = Command::new(env!("CARGO_BIN_EXE_condr"))
            .args(["server", "run", "--endpoint"])
            .arg(endpoint.as_local_path().unwrap())
            .arg("--snapshot")
            .arg(root.join("session.bin"))
            .env("CONDR_CONFIG_DIR", &config)
            .env("CONDR_DATA_DIR", root.join("data"))
            .env("CONDR_LOG_DIR", &config)
            .env("PATH", path)
            .env("CONDR_TEST_ARGS", root.join("args"))
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
        let mut command = Command::new(env!("CARGO_BIN_EXE_condr"));
        command
            .args(args)
            .env("CONDR_SOCKET_PATH", self.endpoint.as_local_path().unwrap())
            .env_remove("CONDR_PANE_ID")
            .env_remove("CONDR_DEVICE");
        command
    }

    fn ok(&self, args: &[&str]) -> Value {
        ok(self.command(args).output().unwrap(), args)
    }

    fn err(&self, args: &[&str]) -> String {
        err(self.command(args).output().unwrap(), args)
    }

    /// Run as a program in Pane `pane` runs it.
    #[cfg_attr(not(unix), allow(dead_code))]
    fn in_pane(&self, pane: u64, args: &[&str]) -> Output {
        self.command(args)
            .env("CONDR_PANE_ID", pane.to_string())
            .output()
            .unwrap()
    }

    /// A Workspace on a new directory under the root, and its first Pane.
    #[cfg_attr(not(unix), allow(dead_code))]
    fn workspace(&self, name: &str) -> (u64, u64) {
        let directory = self.root.join(name);
        std::fs::create_dir_all(&directory).unwrap();
        let created = self.ok(&["workspace", "create", "--cwd", directory.to_str().unwrap()]);
        (
            created["workspace"]["workspace_id"].as_u64().unwrap(),
            created["root_pane"]["pane_id"].as_u64().unwrap(),
        )
    }

    /// What the last Agent was started with, once it has run.
    #[cfg(unix)]
    fn agent_args(&self) -> Vec<String> {
        let file = self.root.join("args");
        let text = wait_for(|| {
            std::fs::read_to_string(&file)
                .ok()
                .filter(|text| !text.is_empty())
        });
        std::fs::remove_file(file).unwrap();
        text.lines().map(str::to_owned).collect()
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

fn ok(output: Output, args: &[&str]) -> Value {
    assert!(
        output.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

/// Exit 1, nothing on stdout, and the error's code.
fn err(output: Output, args: &[&str]) -> String {
    assert_eq!(output.status.code(), Some(1), "{args:?}: {output:?}");
    assert!(output.stdout.is_empty(), "{args:?}");
    let error: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert!(error["error"]["message"].is_string(), "{error}");
    error["error"]["code"].as_str().unwrap().into()
}

#[cfg_attr(not(unix), allow(dead_code))]
fn wait_for<T>(mut done: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(value) = done() {
            return value;
        }
        assert!(Instant::now() < deadline, "timed out");
        thread::sleep(Duration::from_millis(50));
    }
}

/// A Task's id as the CLI prints it: its UUIDv7, hyphenated.
fn id(task: &Value) -> String {
    let id = task["task"].as_str().unwrap_or_else(|| panic!("{task}"));
    assert_eq!(
        id.parse::<uuid::Uuid>().unwrap().get_version_num(),
        7,
        "{id}"
    );
    id.to_owned()
}

fn ids(list: &Value) -> Vec<String> {
    list["tasks"].as_array().unwrap().iter().map(id).collect()
}

#[test]
fn tasks_are_written_listed_archived_restored_and_deleted() {
    let server = Server::start();
    let created = server.ok(&[
        "task",
        "create",
        "--title",
        "Fix the login bug",
        "--description",
        "It fails on Safari.",
    ]);
    let first = id(&created);
    assert_eq!(created["title"], "Fix the login bug");
    assert_eq!(created["description"], "It fails on Safari.");
    assert_eq!(created["status"], "todo");
    assert_eq!(created["repository"], Value::Null);
    assert_eq!(created["workspace"], Value::Null);
    assert_eq!(created["last"], Value::Null);

    // No Workspace on the Device, so nowhere to start by default.
    assert_eq!(
        server.err(&["task", "start", &first, "--kind", "claude"]),
        "where_required"
    );

    let mut piped = server
        .command(&[
            "task",
            "create",
            "--title",
            "Piped",
            "--description-file",
            "-",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    piped
        .stdin
        .take()
        .unwrap()
        .write_all(b"From stdin.\n")
        .unwrap();
    let piped = ok(piped.wait_with_output().unwrap(), &["create from stdin"]);
    assert_eq!(piped["description"], "From stdin.\n");
    let second = id(&piped);
    let file = server.root.join("description.txt");
    std::fs::write(&file, "From a file.").unwrap();
    let from_file = server.ok(&[
        "task",
        "create",
        "--title",
        "Filed",
        "--description-file",
        file.to_str().unwrap(),
    ]);
    assert_eq!(from_file["description"], "From a file.");
    let third = id(&from_file);

    // New Tasks head To do; a list carries no descriptions.
    let list = server.ok(&["task", "list"]);
    assert_eq!(
        ids(&list),
        [third.as_str(), second.as_str(), first.as_str()]
    );
    assert!(list["tasks"][0].get("description").is_none(), "{list}");

    let edited = server.ok(&["task", "edit", &second, "--title", "Piped, renamed"]);
    assert_eq!(edited["title"], "Piped, renamed");
    assert_eq!(edited["description"], "From stdin.\n");
    let got = server.ok(&["task", "get", &second]);
    assert_eq!(got["title"], "Piped, renamed");
    assert_eq!(got["description"], "From stdin.\n");

    assert_eq!(
        server.ok(&["task", "archive", &third])["status"],
        "archived"
    );
    assert_eq!(
        ids(&server.ok(&["task", "list"])),
        [second.as_str(), first.as_str()]
    );
    assert_eq!(
        ids(&server.ok(&["task", "list", "--archived"])),
        [third.as_str()]
    );
    assert_eq!(server.ok(&["task", "restore", &third])["status"], "todo");
    assert_eq!(
        ids(&server.ok(&["task", "list"])),
        [third.as_str(), second.as_str(), first.as_str()]
    );

    assert_eq!(
        server.ok(&["task", "delete", &third]),
        serde_json::json!({ "ok": true })
    );
    assert_eq!(server.err(&["task", "get", &third]), "task_not_found");
    assert_eq!(server.err(&["task", "delete", &third]), "task_not_found");
    // Outside a Pane an omitted id names nothing.
    assert_eq!(server.err(&["task", "get"]), "no_current_pane");
    assert_eq!(
        server.err(&["task", "create", "--title", "x", "--repository", "99"]),
        "workspace_not_found"
    );
    for args in [
        vec!["task", "archive", &first, "--remove-worktree"],
        // An id is the whole UUID: neither a number nor a prefix.
        vec!["task", "get", "1"],
        vec!["task", "get", &first[..8]],
    ] {
        let usage = server.command(&args).output().unwrap();
        assert_eq!(usage.status.code(), Some(2), "{args:?}: {usage:?}");
    }
}

/// A Start types the Agent with a prompt naming the task file, links the Workspace and
/// answers with its Workspace, Tab and Pane; archiving closes only when told.
#[cfg(unix)]
#[test]
fn a_started_task_holds_its_workspace_until_archived() {
    let server = Server::start();
    let (first, _) = server.workspace("plain");
    let fix = id(&server.ok(&[
        "task",
        "create",
        "--title",
        "Fix it",
        "--description",
        "Make it work.",
    ]));

    // The Device's first Workspace no Task holds, in a new Tab named after the Task.
    let started = server.ok(&["task", "start", &fix, "--kind", "claude"]);
    assert_eq!(started["workspace"]["workspace_id"], first);
    assert_eq!(started["tab"]["workspace_id"], first);
    assert_eq!(started["tab"]["name"], "Fix it");
    let pane = started["pane"]["pane_id"].as_u64().unwrap();
    assert_eq!(started["pane"]["tab_id"], started["tab"]["tab_id"]);
    let args = server.agent_args();
    let file = args[0]
        .strip_prefix("Read ")
        .and_then(|rest| rest.strip_suffix(" and do the task it describes."))
        .unwrap_or_else(|| panic!("{args:?}"));
    assert_eq!(
        std::fs::read_to_string(file).unwrap(),
        "Fix it\n\nMake it work."
    );
    assert!(
        std::path::Path::new(file).starts_with(server.root.join("data")),
        "{file}"
    );

    let task = server.ok(&["task", "get", &fix]);
    assert_eq!(task["status"], "started");
    assert_eq!(task["workspace"], first);
    assert_eq!(task["last"]["managed"], false);
    // Inside a Pane of its Workspace, an omitted id is this Task.
    let mine = ok(server.in_pane(pane, &["task", "get"]), &["get in a Pane"]);
    assert_eq!(mine["task"], fix);
    assert_eq!(mine["description"], "Make it work.");

    assert_eq!(
        server.err(&["task", "start", &fix, "--kind", "claude"]),
        "task_started"
    );
    assert_eq!(server.err(&["task", "delete", &fix]), "task_started");
    let other = id(&server.ok(&["task", "create", "--title", "Other"]));
    assert_eq!(
        server.err(&[
            "task",
            "start",
            &other,
            "--kind",
            "claude",
            "--workspace",
            &first.to_string()
        ]),
        "workspace_has_task"
    );
    let (second, second_pane) = server.workspace("second");
    assert_eq!(
        err(
            server.in_pane(second_pane, &["task", "get"]),
            &["get in a Pane without a Task"]
        ),
        "no_task"
    );

    // A Start holds its Task in "Starting" until it has written the task file, here a
    // FIFO that blocks the write until it is read; another Start of it is refused.
    let fifo = std::path::Path::new(file).with_file_name(format!("{other}.md"));
    let made = Command::new("mkfifo").arg(&fifo).status().unwrap();
    assert!(made.success());
    let held = server
        .command(&[
            "task",
            "start",
            &other,
            "--kind",
            "claude",
            "--workspace",
            &second.to_string(),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    wait_for(|| (server.ok(&["task", "get", &other])["status"] == "starting").then_some(()));
    assert_eq!(
        server.err(&["task", "start", &other, "--kind", "claude"]),
        "task_starting"
    );
    assert_eq!(std::fs::read_to_string(&fifo).unwrap(), "Other\n\n");
    let held = ok(held.wait_with_output().unwrap(), &["the held Start"]);
    assert_eq!(held["workspace"]["workspace_id"], second);
    server.agent_args();

    // Archived without being told to close, the Workspace stays and leaves the Task.
    let archived = server.ok(&["task", "archive", &fix]);
    assert_eq!(archived["status"], "archived");
    assert_eq!(archived["workspace"], Value::Null);
    server.ok(&["workspace", "get", &first.to_string()]);
    assert_eq!(server.ok(&["task", "restore", &fix])["status"], "todo");
    let continued = server.ok(&[
        "task",
        "start",
        &fix,
        "--kind",
        "claude",
        "--continue",
        "--focus",
    ]);
    assert_eq!(continued["workspace"]["workspace_id"], first);
    server.agent_args();

    let closed = server.ok(&["task", "archive", &fix, "--close-workspace"]);
    assert_eq!(closed["status"], "archived");
    wait_for(|| {
        let output = server
            .command(&["workspace", "get", &first.to_string()])
            .output()
            .unwrap();
        (!output.status.success()).then_some(())
    });
    assert_eq!(
        server.ok(&["task", "delete", &fix]),
        serde_json::json!({ "ok": true })
    );
}

/// `--repository` names a repository Workspace; `--worktree` makes a Managed Worktree of
/// it, which archiving removes only with `--remove-worktree`.
#[cfg(unix)]
#[test]
fn a_task_starts_in_a_new_worktree_that_archiving_removes() {
    let server = Server::start();
    let repository = server.root.join("api");
    std::fs::create_dir_all(&repository).unwrap();
    git(&repository, &["init", "-q", "-b", "main"]);
    // The worktree's reflog needs a committer, and CI has no global one.
    git(&repository, &["config", "user.name", "Condr Tests"]);
    git(
        &repository,
        &["config", "user.email", "condr@example.invalid"],
    );
    git(&repository, &["config", "commit.gpgsign", "false"]);
    std::fs::write(repository.join("README.md"), "api\n").unwrap();
    git(&repository, &["add", "README.md"]);
    git(&repository, &["commit", "-q", "-m", "initial"]);
    let workspace = server.ok(&["workspace", "create", "--cwd", repository.to_str().unwrap()]);
    let parent = workspace["workspace"]["workspace_id"].as_u64().unwrap();
    let root = workspace["workspace"]["root_directory"].clone();

    let created = server.ok(&[
        "task",
        "create",
        "--title",
        "Add login",
        "--repository",
        &parent.to_string(),
    ]);
    assert_eq!(created["repository"], root);
    let add = id(&created);
    let started = server.ok(&["task", "start", &add, "--kind", "claude", "--worktree"]);
    let worktree = &started["workspace"]["worktree"];
    assert_eq!(worktree["parent_workspace_id"], parent);
    assert_eq!(worktree["managed"], true);
    let path = PathBuf::from(started["workspace"]["root_directory"].as_str().unwrap());
    assert!(path.is_dir());
    server.agent_args();
    let task = server.ok(&["task", "get", &add]);
    assert_eq!(task["last"]["managed"], true);
    assert_eq!(task["last"]["path"], started["workspace"]["root_directory"]);

    let archived = server.ok(&[
        "task",
        "archive",
        &add,
        "--close-workspace",
        "--remove-worktree",
    ]);
    assert_eq!(archived["status"], "archived");
    assert_eq!(archived["last"]["branch"], "worktree/add-login");
    wait_for(|| (!path.exists()).then_some(()));
}

#[cfg(unix)]
fn git(directory: &std::path::Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(directory)
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?}: {output:?}");
}

/// A Server that predates Tasks answers a Task request with the frozen
/// `UNSUPPORTED_REQUEST`, which the CLI reports as `feature_unavailable`.
#[cfg(unix)]
#[test]
fn an_older_server_answers_feature_unavailable() {
    use condr_core::protocol::{
        ClientHandshake, ClientMessage, RuntimeEpoch, ServerId, ServerMessage, SessionId,
        SessionOverview, UNSUPPORTED_REQUEST, Welcome, read_message, write_message,
    };

    let root = std::env::temp_dir().join(format!("condr-task-old-{}", unique_suffix()));
    std::fs::create_dir_all(&root).unwrap();
    let socket = root.join("old.sock");
    let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
    let old = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let _: ClientHandshake = read_message(&mut stream).unwrap();
        let (server_id, session_id) = (ServerId(1), SessionId(1));
        write_message(
            &mut stream,
            &Welcome::new(server_id, session_id, None, "old".into()),
        )
        .unwrap();
        let request: ClientMessage = read_message(&mut stream).unwrap();
        assert!(matches!(request, ClientMessage::OverviewRequest { .. }));
        write_message(
            &mut stream,
            &ServerMessage::Overview(SessionOverview {
                server_id,
                runtime_epoch: RuntimeEpoch(1),
                session_id,
                sequence: 0,
                snapshot: condr_core::Session::new().snapshot(),
                terminals: Vec::new(),
                agents: Vec::new(),
                zoomed_panes: Vec::new(),
            }),
        )
        .unwrap();
        write_message(
            &mut stream,
            &ServerMessage::OverviewTerminals {
                server_id,
                session_id,
                terminals: Vec::new(),
            },
        )
        .unwrap();
        // What a Server answers a message it does not know.
        let request: ClientMessage = read_message(&mut stream).unwrap();
        assert!(matches!(request, ClientMessage::Task { .. }), "{request:?}");
        write_message(
            &mut stream,
            &ServerMessage::Error {
                message: UNSUPPORTED_REQUEST.into(),
            },
        )
        .unwrap();
    });
    let output = Command::new(env!("CARGO_BIN_EXE_condr"))
        .args(["task", "list"])
        .env("CONDR_SOCKET_PATH", &socket)
        .env_remove("CONDR_PANE_ID")
        .env_remove("CONDR_DEVICE")
        .output()
        .unwrap();
    assert_eq!(err(output, &["task", "list"]), "feature_unavailable");
    old.join().unwrap();
    let _ = std::fs::remove_dir_all(root);
}

/// `--device` reaches a saved TCP Device, whose Tasks are its own; caller defaults do not
/// apply there.
#[test]
fn tasks_are_written_on_a_device_over_tcp() {
    let root = std::env::temp_dir().join(format!("condr-task-dev-{}", unique_suffix()));
    let config = root.join("config");
    std::fs::create_dir_all(&config).unwrap();
    let device_key = condr_server::noise::load_device_key(&config).unwrap();
    let identity = condr_server::noise::ServerIdentity::ephemeral()
        .unwrap()
        .with_authorized(device_key.public());
    let key = identity.public_key();
    let server = BoundServer::bind(
        ServerConfig::ephemeral(root.join("lab.sock"))
            .with_listen("127.0.0.1:0".parse().unwrap())
            .with_identity(identity)
            .with_data_directory(root.join("lab data")),
    )
    .unwrap();
    let address = server.local_addr().unwrap().unwrap();
    let handle = server.handle();
    let thread = thread::spawn(move || server.run());
    std::fs::write(
        config.join("config.toml"),
        format!("[[client.servers]]\nname = 'Lab'\naddress = 'tcp://{key}@{address}'\n"),
    )
    .unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_condr"))
            .args(args)
            // No local Server answers here: only the Device does.
            .env("CONDR_SOCKET_PATH", root.join("absent.sock"))
            .env("CONDR_CONFIG_DIR", &config)
            .env("CONDR_DATA_DIR", &config)
            .env("CONDR_PANE_ID", "1")
            .env_remove("CONDR_DEVICE")
            .output()
            .unwrap()
    };

    let args = ["--device", "Lab", "task", "create", "--title", "Remote"];
    let created = ok(run(&args), &args);
    let remote = id(&created);
    let args = ["--device", "Lab", "task", "list"];
    assert_eq!(ids(&ok(run(&args), &args)), [remote.as_str()]);
    let args = ["--device", "Lab", "task", "get"];
    assert_eq!(err(run(&args), &args), "no_current_pane");
    let args = ["task", "list"];
    assert_eq!(err(run(&args), &args), "server_not_running");

    handle.stop();
    thread.join().unwrap().unwrap();
    let _ = std::fs::remove_dir_all(root);
}
