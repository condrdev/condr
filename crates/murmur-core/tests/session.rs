use std::path::PathBuf;

use murmur_core::{
    PaneDirection, PaneLayout, Session, SessionSnapshot, SnapshotError, SplitDirection, WorkspaceId,
};

#[test]
fn creating_workspace_commits_a_complete_initial_tree() {
    let mut session = Session::new();
    assert!(session.is_empty());

    let root_directory = PathBuf::from("projects/murmur");
    let workspace_id = session.create_workspace(root_directory.clone());

    assert!(!session.is_empty());
    assert_eq!(session.active_workspace_id(), Some(workspace_id));
    assert_eq!(session.workspaces().len(), 1);

    let workspace = session.active_workspace().expect("workspace is active");
    assert_eq!(workspace.id(), workspace_id);
    assert_eq!(workspace.name(), "murmur");
    assert_eq!(workspace.root_directory(), root_directory.as_path());
    assert_eq!(workspace.tabs().len(), 1);

    let tab = workspace.active_tab();
    assert_eq!(tab.name(), "Tab 1");
    assert_eq!(tab.panes().len(), 1);

    let pane = tab.focused_pane();
    assert_eq!(pane.cwd(), Some(root_directory.as_path()));
}

#[test]
fn new_tab_follows_focused_pane_cwd_without_changing_workspace_root() {
    let mut session = Session::new();
    let root_directory = PathBuf::from("projects/murmur");
    let workspace_id = session.create_workspace(root_directory.clone());
    let first_pane_id = session
        .active_workspace()
        .expect("workspace is active")
        .active_tab()
        .focused_pane()
        .id();
    let pane_cwd = PathBuf::from("projects/murmur/crates/murmur-core");

    assert!(session.set_pane_cwd(first_pane_id, Some(pane_cwd.clone())));
    let tab_id = session.create_tab(workspace_id).expect("workspace exists");

    let workspace = session.active_workspace().expect("workspace is active");
    assert_eq!(workspace.root_directory(), root_directory.as_path());
    assert_eq!(workspace.active_tab().id(), tab_id);
    assert_eq!(workspace.active_tab().name(), "Tab 2");
    assert_eq!(
        workspace.active_tab().focused_pane().cwd(),
        Some(pane_cwd.as_path())
    );
}

#[test]
fn splitting_a_pane_records_layout_cwd_and_focus_history() {
    let mut session = Session::new();
    session.create_workspace(PathBuf::from("projects/murmur"));
    let first_pane_id = session
        .active_workspace()
        .expect("workspace is active")
        .active_tab()
        .focused_pane()
        .id();
    let pane_cwd = PathBuf::from("projects/murmur/crates/murmur-gui");
    session.set_pane_cwd(first_pane_id, Some(pane_cwd.clone()));

    let second_pane_id = session
        .split_pane(first_pane_id, SplitDirection::Horizontal, 0.75)
        .expect("pane exists");

    let tab = session
        .active_workspace()
        .expect("workspace is active")
        .active_tab();
    assert_eq!(tab.panes().len(), 2);
    assert_eq!(tab.focused_pane().id(), second_pane_id);
    assert_eq!(tab.focus_history(), &[first_pane_id]);
    assert_eq!(tab.focused_pane().cwd(), Some(pane_cwd.as_path()));
    match tab.layout() {
        PaneLayout::Split {
            direction,
            ratio,
            first,
            second,
        } => {
            assert_eq!(*direction, SplitDirection::Horizontal);
            assert_eq!(*ratio, 0.75);
            assert_eq!(**first, PaneLayout::Pane(first_pane_id));
            assert_eq!(**second, PaneLayout::Pane(second_pane_id));
        }
        PaneLayout::Pane(_) => panic!("split creates a split node"),
    }
}

