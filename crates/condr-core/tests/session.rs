use std::path::PathBuf;

use condr_core::{
    PaneDirection, PaneLayout, Session, SessionSnapshot, SnapshotError, SplitDirection, WorkspaceId,
};

#[test]
fn creating_workspace_commits_a_complete_initial_tree() {
    let mut session = Session::new();
    assert!(session.is_empty());

    let root_directory = PathBuf::from("projects/condr");
    let workspace_id = session
        .create_workspace(root_directory.clone())
        .expect("Workspace capacity");

    assert!(!session.is_empty());
    assert_eq!(session.active_workspace_id(), Some(workspace_id));
    assert_eq!(session.workspaces().len(), 1);

    let workspace = session.active_workspace().expect("workspace is active");
    assert_eq!(workspace.id(), workspace_id);
    assert_eq!(workspace.name(), "condr");
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
    let root_directory = PathBuf::from("projects/condr");
    let workspace_id = session
        .create_workspace(root_directory.clone())
        .expect("Workspace capacity");
    let first_pane_id = session
        .active_workspace()
        .expect("workspace is active")
        .active_tab()
        .focused_pane()
        .id();
    let pane_cwd = PathBuf::from("projects/condr/crates/condr-core");

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
    session
        .create_workspace(PathBuf::from("projects/condr"))
        .expect("Workspace capacity");
    let first_pane_id = session
        .active_workspace()
        .expect("workspace is active")
        .active_tab()
        .focused_pane()
        .id();
    let pane_cwd = PathBuf::from("projects/condr/crates/condr-gui");
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
    let workspace_id = session
        .create_workspace(PathBuf::from("projects/condr"))
        .expect("Workspace capacity");
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
    let first_workspace_id = session
        .create_workspace(PathBuf::from("projects/first"))
        .expect("Workspace capacity");
    let second_workspace_id = session
        .create_workspace(PathBuf::from("projects/second"))
        .expect("Workspace capacity");
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
    let root_directory = PathBuf::from("projects/condr");
    let first_workspace_id = session
        .create_workspace(root_directory.clone())
        .expect("Workspace capacity");
    let first_tab_id = session
        .active_workspace()
        .expect("workspace is active")
        .active_tab()
        .id();
    let second_tab_id = session
        .create_tab(first_workspace_id)
        .expect("workspace exists");
    assert!(session.rename_workspace(first_workspace_id, "Condr Core"));
    assert!(session.rename_tab(first_tab_id, "Overview"));
    assert!(session.rename_tab(second_tab_id, "Runtime"));
    assert!(session.move_tab(second_tab_id, 0));
    let second_tab_root = session
        .active_workspace()
        .expect("workspace is active")
        .active_tab()
        .focused_pane()
        .id();
    let pane_cwd = PathBuf::from("projects/condr/crates/condr-core");
    session.set_pane_cwd(second_tab_root, Some(pane_cwd.clone()));
    let split_pane_id = session
        .split_pane(second_tab_root, SplitDirection::Vertical, 0.65)
        .expect("pane exists");
    session.focus_pane(second_tab_root);

    let second_workspace_id = session
        .create_workspace(PathBuf::from("projects/other"))
        .expect("Workspace capacity");
    assert!(session.rename_workspace(second_workspace_id, "Managed Worktree"));
    assert!(session.associate_worktree(
        second_workspace_id,
        first_workspace_id,
        root_directory.clone(),
        true,
    ));
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
    assert_eq!(workspace.name(), "Condr Core");
    assert_eq!(workspace.root_directory(), root_directory.as_path());
    assert_eq!(
        workspace
            .tabs()
            .iter()
            .map(|tab| tab.id())
            .collect::<Vec<_>>(),
        [second_tab_id, first_tab_id]
    );
    let tab = workspace.active_tab();
    assert_eq!(tab.id(), second_tab_id);
    assert_eq!(tab.name(), "Runtime");
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
    let worktree = restored
        .workspace(second_workspace_id)
        .and_then(|workspace| workspace.worktree())
        .expect("managed worktree association restores");
    assert_eq!(worktree.parent_workspace_id(), first_workspace_id);
    assert_eq!(worktree.parent_root_directory(), root_directory.as_path());
    assert!(worktree.is_managed());
    assert_eq!(restored.snapshot(), snapshot);
}

