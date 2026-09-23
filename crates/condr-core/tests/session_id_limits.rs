mod common;

use std::path::PathBuf;

use common::*;

use condr_core::{Session, SessionSnapshot, SplitDirection};

const MAX_STABLE_ID: u64 = u64::MAX / 2;

#[test]
fn restored_maximum_stable_id_blocks_all_new_allocations_without_mutation() {
    let bytes = encode(&EncodedSession {
        workspaces: vec![EncodedWorkspace {
            id: MAX_STABLE_ID - 2,
            name: "boundary".into(),
            root_directory: PathBuf::from("projects/boundary"),
            worktree: None,
            tabs: vec![EncodedTab {
                id: MAX_STABLE_ID - 1,
                name: String::new(),
                content: EncodedTabContent::Terminals {
                    panes: vec![EncodedPane {
                        id: MAX_STABLE_ID,
                        cwd: Some(PathBuf::from("projects/boundary")),
                    }],
                    focused_pane: MAX_STABLE_ID,
                    focus_history: Vec::new(),
                    layout: EncodedLayout {
                        root: 0,
                        nodes: vec![EncodedLayoutNode::Pane(MAX_STABLE_ID)],
                    },
                },
            }],
        }],
    });
    let snapshot = SessionSnapshot::from_bytes(&bytes).expect("boundary schema decodes");
    let mut session = Session::restore(snapshot).expect("maximum stable ID is valid");
    let workspace_id = session.workspaces()[0].id();
    let pane_id = session.workspaces()[0].tabs()[0]
        .focused_pane()
        .unwrap()
        .id();
    let before = session.snapshot();

    assert_eq!(
        session.create_workspace(PathBuf::from("projects/overflow")),
        None
    );
    assert_eq!(session.create_tab(workspace_id, None), None);
    assert_eq!(
        session.split_pane(pane_id, SplitDirection::Horizontal, 0.5),
        None
    );
    assert_eq!(session.snapshot(), before);
}