#[test]
fn closing_panes_uses_focus_history_and_cascades_to_an_empty_session() {
    let mut session = Session::new();
    let workspace_id = session.create_workspace(PathBuf::from("projects/murmur"));
    let tab_id = session
        .active_workspace()
        .expect("workspace is active")
        .active_tab()
        .id();
    let first_pane_id = session
        .active_workspace()
        .expect("workspace is active")
        .active_tab()
        .focused_pane()
        .id();
    let second_pane_id = session
        .split_pane(first_pane_id, SplitDirection::Horizontal, 0.5)
        .expect("pane exists");
    assert!(session.focus_pane(first_pane_id));
    let third_pane_id = session
        .split_pane(first_pane_id, SplitDirection::Vertical, 0.5)
        .expect("pane exists");

    let closed = session.close_pane(third_pane_id).expect("pane exists");
    assert_eq!(closed.panes(), &[third_pane_id]);
    assert!(closed.tabs().is_empty());
    assert!(closed.workspaces().is_empty());
    assert_eq!(
        session
            .active_workspace()
            .expect("workspace remains")
            .active_tab()
            .focused_pane()
            .id(),
        first_pane_id
    );

    session.close_pane(first_pane_id).expect("pane exists");
    assert_eq!(
        session
            .active_workspace()
            .expect("workspace remains")
            .active_tab()
            .focused_pane()
            .id(),
        second_pane_id
    );

    let closed = session.close_pane(second_pane_id).expect("pane exists");
    assert_eq!(closed.panes(), &[second_pane_id]);
    assert_eq!(closed.tabs(), &[tab_id]);
    assert_eq!(closed.workspaces(), &[workspace_id]);
    assert!(session.is_empty());
    assert_eq!(session.active_workspace_id(), None);
}

#[test]
fn reordering_workspaces_and_tabs_preserves_active_identity() {
    let mut session = Session::new();
    let first_workspace_id = session.create_workspace(PathBuf::from("projects/first"));
    let second_workspace_id = session.create_workspace(PathBuf::from("projects/second"));
    let first_tab_id = session
        .active_workspace()
        .expect("second workspace is active")
        .active_tab()
        .id();
    let second_tab_id = session
        .create_tab(second_workspace_id)
        .expect("workspace exists");

    assert!(session.move_workspace(second_workspace_id, 0));
    assert!(session.move_tab(second_tab_id, 0));

    assert_eq!(
        session
            .workspaces()
            .iter()
            .map(|workspace| workspace.id())
            .collect::<Vec<_>>(),
        [second_workspace_id, first_workspace_id]
    );
    assert_eq!(session.active_workspace_id(), Some(second_workspace_id));
    let workspace = session
        .active_workspace()
        .expect("workspace remains active");
    assert_eq!(
        workspace
            .tabs()
            .iter()
            .map(|tab| tab.id())
            .collect::<Vec<_>>(),
        [second_tab_id, first_tab_id]
    );
    assert_eq!(workspace.active_tab().id(), second_tab_id);
}

#[test]
fn snapshot_round_trip_preserves_structural_domain_state() {
    let mut session = Session::new();
    let root_directory = PathBuf::from("projects/murmur");
    let first_workspace_id = session.create_workspace(root_directory.clone());
    let first_tab_id = session
        .active_workspace()
        .expect("workspace is active")
        .active_tab()
        .id();
    let second_tab_id = session
        .create_tab(first_workspace_id)
        .expect("workspace exists");
    let second_tab_root = session
        .active_workspace()
        .expect("workspace is active")
        .active_tab()
        .focused_pane()
        .id();
    let pane_cwd = PathBuf::from("projects/murmur/crates/murmur-core");
    session.set_pane_cwd(second_tab_root, Some(pane_cwd.clone()));
    let split_pane_id = session
        .split_pane(second_tab_root, SplitDirection::Vertical, 0.65)
        .expect("pane exists");
    session.focus_pane(second_tab_root);

    let second_workspace_id = session.create_workspace(PathBuf::from("projects/other"));
    session.move_workspace(second_workspace_id, 0);

    let snapshot = session.snapshot();
    let bytes = snapshot.to_bytes().expect("snapshot encodes");
    let decoded = SessionSnapshot::from_bytes(&bytes).expect("snapshot decodes");
    let restored = Session::restore(decoded).expect("snapshot is valid");

    assert_eq!(restored.active_workspace_id(), Some(second_workspace_id));
    assert_eq!(
        restored
            .workspaces()
            .iter()
            .map(|workspace| workspace.id())
            .collect::<Vec<_>>(),
        [second_workspace_id, first_workspace_id]
    );

    let workspace = restored
        .workspace(first_workspace_id)
        .expect("first workspace restores");
    assert_eq!(workspace.name(), "murmur");
    assert_eq!(workspace.root_directory(), root_directory.as_path());
    assert_eq!(
        workspace
            .tabs()
            .iter()
            .map(|tab| tab.id())
            .collect::<Vec<_>>(),
        [first_tab_id, second_tab_id]
    );
    let tab = workspace.active_tab();
    assert_eq!(tab.id(), second_tab_id);
    assert_eq!(tab.focused_pane().id(), second_tab_root);
    assert_eq!(tab.focus_history(), &[split_pane_id]);
    assert_eq!(tab.focused_pane().cwd(), Some(pane_cwd.as_path()));
    match tab.layout() {
        PaneLayout::Split {
            direction,
            ratio,
            first,
            second,
        } => {
            assert_eq!(*direction, SplitDirection::Vertical);
            assert_eq!(*ratio, 0.65);
            assert_eq!(**first, PaneLayout::Pane(second_tab_root));
            assert_eq!(**second, PaneLayout::Pane(split_pane_id));
        }
        PaneLayout::Pane(_) => panic!("split layout restores"),
    }
    assert_eq!(restored.snapshot(), snapshot);
}

