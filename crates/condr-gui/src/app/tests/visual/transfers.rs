use super::*;
use crate::app::transfers::TransferState;
use condr_core::protocol::RelativePathBuf;
use condr_core::protocol::{LayoutCommand, LayoutResult};

/// Uploads and a download through the GUI (ADR 0061), over a TCP Device's Stream
/// connection: a batch lands under its names, the strip collects them and Clear empties
/// it once they finished, a download goes through the save dialog, and an upload onto a
/// name the directory lists asks once and replaces on Replace.
#[test]
fn files_upload_and_download_through_the_gui_and_collect_in_the_strip() {
    let _serial_guard = acquire_visual_test_lock();
    let root = std::env::temp_dir().join(format!(
        "condr-gui-transfers-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let workspace_root = root.join("workspace");
    let local = root.join("local");
    std::fs::create_dir_all(&workspace_root).unwrap();
    std::fs::create_dir_all(&local).unwrap();
    let payload: Vec<u8> = (0..1_500_000u32).map(|i| (i % 251) as u8).collect();
    std::fs::write(local.join("one.bin"), &payload).unwrap();
    std::fs::write(local.join("two.txt"), "two").unwrap();

    let mut cx = TestAppContext::single();
    cx.update(gpui_kit::init);
    let (server, endpoint) = start_tcp_server();
    let mut admin = ClientConnection::connect_overview(&endpoint, "admin").unwrap();
    let LayoutResult::WorkspaceCreated { workspace_id, .. } = admin
        .layout(LayoutCommand::CreateWorkspace {
            root_directory: workspace_root.clone(),
            name: None,
        })
        .unwrap()
        .unwrap()
    else {
        panic!("expected a Workspace");
    };
    let (view, window, _server) = connected_condr_with(&mut cx, server, endpoint);

    window.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.upload_files(
                1,
                workspace_id,
                RelativePathBuf::new(),
                vec![local.join("one.bin"), local.join("two.txt")],
                window,
                cx,
            )
        })
    });
    assert!(
        wait_until(window, |window| {
            window.read(|app| {
                view.read(app)
                    .transfer_states(1)
                    .iter()
                    .all(|state| *state == TransferState::Done)
            })
        }),
        "the uploads did not finish: {:?}",
        window.read(|app| view.read(app).transfer_states(1))
    );
    assert_eq!(
        std::fs::read(workspace_root.join("one.bin")).unwrap(),
        payload
    );
    assert_eq!(
        std::fs::read_to_string(workspace_root.join("two.txt")).unwrap(),
        "two"
    );

    // The strip shows while the Workspace's right sidebar is open; Clear empties it.
    window.update(|window, cx| {
        let _ = window;
        view.update(cx, |this, cx| this.toggle_changes_sidebar(cx))
    });
    window.update(|window, cx| _ = window.draw(cx));
    assert!(window.debug_bounds("transfer-strip").is_some());
    let clear = window
        .debug_bounds("transfer-clear")
        .expect("finished transfers offer Clear");
    window.simulate_click(clear.center(), Modifiers::default());
    window.run_until_parked();
    assert!(
        window
            .read(|app| view.read(app).transfer_states(1))
            .is_empty()
    );

    // A download asks where to save, then writes there.
    let saved = local.join("back.bin");
    window.update(|_, cx| {
        view.update(cx, |this, cx| {
            this.download_file(1, workspace_id, "one.bin".into(), cx)
        })
    });
    window.run_until_parked();
    assert!(window.did_prompt_for_new_path());
    let chosen = saved.clone();
    window.simulate_new_path_selection(move |_| Some(chosen));
    assert!(wait_until(window, |window| {
        window.read(|app| {
            view.read(app)
                .transfer_states(1)
                .first()
                .is_some_and(|state| *state == TransferState::Done)
        })
    }));
    assert_eq!(std::fs::read(&saved).unwrap(), payload);

    // The Files view lists the Root Directory by now, so an existing name is asked about
    // before anything is sent, and Replace overwrites it.
    std::fs::write(local.join("two.txt"), "changed").unwrap();
    let before = window.read(|app| view.read(app).transfer_states(1)).len();
    window.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.upload_files(
                1,
                workspace_id,
                RelativePathBuf::new(),
                vec![local.join("two.txt")],
                window,
                cx,
            )
        })
    });
    window.run_until_parked();
    assert_eq!(
        window.read(|app| view.read(app).transfer_states(1)).len(),
        before,
        "nothing is sent before the person answers"
    );
    confirm_alert_dialog(window);
    assert!(wait_until(window, |window| {
        window.read(|app| {
            view.read(app)
                .transfer_states(1)
                .last()
                .is_some_and(|state| *state == TransferState::Done)
        })
    }));
    assert_eq!(
        std::fs::read_to_string(workspace_root.join("two.txt")).unwrap(),
        "changed"
    );
    let _ = std::fs::remove_dir_all(&root);
}