#[test]
fn activating_a_tab_or_pane_activates_its_owners() {
    let mut session = Session::new();
    let first_workspace_id = session
        .create_workspace(PathBuf::from("projects/first"))
        .expect("Workspace capacity");
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
    let second_workspace_id = session
        .create_workspace(PathBuf::from("projects/second"))
        .expect("Workspace capacity");
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
    let first_workspace_id = session
        .create_workspace(PathBuf::from("projects/first"))
        .expect("Workspace capacity");
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
    let second_workspace_id = session
        .create_workspace(PathBuf::from("projects/second"))
        .expect("Workspace capacity");
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
    let first_workspace_id = session
        .create_workspace(PathBuf::from("projects/first"))
        .expect("Workspace capacity");
    session
        .create_workspace(PathBuf::from("projects/second"))
        .expect("Workspace capacity");

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
    let first_workspace_id = session
        .create_workspace(PathBuf::from("projects/first"))
        .expect("Workspace capacity");
    let first_pane_id = session
        .active_workspace()
        .expect("workspace is active")
        .active_tab()
        .focused_pane()
        .id();
    session
        .create_workspace(PathBuf::from("projects/second"))
        .expect("Workspace capacity");

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
    let root_directory = PathBuf::from("projects/condr");
    let workspace_id = session
        .create_workspace(root_directory.clone())
        .expect("Workspace capacity");
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
    session
        .create_workspace(PathBuf::from("projects/condr"))
        .expect("Workspace capacity");
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
        version: 3,
        workspaces: Vec::new(),
        active_workspace: None,
    })
    .expect("test snapshot encodes");
    let snapshot = SessionSnapshot::from_bytes(&bytes).expect("schema decodes");

    assert_eq!(
        Session::restore(snapshot).expect_err("version is unsupported"),
        SnapshotError::UnsupportedVersion(3)
    );
}

#[test]
fn snapshot_decode_rejects_trailing_bytes() {
    let mut bytes = Session::new().snapshot().to_bytes().unwrap();
    bytes.extend_from_slice(b"trailing corruption");

    assert!(SessionSnapshot::from_bytes(&bytes).is_err());
}

#[test]
fn snapshot_codec_enforces_the_eight_mebibyte_limit_in_both_directions() {
    const LIMIT: usize = 8 * 1024 * 1024;

    assert!(SessionSnapshot::from_bytes(&vec![0; LIMIT + 1]).is_err());

    let mut session = Session::new();
    session
        .create_workspace(PathBuf::from("projects/condr"))
        .expect("Workspace capacity");
    let pane_id = session
        .active_workspace()
        .unwrap()
        .active_tab()
        .focused_pane()
        .id();
    assert!(session.set_pane_cwd(pane_id, Some(PathBuf::from("x".repeat(LIMIT)))));
    assert!(session.snapshot().to_bytes().is_err());
}

#[test]
fn restore_rejects_exhausted_tab_numbers_and_stable_ids() {
    let mut session = Session::new();
    let workspace_id = session
        .create_workspace(PathBuf::from("projects/condr"))
        .expect("Workspace capacity");
    let tab = session.active_workspace().unwrap().active_tab();
    let tab_id = tab.id().as_u64();
    let pane_id = tab.focused_pane().id().as_u64();
    let exhausted_tab_number =
        encoded_snapshot(workspace_id.as_u64(), tab_id, pane_id, u64::MAX - 1);
    let exhausted_stable_id = encoded_snapshot(u64::MAX - 2, tab_id, pane_id, 2);

    for bytes in [exhausted_tab_number, exhausted_stable_id] {
        let snapshot = SessionSnapshot::from_bytes(&bytes).expect("schema decodes");
        assert!(matches!(
            Session::restore(snapshot),
            Err(SnapshotError::Invalid(_))
        ));
    }
}