#[test]
fn activating_a_tab_or_pane_activates_its_owners() {
    let mut session = Session::new();
    let first_workspace_id = session.create_workspace(PathBuf::from("projects/first"));
    let first_tab_id = session
        .active_workspace()
        .expect("workspace is active")
        .active_tab()
        .id();
    let first_pane_id = session
        .active_workspace()
        .expect("workspace is active")
        .active_tab()
        .focused_pane()
        .id();
    let second_tab_id = session
        .create_tab(first_workspace_id)
        .expect("workspace exists");
    let second_workspace_id = session.create_workspace(PathBuf::from("projects/second"));
    assert_eq!(session.active_workspace_id(), Some(second_workspace_id));

    assert!(session.activate_tab(second_tab_id));
    assert_eq!(session.active_workspace_id(), Some(first_workspace_id));
    assert_eq!(
        session
            .active_workspace()
            .expect("first workspace is active")
            .active_tab()
            .id(),
        second_tab_id
    );

    assert!(session.activate_workspace(second_workspace_id));
    assert!(session.focus_pane(first_pane_id));
    assert_eq!(session.active_workspace_id(), Some(first_workspace_id));
    assert_eq!(
        session
            .active_workspace()
            .expect("first workspace is active")
            .active_tab()
            .id(),
        first_tab_id
    );
}

#[test]
fn closing_tabs_and_workspaces_reports_every_removed_domain_object() {
    let mut session = Session::new();
    let first_workspace_id = session.create_workspace(PathBuf::from("projects/first"));
    let first_tab_id = session
        .active_workspace()
        .expect("workspace is active")
        .active_tab()
        .id();
    let second_tab_id = session
        .create_tab(first_workspace_id)
        .expect("workspace exists");
    let second_tab_pane_id = session
        .active_workspace()
        .expect("workspace is active")
        .active_tab()
        .focused_pane()
        .id();
    let second_workspace_id = session.create_workspace(PathBuf::from("projects/second"));
    let second_workspace_tab_id = session
        .active_workspace()
        .expect("workspace is active")
        .active_tab()
        .id();
    let second_workspace_pane_id = session
        .active_workspace()
        .expect("workspace is active")
        .active_tab()
        .focused_pane()
        .id();

    let closed = session.close_tab(second_tab_id).expect("tab exists");
    assert_eq!(closed.panes(), &[second_tab_pane_id]);
    assert_eq!(closed.tabs(), &[second_tab_id]);
    assert!(closed.workspaces().is_empty());
    assert_eq!(
        session
            .workspace(first_workspace_id)
            .expect("workspace remains")
            .active_tab()
            .id(),
        first_tab_id
    );

    let closed = session
        .close_workspace(second_workspace_id)
        .expect("workspace exists");
    assert_eq!(closed.panes(), &[second_workspace_pane_id]);
    assert_eq!(closed.tabs(), &[second_workspace_tab_id]);
    assert_eq!(closed.workspaces(), &[second_workspace_id]);
    assert_eq!(session.active_workspace_id(), Some(first_workspace_id));
}

#[test]
fn creating_a_tab_activates_its_workspace() {
    let mut session = Session::new();
    let first_workspace_id = session.create_workspace(PathBuf::from("projects/first"));
    session.create_workspace(PathBuf::from("projects/second"));

    let tab_id = session
        .create_tab(first_workspace_id)
        .expect("workspace exists");

    assert_eq!(session.active_workspace_id(), Some(first_workspace_id));
    assert_eq!(
        session
            .active_workspace()
            .expect("first workspace is active")
            .active_tab()
            .id(),
        tab_id
    );
}

