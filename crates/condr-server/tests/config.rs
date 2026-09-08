use condr_server::{read_config_text, update_config_values};
#[cfg(windows)]
use std::fs::OpenOptions;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(1);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "condr-config-tests-{}-{}",
            std::process::id(),
            NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
#[ignore = "spawned by config_transactions_preserve_concurrent_process_updates"]
fn config_child_update() {
    let path = PathBuf::from(std::env::var_os("CONDR_TEST_CONFIG_TRANSACTION_PATH").unwrap());
    let key = std::env::var("CONDR_TEST_CONFIG_TRANSACTION_KEY").unwrap();
    for value in 0..80 {
        update_config_values(&path, &["client"], [(&key, Some(toml_edit::value(value)))]).unwrap();
    }
}

#[test]
fn config_transactions_preserve_concurrent_process_updates() {
    let directory = TestDirectory::new();
    let path = directory.path().join("config.toml");
    let initial = format!(
        "# keep comment\n[server]\nlisten = '127.0.0.1:42' # keep suffix\n[client]\nmarker = '{}'\n",
        "x".repeat(65536)
    );
    fs::write(&path, &initial).unwrap();
    #[cfg(unix)]
    let alias = {
        let alias = directory.path().join("linked.toml");
        std::os::unix::fs::symlink(&path, &alias).unwrap();
        alias
    };
    #[cfg(not(unix))]
    let alias = path.clone();
    let mut children = [(&path, "appearance"), (&alias, "font_size")].map(|(path, key)| {
        std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--ignored", "--exact", "config_child_update"])
            .env("CONDR_TEST_CONFIG_TRANSACTION_PATH", path)
            .env("CONDR_TEST_CONFIG_TRANSACTION_KEY", key)
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap()
    });
    for _ in 0..160 {
        let text = read_config_text(&path).unwrap().unwrap();
        let root: toml::Table = text.parse().unwrap();
        assert_eq!(root["client"]["marker"].as_str().unwrap().len(), 65536);
    }
    for child in &mut children {
        assert!(child.wait().unwrap().success());
    }
    let text = read_config_text(&path).unwrap().unwrap();
    let root: toml::Table = text.parse().unwrap();
    assert_eq!(root["client"]["appearance"].as_integer(), Some(79));
    assert_eq!(root["client"]["font_size"].as_integer(), Some(79));
    assert!(text.starts_with("# keep comment\n[server]\nlisten = '127.0.0.1:42' # keep suffix"));
    #[cfg(unix)]
    assert!(fs::symlink_metadata(alias).unwrap().is_symlink());
}

#[test]
fn config_updates_keep_scalar_comments_and_reject_malformed_documents() {
    let directory = TestDirectory::new();
    let path = directory.path().join("config.toml");
    fs::write(
        &path,
        "[server.terminal]\n# shell comment\nshell = 'old' # trailing\n",
    )
    .unwrap();
    update_config_values(
        &path,
        &["server", "terminal"],
        [("shell", Some(toml_edit::value("new")))],
    )
    .unwrap();
    let text = read_config_text(&path).unwrap().unwrap();
    assert!(text.contains("# shell comment"));
    assert!(text.contains("# trailing"));
    fs::write(&path, "[broken").unwrap();
    assert!(
        update_config_values(
            &path,
            &["client"],
            [("appearance", Some(toml_edit::value("dark")))]
        )
        .is_err()
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), "[broken");
}

/// An editor showing config.toml keeps it open without delete sharing, which would
/// refuse a replace; the in-place save must still land.
#[cfg(windows)]
#[test]
fn config_write_lands_while_another_program_holds_the_file() {
    use std::os::windows::fs::OpenOptionsExt as _;

    let directory = std::env::temp_dir().join(format!(
        "condr-config-held-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join("config.toml");
    fs::write(&path, "[client]\nappearance = 'light'\n").unwrap();
    // FILE_SHARE_READ | FILE_SHARE_WRITE, no FILE_SHARE_DELETE: what a typical editor holds.
    let holder = OpenOptions::new()
        .read(true)
        .share_mode(0x1 | 0x2)
        .open(&path)
        .unwrap();

    update_config_values(
        &path,
        &["client"],
        [("appearance", Some(toml_edit::value("dark")))],
    )
    .unwrap();
    let text = read_config_text(&path).unwrap().unwrap();
    assert_eq!(
        text.parse::<toml::Table>().unwrap()["client"]["appearance"].as_str(),
        Some("dark")
    );

    drop(holder);
    let _ = fs::remove_dir_all(directory);
}
