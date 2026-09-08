use std::path::PathBuf;

use condr_core::{Session, SessionSnapshot, SplitDirection};
use serde::Serialize;

const MAX_STABLE_ID: u64 = u64::MAX / 2;

#[test]
fn restored_maximum_stable_id_blocks_all_new_allocations_without_mutation() {
    let bytes = bincode::serialize(&EncodedSession {
        version: 1,
        workspaces: vec![EncodedWorkspace {
            id: MAX_STABLE_ID - 2,
            name: "boundary".into(),
            root_directory: PathBuf::from("projects/boundary"),
            worktree: None,
            tabs: vec![EncodedTab {
                id: MAX_STABLE_ID - 1,
                name: String::new(),
                panes: vec![EncodedPane {
                    id: MAX_STABLE_ID,
                    cwd: Some(PathBuf::from("projects/boundary")),
                    agent_resume: None,
                }],
                focused_pane: MAX_STABLE_ID,
                focus_history: Vec::new(),
                layout: EncodedLayout {
                    root: 0,
                    nodes: vec![EncodedLayoutNode::Pane(MAX_STABLE_ID)],
                },
            }],
            active_tab: MAX_STABLE_ID - 1,
        }],
        active_workspace: Some(MAX_STABLE_ID - 2),
    })
    .expect("boundary Snapshot encodes");
    let snapshot = SessionSnapshot::from_bytes(&bytes).expect("boundary schema decodes");
    let mut session = Session::restore(snapshot).expect("maximum stable ID is valid");
    let workspace_id = session.active_workspace_id().unwrap();
    let pane_id = session
        .active_workspace()
        .unwrap()
        .active_tab()
        .focused_pane()
        .id();
    let before = session.snapshot();

    assert_eq!(
        session.create_workspace(PathBuf::from("projects/overflow")),
        None
    );
    assert_eq!(session.create_tab(workspace_id), None);
    assert_eq!(
        session.split_pane(pane_id, SplitDirection::Horizontal, 0.5),
        None
    );
    assert_eq!(session.snapshot(), before);
}

#[derive(Serialize)]
struct EncodedSession {
    version: u32,
    workspaces: Vec<EncodedWorkspace>,
    active_workspace: Option<u64>,
}

#[derive(Serialize)]
struct EncodedWorkspace {
    id: u64,
    name: String,
    root_directory: PathBuf,
    worktree: Option<()>,
    tabs: Vec<EncodedTab>,
    active_tab: u64,
}

#[derive(Serialize)]
struct EncodedTab {
    id: u64,
    name: String,
    panes: Vec<EncodedPane>,
    focused_pane: u64,
    focus_history: Vec<u64>,
    layout: EncodedLayout,
}

#[derive(Serialize)]
struct EncodedPane {
    id: u64,
    cwd: Option<PathBuf>,
    agent_resume: Option<condr_core::AgentResume>,
}

#[derive(Serialize)]
struct EncodedLayout {
    root: u32,
    nodes: Vec<EncodedLayoutNode>,
}

#[derive(Serialize)]
enum EncodedLayoutNode {
    Pane(u64),
}
