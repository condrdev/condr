use super::*;
use crate::test_support::unique_suffix;

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("condr-task-store-{}", unique_suffix()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn store(&self) -> PathBuf {
        self.0.join("tasks.sqlite")
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
                    .starts_with("tasks.sqlite.unreadable-")
            })
            .collect()
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn task(id: u64, title: &str, order: i64) -> Task {
    Task {
        id: TaskId::from_u64(id),
        title: title.into(),
        description: Some(format!("{title}, in detail\nover two lines")),
        repository: Some("/src/api".into()),
        order,
        archived: false,
        workspace: None,
        starting: false,
        last: None,
    }
}

fn user_version(path: &Path) -> i64 {
    Connection::open(path)
        .unwrap()
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap()
}

fn sorted(mut tasks: Vec<Task>) -> Vec<Task> {
    tasks.sort_by_key(|task| task.id);
    tasks
}

/// ADR 0068: what the Server writes is what it reads back after a restart: created,
/// edited, reordered, archived, restored and deleted Tasks, links and last places.
#[test]
fn the_store_keeps_every_change_across_a_restart_and_never_reuses_a_number() {
    let directory = TestDirectory::new();
    let mut opened = TaskStore::open(&directory.store()).unwrap();
    assert!(opened.tasks.is_empty());
    assert_eq!(opened.next_id, 1);
    assert_eq!(user_version(&directory.store()), SCHEMA_VERSION);

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

    let mut reopened = TaskStore::open(&directory.store()).unwrap();
    assert_eq!(
        sorted(reopened.tasks.clone()),
        vec![first.clone(), second.clone()]
    );
    assert_eq!(
        reopened.next_id, 4,
        "a deleted Task's number does not return"
    );
    assert!(directory.set_aside().is_empty());

    second.archived = false;
    reopened.store.schedule(second.id, Some(second.clone()));
    reopened.store.shutdown().unwrap();
    drop(reopened);
    let restored = TaskStore::open(&directory.store()).unwrap();
    assert_eq!(sorted(restored.tasks.clone()), vec![first, second]);
}

/// ADR 0068 and 0028: a store from a newer Server, or one that is not a database, is
/// renamed aside, never overwritten, and the Server starts with no Tasks.
#[test]
fn a_newer_or_unreadable_store_is_set_aside_and_tasks_start_empty() {
    let directory = TestDirectory::new();
    {
        let newer = Connection::open(directory.store()).unwrap();
        newer
            .pragma_update(None, "user_version", SCHEMA_VERSION + 1)
            .unwrap();
    }
    let opened = TaskStore::open(&directory.store()).unwrap();
    assert!(opened.tasks.is_empty());
    let aside = directory.set_aside();
    assert_eq!(aside.len(), 1);
    assert_eq!(
        user_version(&aside[0]),
        SCHEMA_VERSION + 1,
        "left as it was"
    );
    assert_eq!(user_version(&directory.store()), SCHEMA_VERSION);
    drop(opened);

    let directory = TestDirectory::new();
    fs::write(directory.store(), b"not a database, just text").unwrap();
    let opened = TaskStore::open(&directory.store()).unwrap();
    assert!(opened.tasks.is_empty());
    let aside = directory.set_aside();
    assert_eq!(aside.len(), 1);
    assert_eq!(fs::read(&aside[0]).unwrap(), b"not a database, just text");
}
