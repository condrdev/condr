#![cfg(any(target_os = "linux", target_os = "windows"))]

use condr_core::Session;
use condr_server::{BoundServer, ClientConnection, Endpoint, ServerConfig};
use std::{path::PathBuf, thread, time::Duration};

mod common;
use common::unique_suffix;

fn test_endpoint() -> Endpoint {
    Endpoint::local(std::env::temp_dir().join(format!(
        "condr-server-{}-{}.sock",
        std::process::id(),
        unique_suffix()
    )))
}

fn run_git(cwd: &std::path::Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "Git failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn write_snapshot_fixture(path: PathBuf, snapshot: condr_core::SessionSnapshot) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, snapshot.to_bytes().unwrap()).unwrap();
}

fn wait_for_connection(endpoint: &Endpoint) {
    for _ in 0..100 {
        if endpoint.connect().is_ok() {
            return;
        }
        thread::sleep(Duration::from_millis(5));
    }
    panic!("server did not start");
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[test]
fn restart_revalidates_worktree_authority_against_the_git_topology() {
    let directory = std::env::temp_dir().join(format!(
        "condr-server-worktree-restore-{}-{}",
        std::process::id(),
        unique_suffix()
    ));
    let repository = directory.join("repository");
    let parent_workspace_root = repository.join("workspace-root");
    let child_root = directory.join("child");
    let snapshot_path = directory.join("session.snapshot");
    std::fs::create_dir_all(&repository).unwrap();
    run_git(&repository, &["init"]);
    run_git(&repository, &["config", "user.name", "Condr Tests"]);
    run_git(
        &repository,
        &["config", "user.email", "condr@example.invalid"],
    );
    std::fs::create_dir_all(&parent_workspace_root).unwrap();
    std::fs::write(parent_workspace_root.join("README.md"), "condr\n").unwrap();
    run_git(&repository, &["add", "workspace-root/README.md"]);
    run_git(&repository, &["commit", "-m", "initial"]);
    run_git(
        &repository,
        &[
            "worktree",
            "add",
            "-b",
            "feature/recovered",
            child_root.to_str().unwrap(),
        ],
    );

    let mut session = Session::new();
    let parent_workspace_id = session
        .create_workspace(parent_workspace_root.clone())
        .expect("Workspace capacity");
    let child_workspace_id = session
        .create_workspace(child_root.clone())
        .expect("Workspace capacity");
    assert!(session.associate_worktree(
        child_workspace_id,
        parent_workspace_id,
        parent_workspace_root,
        true,
    ));
    session
        .close_workspace(parent_workspace_id)
        .expect("historical parent Workspace exists");
    write_snapshot_fixture(snapshot_path.clone(), session.snapshot());

    let first_endpoint = test_endpoint();
    let first_server = BoundServer::bind(
        ServerConfig::at_socket(first_endpoint.as_local_path().unwrap())
            .with_snapshot_path(snapshot_path.clone()),
    )
    .unwrap();
    let first_handle = first_server.handle();
    let first_thread = thread::spawn(move || first_server.run());
    wait_for_connection(&first_endpoint);
    let first = ClientConnection::connect(&first_endpoint, "valid-worktree-restore").unwrap();
    let first_session = Session::restore(first.bootstrap().unwrap().snapshot.clone()).unwrap();
    assert!(
        first_session
            .workspace(child_workspace_id)
            .and_then(|workspace| workspace.worktree())
            .is_some_and(|association| association.is_managed())
    );
    drop(first);
    first_handle.stop();
    first_thread.join().unwrap().unwrap();

    run_git(
        &repository,
        &["worktree", "remove", child_root.to_str().unwrap()],
    );
    std::fs::create_dir_all(&child_root).unwrap();

    let second_endpoint = test_endpoint();
    let second_server = BoundServer::bind(
        ServerConfig::at_socket(second_endpoint.as_local_path().unwrap())
            .with_snapshot_path(snapshot_path.clone()),
    )
    .unwrap();
    let second_handle = second_server.handle();
    let second_thread = thread::spawn(move || second_server.run());
    wait_for_connection(&second_endpoint);
    let second = ClientConnection::connect(&second_endpoint, "stale-worktree-restore").unwrap();
    let repaired = Session::restore(second.bootstrap().unwrap().snapshot.clone()).unwrap();
    assert!(
        repaired
            .workspace(child_workspace_id)
            .expect("child Workspace survives")
            .worktree()
            .is_none()
    );
    drop(second);
    second_handle.stop();
    second_thread.join().unwrap().unwrap();

    let persisted = condr_core::SessionSnapshot::from_bytes(
        &std::fs::read(&snapshot_path).expect("repaired Snapshot exists"),
    )
    .unwrap();
    assert!(
        Session::restore(persisted)
            .unwrap()
            .workspace(child_workspace_id)
            .expect("child Workspace remains persisted")
            .worktree()
            .is_none()
    );
    let _ = std::fs::remove_dir_all(directory);
}
