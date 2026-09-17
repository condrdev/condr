//! `condr server install` / `uninstall` against a throwaway home: the binary copies
//! itself, links and registers PATH, and uninstall takes all of it back (ADR 0016).
//! Windows edits the real user Path, so this only runs where everything is a file.
#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

struct Home {
    root: PathBuf,
}

impl Home {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("condr-install-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(root.join("home")).unwrap();
        Self { root }
    }
    fn home(&self) -> PathBuf {
        self.root.join("home")
    }
    fn install_dir(&self) -> PathBuf {
        self.root.join("opt")
    }
    fn profile(&self) -> PathBuf {
        self.home().join(".profile")
    }
    fn link(&self) -> PathBuf {
        self.home().join(".local/bin/condr")
    }
    fn run(&self, exe: &Path, args: &[&str]) -> Output {
        Command::new(exe)
            .args(args)
            .env("HOME", self.home())
            .env("CONDR_INSTALL_DIR", self.install_dir())
            .env("CONDR_PROFILE", self.profile())
            .env("CONDR_CONFIG_DIR", self.root.join("config"))
            .env("CONDR_SOCKET_PATH", self.root.join("absent.sock"))
            .env_remove("CONDR_PANE_ID")
            .output()
            .unwrap()
    }
    fn json(&self, exe: &Path, args: &[&str]) -> Value {
        let output = self.run(exe, args);
        assert!(
            output.status.success(),
            "{args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn marker_count(profile: &Path) -> usize {
    fs::read_to_string(profile)
        .unwrap_or_default()
        .lines()
        .filter(|line| *line == "# condr user bin")
        .count()
}

#[test]
fn install_places_links_registers_and_uninstall_takes_it_all_back() {
    let home = Home::new();
    let condr = PathBuf::from(env!("CARGO_BIN_EXE_condr"));
    let installed = home.install_dir().join("condr");
    fs::write(home.profile(), "# keep me\n").unwrap();

    let report = home.json(&condr, &["server", "install", "--json"]);
    assert_eq!(
        report["install_path"],
        Value::from(installed.to_str().unwrap())
    );
    assert_eq!(report["version"], Value::from(env!("CARGO_PKG_VERSION")));
    assert_eq!(
        report["protocol"],
        Value::from(condr_core::protocol::PROTOCOL_VERSION)
    );
    assert_eq!(report["replaced"], Value::from(false));
    assert_eq!(report["path_registered"], Value::from(true));
    assert_eq!(report["server"], Value::from("none"));
    assert!(installed.is_file());
    assert_eq!(fs::read_link(home.link()).unwrap(), installed);
    assert_eq!(marker_count(&home.profile()), 1);
    let profile = fs::read_to_string(home.profile()).unwrap();
    assert!(profile.starts_with("# keep me\n"));
    assert!(profile.contains("export PATH=\"$HOME/.local/bin:$PATH\"\n"));
    // The copy is a working executable.
    assert!(home.run(&installed, &["--version"]).status.success());

    // Again from the temporary copy: the file is replaced, PATH is registered once.
    let report = home.json(&condr, &["server", "install", "--json"]);
    assert_eq!(report["replaced"], Value::from(true));
    assert_eq!(marker_count(&home.profile()), 1);
    assert_eq!(fs::read_link(home.link()).unwrap(), installed);

    // From the installed path itself: nothing is copied.
    let report = home.json(&installed, &["server", "install", "--json"]);
    assert_eq!(report["replaced"], Value::from(false));
    assert_eq!(
        report["install_path"],
        Value::from(installed.to_str().unwrap())
    );
    assert!(installed.is_file());

    let report = home.json(&condr, &["server", "uninstall", "--yes", "--json"]);
    assert_eq!(
        report["removed_path"],
        Value::from(installed.to_str().unwrap())
    );
    assert_eq!(report["path_unregistered"], Value::from(true));
    assert_eq!(report["server"], Value::from("none"));
    assert_eq!(report["leftovers"], Value::Array(Vec::new()));
    assert!(!home.install_dir().exists(), "install directory was kept");
    assert!(
        fs::symlink_metadata(home.link()).is_err(),
        "symlink was kept"
    );
    assert_eq!(fs::read_to_string(home.profile()).unwrap(), "# keep me\n");

    // Uninstalling again is harmless.
    let report = home.json(&condr, &["server", "uninstall", "--yes", "--json"]);
    assert_eq!(report["server"], Value::from("none"));
}

#[test]
fn human_output_is_one_line_per_step() {
    let home = Home::new();
    let condr = PathBuf::from(env!("CARGO_BIN_EXE_condr"));
    let output = home.run(&condr, &["server", "install"]);
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.lines().all(|line| !line.trim().is_empty()),
        "{stdout}"
    );
    assert!(stdout.contains("installed "), "{stdout}");
    let output = home.run(&condr, &["server", "uninstall", "--yes"]);
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("kept your data in "), "{stdout}");
}