#[test]
fn restore_rejects_invalid_historical_worktree_parent_ids() {
    for parent_workspace_id in [0, u64::MAX / 2 + 1] {
        let mut workspace =
            encoded_workspace(10, vec![encoded_tab(11, vec![12], EncodedLayout::pane(12))]);
        workspace.worktree = Some(EncodedWorktree {
            parent_workspace_id,
            parent_root_directory: PathBuf::from("projects/parent"),
            managed: true,
        });
        let snapshot = SessionSnapshot::from_bytes(&encode_session(vec![workspace])).unwrap();

        assert_eq!(
            Session::restore(snapshot).expect_err("historical parent ID is invalid"),
            SnapshotError::Invalid("invalid stable ID")
        );
    }
}

#[test]
fn create_tab_refuses_to_advance_an_extreme_counter() {
    let bytes = encoded_snapshot(10, 11, 12, u64::MAX - 2);
    let snapshot = SessionSnapshot::from_bytes(&bytes).expect("schema decodes");
    let mut session = Session::restore(snapshot).expect("counter still has a valid value");
    let before = session.snapshot();

    assert_eq!(
        session.create_tab(session.active_workspace_id().unwrap()),
        None
    );
    assert_eq!(session.snapshot(), before);
}

#[test]
fn live_mutations_enforce_workspace_and_tab_limits() {
    let mut workspaces = Session::new();
    for index in 0..64 {
        workspaces
            .create_workspace(PathBuf::from(format!("projects/workspace-{index}")))
            .expect("the first 64 Workspaces fit");
    }
    let before = workspaces.snapshot();
    assert_eq!(
        workspaces.create_workspace(PathBuf::from("projects/overflow")),
        None
    );
    assert_eq!(workspaces.snapshot(), before);
    Session::restore(before).expect("the Workspace boundary restores");

    let mut tabs = Session::new();
    let workspace_id = tabs
        .create_workspace(PathBuf::from("projects/tabs"))
        .expect("Workspace capacity");
    for _ in 1..256 {
        tabs.create_tab(workspace_id)
            .expect("the first 256 Tabs fit");
    }
    let before = tabs.snapshot();
    assert_eq!(tabs.create_tab(workspace_id), None);
    assert_eq!(
        tabs.create_workspace(PathBuf::from("projects/tab-overflow")),
        None
    );
    assert_eq!(tabs.snapshot(), before);
    Session::restore(before).expect("the Tab boundary restores");
}

#[test]
fn live_mutations_enforce_the_total_pane_limit() {
    let mut session = Session::new();
    session
        .create_workspace(PathBuf::from("projects/panes"))
        .expect("Workspace capacity");
    let first_pane = session
        .active_workspace()
        .unwrap()
        .active_tab()
        .focused_pane()
        .id();
    let mut leaves = vec![first_pane];
    for _ in 0..10 {
        let current = leaves.clone();
        for pane_id in current {
            leaves.push(
                session
                    .split_pane(pane_id, SplitDirection::Horizontal, 0.5)
                    .expect("the first 1,024 Panes fit"),
            );
        }
    }

    assert_eq!(leaves.len(), 1_024);
    let before = session.snapshot();
    assert_eq!(
        session.split_pane(first_pane, SplitDirection::Horizontal, 0.5),
        None
    );
    assert_eq!(
        session.create_tab(session.active_workspace_id().unwrap()),
        None
    );
    assert_eq!(
        session.create_workspace(PathBuf::from("projects/pane-overflow")),
        None
    );
    assert_eq!(session.snapshot(), before);
    Session::restore(before).expect("the Pane boundary restores");
}

#[test]
fn live_split_refuses_to_exceed_the_restore_depth() {
    let mut session = Session::new();
    session
        .create_workspace(PathBuf::from("projects/depth"))
        .expect("Workspace capacity");
    let mut deepest = session
        .active_workspace()
        .unwrap()
        .active_tab()
        .focused_pane()
        .id();
    for _ in 1..64 {
        deepest = session
            .split_pane(deepest, SplitDirection::Vertical, 0.5)
            .expect("a depth of 64 fits");
    }

    let before = session.snapshot();
    assert_eq!(
        session.split_pane(deepest, SplitDirection::Vertical, 0.5),
        None
    );
    assert_eq!(session.snapshot(), before);
    Session::restore(before).expect("the depth boundary restores");
}

