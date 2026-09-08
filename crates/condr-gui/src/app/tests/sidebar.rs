use super::*;

#[test]
fn tab_labels_follow_workspace_order_and_only_include_user_names() {
    let mut session = Session::new();
    let first_workspace = session.create_workspace("projects/first".into()).unwrap();
    let first_tab = session.active_workspace().unwrap().active_tab().id();
    let second_tab = session.create_tab(first_workspace).unwrap();
    let third_tab = session.create_tab(first_workspace).unwrap();
    let second_workspace = session.create_workspace("projects/second".into()).unwrap();
    let labels = |session: &Session, workspace_id| {
        session
            .workspace(workspace_id)
            .unwrap()
            .tabs()
            .iter()
            .enumerate()
            .map(|(index, tab)| crate::app::workspace::tab_label(index, tab.name()))
            .collect::<Vec<_>>()
    };
    assert_eq!(labels(&session, first_workspace), ["1", "2", "3"]);
    assert_eq!(labels(&session, second_workspace), ["1"]);

    // A user name that resembles an old default is still a real name.
    assert!(session.rename_tab(second_tab, "Tab 2"));
    assert_eq!(labels(&session, first_workspace), ["1", "2  Tab 2", "3"]);
    assert!(session.move_tab(second_tab, 0));
    assert_eq!(labels(&session, first_workspace), ["1  Tab 2", "2", "3"]);
    assert!(session.close_tab(first_tab).is_some());
    assert_eq!(labels(&session, first_workspace), ["1  Tab 2", "2"]);
    assert!(session.move_tab(third_tab, 0));
    assert_eq!(labels(&session, first_workspace), ["1", "2  Tab 2"]);

    let mut restored = Session::restore(session.snapshot()).unwrap();
    assert_eq!(labels(&restored, first_workspace), ["1", "2  Tab 2"]);
    assert_eq!(labels(&restored, second_workspace), ["1"]);
    restored.create_tab(first_workspace).unwrap();
    assert_eq!(labels(&restored, first_workspace), ["1", "2  Tab 2", "3"]);
}

#[test]
fn reordering_connections_moves_the_dragged_server_into_the_target_slot() {
    let endpoint = || tcp("127.0.0.1:4242");
    let connection = |key: u64| ServerConnection::new(key, format!("server-{key}"), endpoint());
    let keys =
        |connections: &Vec<ServerConnection>| connections.iter().map(|c| c.key).collect::<Vec<_>>();

    let mut connections = vec![connection(1), connection(2), connection(3)];
    assert!(reorder_connection(&mut connections, 3, 1, false));
    assert_eq!(keys(&connections), [3, 1, 2]);

    assert!(reorder_connection(&mut connections, 3, 2, true));
    assert_eq!(keys(&connections), [1, 2, 3]);

    assert!(
        !reorder_connection(&mut connections, 2, 2, false),
        "dropping a server on itself should not change the order"
    );
    assert!(
        !reorder_connection(&mut connections, 1, 2, false),
        "dropping just before the next item is where it already is"
    );
    assert!(
        !reorder_connection(&mut connections, 9, 1, false),
        "an unknown dragged key should be ignored"
    );
    assert!(
        !reorder_connection(&mut connections, 1, 9, true),
        "an unknown target key should be ignored"
    );
    assert_eq!(keys(&connections), [1, 2, 3]);
}

#[test]
fn drop_index_inserts_before_or_after_the_target_in_the_list_without_the_source() {
    use crate::app::sidebar::drop_index;
    // Moving right: the removal shifts the target left by one.
    assert_eq!(drop_index(0, 2, false), Some(1));
    assert_eq!(drop_index(0, 2, true), Some(2));
    // Moving left: indexes are unaffected by the removal.
    assert_eq!(drop_index(2, 0, false), Some(0));
    assert_eq!(drop_index(2, 0, true), Some(1));
    // Both sides of the source's own slot are no-ops.
    assert_eq!(drop_index(1, 1, false), None);
    assert_eq!(drop_index(1, 1, true), None);
    assert_eq!(drop_index(1, 0, true), None);
    assert_eq!(drop_index(1, 2, false), None);
}

#[test]
fn sidebar_status_visuals_follow_the_prototype_semantics() {
    let agent_cases = [
        (
            AgentDisplayState::Unknown,
            SidebarGlyph::Info,
            SidebarIconTone::Muted,
            "unknown",
        ),
        (
            AgentDisplayState::Idle,
            SidebarGlyph::Circle,
            SidebarIconTone::Muted,
            "idle",
        ),
        (
            AgentDisplayState::Working,
            SidebarGlyph::CircleFilled,
            SidebarIconTone::Warning,
            "working",
        ),
        (
            AgentDisplayState::Blocked,
            SidebarGlyph::CircleFilled,
            SidebarIconTone::Danger,
            "blocked",
        ),
        (
            AgentDisplayState::Done,
            SidebarGlyph::CircleFilled,
            SidebarIconTone::Success,
            "done",
        ),
    ];
    for (state, glyph, tone, key) in agent_cases {
        let visual = agent_sidebar_status(state);
        assert_eq!((visual.glyph, visual.tone, visual.key), (glyph, tone, key));
    }
}
