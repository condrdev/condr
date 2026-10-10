use super::*;
use std::hint::black_box;
use std::time::Instant;

pub(super) fn duplicate_workspaces(
    snapshot: &condr_core::SessionSnapshot,
) -> condr_core::SessionSnapshot {
    let bytes = snapshot.to_bytes().unwrap();
    // Concatenating protobuf messages appends their repeated Workspace fields.
    condr_core::SessionSnapshot::from_bytes(&[bytes.as_slice(), bytes.as_slice()].concat()).unwrap()
}

fn bootstrap(session: &Session) -> SessionBootstrap {
    SessionBootstrap {
        settings: Default::default(),
        server_id: ServerId(1),
        runtime_epoch: RuntimeEpoch(2),
        session_id: SessionId(3),
        sequence: 7,
        snapshot: session.snapshot(),
        terminals: Vec::new(),
        agents: Vec::new(),
        workspace_git: Vec::new(),
        zoomed_panes: Vec::new(),
    }
}

#[test]
fn invalid_models_preserve_the_last_valid_structure_and_terminal_state() {
    let mut session = Session::new();
    session.create_workspace(std::env::temp_dir()).unwrap();
    let workspace = &session.workspaces()[0];
    let workspace_id = workspace.id();
    let tab_id = workspace.tabs()[0].id();
    let pane_id = workspace.tabs()[0].focused_pane().unwrap().id();
    let mut connection = connection_with_io();
    assert!(connection.session().is_none());
    connection.apply_bootstrap(bootstrap(&session)).unwrap();
    connection.set_view(workspace_id, Some(tab_id));
    connection
        .model
        .terminal_titles
        .insert(pane_id, "keep title".into());
    connection.model.terminals.insert(
        pane_id,
        ClientTerminal {
            view: std::sync::Arc::new(terminal_view(11, "keep terminal")),
            exited: false,
        },
    );
    connection.model.zoomed_panes.insert(pane_id);
    let terminal = connection.model.terminals[&pane_id].view.clone();
    let mut invalid = bootstrap(&session);
    invalid.server_id = ServerId(100);
    invalid.runtime_epoch = RuntimeEpoch(200);
    invalid.session_id = SessionId(300);
    invalid.sequence = 99;
    invalid.snapshot = duplicate_workspaces(&invalid.snapshot);
    let invalid_layout = invalid.snapshot.clone();

    assert!(connection.apply_bootstrap(invalid).is_err());
    assert!(connection.apply_layout(invalid_layout, Vec::new()).is_err());
    assert_eq!(connection.session().unwrap().snapshot(), session.snapshot());
    assert_eq!(connection.model.server_id, Some(ServerId(1)));
    assert_eq!(connection.model.runtime_epoch, Some(RuntimeEpoch(2)));
    assert_eq!(connection.model.session_id, Some(SessionId(3)));
    assert_eq!(connection.model.sequence, 7);
    assert_eq!(connection.view_workspace, Some(workspace_id));
    assert_eq!(connection.view_tabs.get(&workspace_id), Some(&tab_id));
    assert!(connection.model.zoomed_panes.contains(&pane_id));
    assert_eq!(connection.model.terminal_titles[&pane_id], "keep title");
    assert!(std::sync::Arc::ptr_eq(
        &connection.model.terminals[&pane_id].view,
        &terminal
    ));
}

#[test]
fn borrowed_models_keep_local_dock_edits_out_of_the_authoritative_structure() {
    let mut session = Session::new();
    session.create_workspace(std::env::temp_dir()).unwrap();
    let workspace_id = session.workspaces()[0].id();
    let tab_id = session.workspaces()[0].tabs()[0].id();
    let pane_id = session.workspaces()[0].tabs()[0]
        .focused_pane()
        .unwrap()
        .id();
    session
        .split_pane(pane_id, condr_core::SplitDirection::Horizontal, 0.5)
        .unwrap();
    let mut connection = connection_with_io();
    connection.apply_bootstrap(bootstrap(&session)).unwrap();
    connection.set_view(workspace_id, Some(tab_id));

    let model = connection.session().unwrap();
    assert!(std::ptr::eq(model, connection.session().unwrap()));
    let authoritative_layout = connection.dock_projection();
    let mut local = model.clone();
    assert!(local.set_tab_split_ratios(tab_id, &[0.72]));
    assert_eq!(connection.dock_projection(), authoritative_layout);
    assert!(
        connection
            .apply_layout(local.snapshot(), Vec::new())
            .unwrap()
    );
    assert_eq!(
        connection.viewed(connection.session().unwrap()),
        Some((workspace_id, tab_id))
    );
    assert_eq!(
        connection.dock_projection(),
        local.tab(tab_id).unwrap().layout().cloned()
    );
}

#[test]
fn a_closed_pane_leaves_no_attention_behind() {
    let mut session = Session::new();
    session.create_workspace(std::env::temp_dir()).unwrap();
    let pane_id = session.workspaces()[0].tabs()[0]
        .focused_pane()
        .unwrap()
        .id();
    let split = session
        .split_pane(pane_id, condr_core::SplitDirection::Horizontal, 0.5)
        .unwrap();
    let mut connection = connection_with_io();
    connection.apply_bootstrap(bootstrap(&session)).unwrap();
    connection.model.attention.extend([pane_id, split]);

    session.close_pane(split).unwrap();
    connection
        .apply_layout(session.snapshot(), Vec::new())
        .unwrap();
    assert_eq!(connection.model.attention, [pane_id].into());
}

/// Run manually with `cargo test -p condr-gui session_model_read_cost -- --ignored --nocapture`.
#[test]
#[ignore = "reports model-query cost; timings are not a correctness assertion"]
fn session_model_read_cost() {
    let mut session = Session::new();
    for _ in 0..64 {
        session.create_workspace(std::env::temp_dir()).unwrap();
    }
    let snapshot = session.snapshot();
    let mut connection = connection_with_io();
    connection.apply_bootstrap(bootstrap(&session)).unwrap();
    let queries = 2_000;
    let start = Instant::now();
    for _ in 0..queries {
        black_box(
            Session::restore(black_box(&snapshot).clone())
                .unwrap()
                .workspaces()
                .len(),
        );
    }
    let restore_elapsed = start.elapsed();
    let start = Instant::now();
    for _ in 0..queries {
        black_box(black_box(&connection).session().unwrap().workspaces().len());
    }
    println!(
        "64 Workspaces / {queries} reads: snapshot restore {restore_elapsed:?}; borrowed model {:?}",
        start.elapsed()
    );
}