#[test]
fn snapshot_decode_rejects_more_than_64_workspaces() {
    let workspaces = (0..65)
        .map(|index| {
            let base = 100 + index * 3;
            encoded_workspace(
                base,
                vec![encoded_tab(
                    base + 1,
                    vec![base + 2],
                    EncodedLayout::pane(base + 2),
                )],
            )
        })
        .collect();
    let bytes = encode_session(workspaces);

    assert!(SessionSnapshot::from_bytes(&bytes).is_err());
}

#[test]
fn snapshot_decode_rejects_more_than_2047_layout_nodes() {
    let layout = EncodedLayout {
        root: 0,
        nodes: (0..2_048).map(|_| EncodedLayoutNode::Pane(12)).collect(),
    };
    let bytes = encode_session(vec![encoded_workspace(
        10,
        vec![encoded_tab(11, vec![12], layout)],
    )]);

    assert!(SessionSnapshot::from_bytes(&bytes).is_err());
}

#[test]
fn restore_rejects_more_than_256_total_tabs() {
    let mut next_tab_id = 1_000;
    let mut next_pane_id = 10_000;
    let workspaces = [128, 129]
        .into_iter()
        .enumerate()
        .map(|(workspace_index, count)| {
            let tabs = (0..count)
                .map(|_| {
                    let tab_id = next_tab_id;
                    let pane_id = next_pane_id;
                    next_tab_id += 1;
                    next_pane_id += 1;
                    encoded_tab(tab_id, vec![pane_id], EncodedLayout::pane(pane_id))
                })
                .collect();
            encoded_workspace(100 + workspace_index as u64, tabs)
        })
        .collect();
    let snapshot = SessionSnapshot::from_bytes(&encode_session(workspaces)).unwrap();

    assert_eq!(
        Session::restore(snapshot).expect_err("total Tab count is bounded"),
        SnapshotError::Invalid("too many Tabs")
    );
}

#[test]
fn restore_rejects_more_than_1024_total_panes() {
    let first_panes = (10_000..10_512).collect::<Vec<_>>();
    let second_panes = (20_000..20_513).collect::<Vec<_>>();
    let workspaces = vec![encoded_workspace(
        100,
        vec![
            encoded_tab(
                1_000,
                first_panes.clone(),
                EncodedLayout::pane(first_panes[0]),
            ),
            encoded_tab(
                1_001,
                second_panes.clone(),
                EncodedLayout::pane(second_panes[0]),
            ),
        ],
    )];
    let snapshot = SessionSnapshot::from_bytes(&encode_session(workspaces)).unwrap();

    assert_eq!(
        Session::restore(snapshot).expect_err("total Pane count is bounded"),
        SnapshotError::Invalid("too many Panes")
    );
}

#[test]
fn restore_rejects_cycles_out_of_bounds_duplicates_and_unreachable_nodes() {
    let invalid_layouts = [
        (
            EncodedLayout {
                root: 0,
                nodes: vec![
                    EncodedLayoutNode::split(0, 1),
                    EncodedLayoutNode::Pane(11),
                    EncodedLayoutNode::Pane(12),
                ],
            },
            "cycle in layout",
        ),
        (
            EncodedLayout {
                root: 0,
                nodes: vec![
                    EncodedLayoutNode::split(99, 1),
                    EncodedLayoutNode::Pane(11),
                    EncodedLayoutNode::Pane(12),
                ],
            },
            "layout node is out of bounds",
        ),
        (
            EncodedLayout {
                root: 0,
                nodes: vec![
                    EncodedLayoutNode::split(1, 1),
                    EncodedLayoutNode::Pane(11),
                    EncodedLayoutNode::Pane(12),
                ],
            },
            "duplicate layout node reference",
        ),
        (
            EncodedLayout {
                root: 0,
                nodes: vec![
                    EncodedLayoutNode::split(1, 2),
                    EncodedLayoutNode::Pane(11),
                    EncodedLayoutNode::Pane(12),
                    EncodedLayoutNode::Pane(13),
                    EncodedLayoutNode::Pane(14),
                ],
            },
            "unreachable layout node",
        ),
    ];

    for (layout, reason) in invalid_layouts {
        let pane_ids = match reason {
            "unreachable layout node" => vec![11, 12, 13],
            _ => vec![11, 12],
        };
        let session = vec![encoded_workspace(1, vec![encoded_tab(2, pane_ids, layout)])];
        let snapshot = SessionSnapshot::from_bytes(&encode_session(session)).unwrap();
        assert_eq!(
            Session::restore(snapshot).expect_err("layout graph is invalid"),
            SnapshotError::Invalid(reason)
        );
    }
}