#[test]
fn splitting_a_pane_activates_its_workspace_and_tab() {
    let mut session = Session::new();
    let first_workspace_id = session.create_workspace(PathBuf::from("projects/first"));
    let first_pane_id = session
        .active_workspace()
        .expect("workspace is active")
        .active_tab()
        .focused_pane()
        .id();
    session.create_workspace(PathBuf::from("projects/second"));

    let split_pane_id = session
        .split_pane(first_pane_id, SplitDirection::Horizontal, 0.5)
        .expect("pane exists");

    assert_eq!(session.active_workspace_id(), Some(first_workspace_id));
    assert_eq!(
        session
            .active_workspace()
            .expect("first workspace is active")
            .active_tab()
            .focused_pane()
            .id(),
        split_pane_id
    );
}

#[test]
fn unknown_pane_cwd_falls_back_to_the_stable_workspace_root() {
    let mut session = Session::new();
    let root_directory = PathBuf::from("projects/murmur");
    let workspace_id = session.create_workspace(root_directory.clone());
    let first_pane_id = session
        .active_workspace()
        .expect("workspace is active")
        .active_tab()
        .focused_pane()
        .id();
    session.set_pane_cwd(first_pane_id, None);

    session.create_tab(workspace_id).expect("workspace exists");
    let tab_root = session
        .active_workspace()
        .expect("workspace is active")
        .active_tab()
        .focused_pane()
        .id();
    assert_eq!(
        session
            .active_workspace()
            .expect("workspace is active")
            .active_tab()
            .focused_pane()
            .cwd(),
        Some(root_directory.as_path())
    );

    session.set_pane_cwd(tab_root, None);
    let split_pane_id = session
        .split_pane(tab_root, SplitDirection::Horizontal, 0.5)
        .expect("pane exists");
    let split_pane = session
        .active_workspace()
        .expect("workspace is active")
        .active_tab()
        .panes()
        .iter()
        .find(|pane| pane.id() == split_pane_id)
        .expect("split Pane exists");
    assert_eq!(split_pane.cwd(), Some(root_directory.as_path()));
}

#[test]
fn pane_layout_commands_preserve_focus_and_keep_zoom_runtime_only() {
    let mut session = Session::new();
    session.create_workspace(PathBuf::from("projects/murmur"));
    let first = session
        .active_workspace()
        .unwrap()
        .active_tab()
        .focused_pane()
        .id();
    let tab_id = session.active_workspace().unwrap().active_tab().id();
    let second = session
        .split_pane(first, SplitDirection::Horizontal, 0.5)
        .unwrap();
    let third = session
        .split_pane(second, SplitDirection::Vertical, 0.5)
        .unwrap();

    assert!(session.focus_pane(first));
    assert!(session.focus_pane_in_direction(first, PaneDirection::Right));
    assert_eq!(
        session
            .active_workspace()
            .unwrap()
            .active_tab()
            .focused_pane()
            .id(),
        second
    );
    assert!(session.resize_pane(second, PaneDirection::Left, 0.1));
    assert!(session.swap_pane(second, PaneDirection::Down));
    assert_eq!(
        session
            .active_workspace()
            .unwrap()
            .active_tab()
            .focused_pane()
            .id(),
        second
    );
    assert!(session.toggle_pane_zoom(third));
    let tab = session.active_workspace().unwrap().active_tab();
    assert_eq!(tab.focused_pane().id(), third);
    assert_eq!(tab.zoomed_pane_id(), Some(third));

    assert!(session.set_tab_split_ratios(tab_id, &[0.7, 0.3]));
    let restored = Session::restore(session.snapshot()).unwrap();
    assert_eq!(
        restored
            .active_workspace()
            .unwrap()
            .active_tab()
            .zoomed_pane_id(),
        None
    );
}

#[test]
fn restore_rejects_an_unsupported_snapshot_version() {
    #[derive(serde::Serialize)]
    struct UnsupportedSnapshot {
        version: u32,
        workspaces: Vec<()>,
        active_workspace: Option<WorkspaceId>,
    }

    let bytes = bincode::serialize(&UnsupportedSnapshot {
        version: 2,
        workspaces: Vec::new(),
        active_workspace: None,
    })
    .expect("test snapshot encodes");
    let snapshot = SessionSnapshot::from_bytes(&bytes).expect("schema decodes");

    assert_eq!(
        Session::restore(snapshot).expect_err("version is unsupported"),
        SnapshotError::UnsupportedVersion(2)
    );
}
