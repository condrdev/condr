use condr_core::AgentKind;
use condr_core::agent_hooks::{HookTarget, HooksState, install, state, uninstall};
use std::fs::{self, OpenOptions};
use std::io::{BufRead as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

static NEXT_ROOT: AtomicU64 = AtomicU64::new(1);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "condr-hook-transactions-{}-{}",
            std::process::id(),
            NEXT_ROOT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn target(&self) -> HookTarget {
        HookTarget::in_home(&self.0, "condr".into())
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn read(path: &Path) -> serde_json::Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

#[test]
#[ignore = "spawned by hook_transactions_lock_actual_files_before_reading_or_writing"]
fn hook_transaction_child() {
    let home = PathBuf::from(std::env::var_os("CONDR_TEST_HOOK_HOME").unwrap());
    let agent = AgentKind::parse_label(&std::env::var("CONDR_TEST_HOOK_AGENT").unwrap()).unwrap();
    let target = HookTarget::in_home(&home, "condr".into());
    println!("CONDR_HOOK_CHILD_READY");
    std::io::stdout().flush().unwrap();
    install(&target, agent).unwrap();
}

#[test]
fn hook_transactions_lock_actual_files_before_reading_or_writing() {
    for agent in [AgentKind::Claude, AgentKind::OpenCode] {
        let directory = TestDirectory::new();
        let target = directory.target();
        let hooks = target.path(agent);
        let config = if agent == AgentKind::OpenCode {
            hooks.with_file_name("tui.json")
        } else {
            hooks.clone()
        };
        fs::create_dir_all(config.parent().unwrap()).unwrap();
        #[cfg(unix)]
        let real = {
            let real = directory.0.join("dotfile.json");
            fs::write(&real, r#"{"userSetting":"before"}"#).unwrap();
            std::os::unix::fs::symlink(&real, &config).unwrap();
            real
        };
        #[cfg(not(unix))]
        let real = {
            fs::write(&config, r#"{"userSetting":"before"}"#).unwrap();
            config.clone()
        };
        let mut lock_path = real.as_os_str().to_os_string();
        lock_path.push(".condr-lock");
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)
            .unwrap();
        lock.lock().unwrap();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "hook_transaction_child",
                "--nocapture",
            ])
            .env("CONDR_TEST_HOOK_HOME", &directory.0)
            .env("CONDR_TEST_HOOK_AGENT", agent.id())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let output = child.stdout.take().unwrap();
        let mut output = std::io::BufReader::new(output);
        let mut line = String::new();
        while !line.contains("CONDR_HOOK_CHILD_READY") {
            line.clear();
            assert_ne!(output.read_line(&mut line).unwrap(), 0);
        }
        std::thread::sleep(Duration::from_millis(150));
        assert!(
            child.try_wait().unwrap().is_none(),
            "Hooks bypassed the held lock"
        );
        assert_eq!(read(&real)["userSetting"], "before");
        if agent == AgentKind::OpenCode {
            assert!(
                !hooks.exists(),
                "script was written before its registry was locked"
            );
        }
        // The child must read after acquiring the lock, not overwrite this newer edit.
        fs::write(&real, r#"{"userSetting":"after","plugin":["user-plugin"]}"#).unwrap();
        drop(lock);
        assert!(child.wait().unwrap().success());
        assert_eq!(read(&real)["userSetting"], "after");
        assert_eq!(state(&target, agent).unwrap(), HooksState::Installed);
        if agent == AgentKind::OpenCode {
            assert_eq!(
                read(&real)["plugin"],
                serde_json::json!(["user-plugin", "./condr-tui.js"])
            );
        }
        #[cfg(unix)]
        assert!(fs::symlink_metadata(config).unwrap().is_symlink());
    }
}

#[test]
fn hook_installation_does_not_reuse_a_shared_temporary_file() {
    let directory = TestDirectory::new();
    let target = directory.target();
    let hooks = target.path(AgentKind::Claude);
    fs::create_dir_all(hooks.parent().unwrap()).unwrap();
    let shared_temporary = hooks.with_extension("condr-tmp");
    fs::write(&shared_temporary, "another writer's temporary file").unwrap();
    install(&target, AgentKind::Claude).unwrap();
    assert_eq!(
        state(&target, AgentKind::Claude).unwrap(),
        HooksState::Installed
    );
    assert_eq!(
        fs::read_to_string(shared_temporary).unwrap(),
        "another writer's temporary file"
    );
}

#[test]
fn inspecting_or_removing_absent_hooks_creates_no_agent_directories() {
    let directory = TestDirectory::new();
    let target = directory.target();
    for agent in AgentKind::ALL {
        let expected = if agent.spec().hooks().is_some() {
            HooksState::Missing
        } else {
            HooksState::Unsupported
        };
        assert_eq!(state(&target, agent).unwrap(), expected);
        uninstall(&target, agent).unwrap();
    }
    assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 0);
}