#[test]
fn restore_enforces_a_64_level_layout_depth_limit() {
    let (pane_ids, accepted_layout) = encoded_chain_layout(64, 1_000);
    let accepted = vec![encoded_workspace(
        1,
        vec![encoded_tab(2, pane_ids, accepted_layout)],
    )];
    let snapshot = SessionSnapshot::from_bytes(&encode_session(accepted)).unwrap();
    assert!(Session::restore(snapshot).is_ok());

    let (pane_ids, rejected_layout) = encoded_chain_layout(65, 2_000);
    let rejected = vec![encoded_workspace(
        3,
        vec![encoded_tab(4, pane_ids, rejected_layout)],
    )];
    let snapshot = SessionSnapshot::from_bytes(&encode_session(rejected)).unwrap();
    assert_eq!(
        Session::restore(snapshot).expect_err("layout is too deep"),
        SnapshotError::Invalid("layout exceeds maximum depth")
    );
}

fn encoded_snapshot(workspace_id: u64, tab_id: u64, pane_id: u64, next_tab_number: u64) -> Vec<u8> {
    let root = PathBuf::from("projects/condr");
    bincode::serialize(&EncodedSession {
        version: 1,
        workspaces: vec![EncodedWorkspace {
            id: workspace_id,
            name: "condr".into(),
            root_directory: root.clone(),
            worktree: None,
            tabs: vec![EncodedTab {
                id: tab_id,
                name: "Tab 1".into(),
                panes: vec![EncodedPane {
                    id: pane_id,
                    cwd: Some(root),
                }],
                focused_pane: pane_id,
                focus_history: Vec::new(),
                layout: EncodedLayout::pane(pane_id),
            }],
            active_tab: tab_id,
            next_tab_number,
        }],
        active_workspace: Some(workspace_id),
    })
    .unwrap()
}

fn encode_session(workspaces: Vec<EncodedWorkspace>) -> Vec<u8> {
    let active_workspace = workspaces.first().map(|workspace| workspace.id);
    bincode::serialize(&EncodedSession {
        version: 1,
        workspaces,
        active_workspace,
    })
    .unwrap()
}

fn encoded_workspace(id: u64, tabs: Vec<EncodedTab>) -> EncodedWorkspace {
    EncodedWorkspace {
        id,
        name: format!("Workspace {id}"),
        root_directory: PathBuf::from(format!("projects/{id}")),
        worktree: None,
        active_tab: tabs.first().expect("test Workspace has a Tab").id,
        next_tab_number: tabs.len() as u64 + 1,
        tabs,
    }
}

fn encoded_tab(id: u64, pane_ids: Vec<u64>, layout: EncodedLayout) -> EncodedTab {
    EncodedTab {
        id,
        name: format!("Tab {id}"),
        focused_pane: pane_ids[0],
        panes: pane_ids
            .into_iter()
            .map(|id| EncodedPane { id, cwd: None })
            .collect(),
        focus_history: Vec::new(),
        layout,
    }
}

fn encoded_chain_layout(depth: usize, first_pane_id: u64) -> (Vec<u64>, EncodedLayout) {
    assert!(depth > 0);
    let split_count = depth - 1;
    let pane_ids = (0..depth)
        .map(|offset| first_pane_id + offset as u64)
        .collect::<Vec<_>>();
    let mut nodes = Vec::with_capacity(depth * 2 - 1);
    for split in 0..split_count {
        let first = if split + 1 < split_count {
            split + 1
        } else {
            split_count
        };
        nodes.push(EncodedLayoutNode::split(
            first as u32,
            (split_count + 1 + split) as u32,
        ));
    }
    nodes.extend(pane_ids.iter().copied().map(EncodedLayoutNode::Pane));
    (pane_ids, EncodedLayout { root: 0, nodes })
}

