use super::*;
use crate::test_support::unique_suffix;
use std::fs;

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("condr-task-store-{}", unique_suffix()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn store(&self) -> PathBuf {
        self.0.join(crate::store::FILE_NAME)
    }

    /// The files set aside next to the store.
    fn set_aside(&self) -> Vec<PathBuf> {
        fs::read_dir(&self.0)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("server.sqlite.unreadable-")
            })
            .collect()
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn task(id: u128, title: &str, order: i64) -> Task {
    Task {
        id: TaskId::from_u128(id),
        title: title.into(),
        description: Some(format!("{title}, in detail\nover two lines")),
        repository: Some("/src/api".into()),
        order,
        archived: false,
        workspace: None,
        made_workspace: false,
        starting: false,
        last: None,
    }
}

/// The migrations the database at `path` has run, by version.
fn applied(path: &Path) -> Vec<i32> {
    let connection = Connection::open(path).unwrap();
    let mut statement = connection
        .prepare("SELECT version FROM refinery_schema_history ORDER BY version")
        .unwrap();
    statement
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

/// Every migration this build has, by version.
fn latest() -> Vec<i32> {
    crate::store::migrations::runner()
        .get_migrations()
        .iter()
        .map(|migration| migration.version())
        .collect()
}

fn sorted(mut tasks: Vec<Task>) -> Vec<Task> {
    tasks.sort_by_key(|task| task.id);
    tasks
}

/// ADR 0068: what the Server writes is what it reads back after a restart: created,
/// edited, reordered, archived, restored and deleted Tasks, links and last places.
#[test]
fn the_store_keeps_every_change_across_a_restart() {
    let directory = TestDirectory::new();
    let mut opened = TaskStore::open(&directory.store()).unwrap();
    assert!(opened.tasks.is_empty());

    let mut first = task(1, "Fix the login bug", 0);
    let mut second = task(2, "修复输入法", -1);
    let third = task(3, "Dropped", -2);
    for task in [&first, &second, &third] {
        opened.store.schedule(task.id, Some(task.clone()));
    }
    // Edited, moved, started and archived, then restored.
    first.title = "Fix the login bug on Safari".into();
    first.order = 5;
    first.workspace = Some(WorkspaceId::from_u64(7));
    first.made_workspace = true;
    first.last = Some(LastPlace {
        path: "/src/api-login".into(),
        branch: Some("worktree/fix-the-login-bug".into()),
        base_branch: Some("main".into()),
        managed: true,
        repository: Some("/src/api".into()),
    });
    second.archived = true;
    second.repository = None;
    opened.store.schedule(first.id, Some(first.clone()));
    opened.store.schedule(second.id, Some(second.clone()));
    opened.store.schedule(third.id, None);
    opened.store.shutdown().unwrap();
    drop(opened);
    assert_eq!(applied(&directory.store()), latest());

    let mut reopened = TaskStore::open(&directory.store()).unwrap();
    assert_eq!(
        sorted(reopened.tasks.clone()),
        vec![first.clone(), second.clone()]
    );
    assert!(directory.set_aside().is_empty());

    second.archived = false;
    reopened.store.schedule(second.id, Some(second.clone()));
    reopened.store.shutdown().unwrap();
    drop(reopened);
    let restored = TaskStore::open(&directory.store()).unwrap();
    assert_eq!(sorted(restored.tasks.clone()), vec![first, second]);
}

/// ADR 0068 and 0028: a database a newer Server has migrated further, one whose migration
/// has been edited since it ran, and one that is not a database are renamed aside, never
/// overwritten, and the Server starts with no Tasks.
#[test]
fn a_newer_edited_or_unreadable_store_is_set_aside_and_tasks_start_empty() {
    for change in [
        "INSERT INTO refinery_schema_history (version, name, applied_on, checksum) \
         VALUES (9999, 'from_a_newer_server', '2026-11-18T00:00:00Z', '0')",
        "UPDATE refinery_schema_history SET checksum = '0' WHERE version = 1",
    ] {
        let directory = TestDirectory::new();
        let mut opened = TaskStore::open(&directory.store()).unwrap();
        let kept = task(1, "Kept", 0);
        opened.store.schedule(kept.id, Some(kept));
        opened.store.shutdown().unwrap();
        drop(opened);
        Connection::open(directory.store())
            .unwrap()
            .execute_batch(change)
            .unwrap();

        let opened = TaskStore::open(&directory.store()).unwrap();
        assert!(opened.tasks.is_empty());
        let aside = directory.set_aside();
        assert_eq!(aside.len(), 1);
        let left = Connection::open(&aside[0]).unwrap();
        let count = |sql: &str| left.query_row(sql, [], |row| row.get::<_, i64>(0)).unwrap();
        assert_eq!(
            count("SELECT count(*) FROM refinery_schema_history WHERE checksum = '0'"),
            1,
            "left as it was"
        );
        assert_eq!(count("SELECT count(*) FROM tasks"), 1, "its Task kept");
        drop(opened);
        assert_eq!(applied(&directory.store()), latest());
    }

    let directory = TestDirectory::new();
    fs::write(directory.store(), b"not a database, just text").unwrap();
    let opened = TaskStore::open(&directory.store()).unwrap();
    assert!(opened.tasks.is_empty());
    let aside = directory.set_aside();
    assert_eq!(aside.len(), 1);
    assert_eq!(fs::read(&aside[0]).unwrap(), b"not a database, just text");
}

/// A store another Server holds, or one that cannot be opened for a reason that may pass,
/// is left exactly as it is: the caller keeps its Tasks in memory.
#[test]
fn a_store_in_use_or_unavailable_is_left_alone() {
    let directory = TestDirectory::new();
    let mut first = TaskStore::open(&directory.store()).unwrap();
    let error = TaskStore::open(&directory.store())
        .err()
        .expect("a second Server cannot open a held store");
    assert!(error.to_string().contains("another Server"), "{error}");
    assert!(directory.set_aside().is_empty(), "nothing is set aside");
    let kept = task(1, "Kept", 0);
    first.store.schedule(kept.id, Some(kept.clone()));
    first.store.shutdown().unwrap();
    drop(first);
    assert_eq!(
        TaskStore::open(&directory.store()).unwrap().tasks,
        vec![kept]
    );

    // A directory where the file should be cannot be opened, and is not a damaged store.
    let directory = TestDirectory::new();
    fs::create_dir(directory.store()).unwrap();
    assert!(TaskStore::open(&directory.store()).is_err());
    assert!(directory.set_aside().is_empty());
    assert!(directory.store().is_dir());
}

#[cfg(unix)]
#[test]
fn the_store_is_the_persons_alone() {
    use std::os::unix::fs::PermissionsExt as _;
    let directory = TestDirectory::new();
    let store = directory.0.join("data").join(crate::store::FILE_NAME);
    let opened = TaskStore::open(&store).unwrap();
    let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&store), 0o600);
    assert_eq!(mode(store.parent().unwrap()), 0o700);
    drop(opened);
}