#[derive(serde::Serialize)]
struct EncodedSession {
    version: u32,
    workspaces: Vec<EncodedWorkspace>,
    active_workspace: Option<u64>,
}

#[derive(serde::Serialize)]
struct EncodedWorkspace {
    id: u64,
    name: String,
    root_directory: PathBuf,
    worktree: Option<EncodedWorktree>,
    tabs: Vec<EncodedTab>,
    active_tab: u64,
    next_tab_number: u64,
}

#[derive(serde::Serialize)]
struct EncodedWorktree {
    parent_workspace_id: u64,
    parent_root_directory: PathBuf,
    managed: bool,
}

#[derive(serde::Serialize)]
struct EncodedTab {
    id: u64,
    name: String,
    panes: Vec<EncodedPane>,
    focused_pane: u64,
    focus_history: Vec<u64>,
    layout: EncodedLayout,
}

#[derive(serde::Serialize)]
struct EncodedPane {
    id: u64,
    cwd: Option<PathBuf>,
}

#[derive(serde::Serialize)]
struct EncodedLayout {
    root: u32,
    nodes: Vec<EncodedLayoutNode>,
}

impl EncodedLayout {
    fn pane(pane_id: u64) -> Self {
        Self {
            root: 0,
            nodes: vec![EncodedLayoutNode::Pane(pane_id)],
        }
    }
}

#[derive(serde::Serialize)]
enum EncodedLayoutNode {
    Pane(u64),
    Split {
        direction: SplitDirection,
        ratio: f32,
        first: u32,
        second: u32,
    },
}

impl EncodedLayoutNode {
    fn split(first: u32, second: u32) -> Self {
        Self::Split {
            direction: SplitDirection::Horizontal,
            ratio: 0.5,
            first,
            second,
        }
    }
}

fn only_split_ratio(session: &Session) -> f32 {
    match session.active_workspace().unwrap().active_tab().layout() {
        PaneLayout::Split { ratio, .. } => *ratio,
        PaneLayout::Pane(_) => panic!("the tab should hold one split"),
    }
}

#[test]
fn resizing_moves_the_divider_in_the_key_direction_from_either_side() {
    let mut session = Session::new();
    session
        .create_workspace(PathBuf::from("projects/condr"))
        .unwrap();
    let left = session
        .active_workspace()
        .unwrap()
        .active_tab()
        .focused_pane()
        .id();
    let right = session
        .split_pane(left, SplitDirection::Horizontal, 0.5)
        .expect("Pane capacity");

    // The left pane's right edge moves left: it shrinks.
    assert!(session.resize_pane(left, PaneDirection::Left, 0.1));
    assert!((only_split_ratio(&session) - 0.4).abs() < 1e-6);
    // The same key from the right pane moves the same divider the same way.
    assert!(session.resize_pane(right, PaneDirection::Left, 0.1));
    assert!((only_split_ratio(&session) - 0.3).abs() < 1e-6);
    // And Right brings it back from either side.
    assert!(session.resize_pane(right, PaneDirection::Right, 0.1));
    assert!(session.resize_pane(left, PaneDirection::Right, 0.1));
    assert!((only_split_ratio(&session) - 0.5).abs() < 1e-6);
    // Nothing to move across the other axis.
    assert!(!session.resize_pane(left, PaneDirection::Up, 0.1));
    assert!(!session.resize_pane(left, PaneDirection::Down, 0.1));

    let mut session = Session::new();
    session
        .create_workspace(PathBuf::from("projects/condr"))
        .unwrap();
    let top = session
        .active_workspace()
        .unwrap()
        .active_tab()
        .focused_pane()
        .id();
    let bottom = session
        .split_pane(top, SplitDirection::Vertical, 0.5)
        .expect("Pane capacity");
    assert!(session.resize_pane(top, PaneDirection::Up, 0.1));
    assert!((only_split_ratio(&session) - 0.4).abs() < 1e-6);
    assert!(session.resize_pane(bottom, PaneDirection::Down, 0.1));
    assert!((only_split_ratio(&session) - 0.5).abs() < 1e-6);
}
