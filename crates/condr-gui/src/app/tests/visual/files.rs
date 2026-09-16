use super::*;

/// The right sidebar opens on Files for a Workspace outside a repository, lists the root
/// from the real Server, follows the Server's watcher, unfolds a directory on click, and a
/// file click opens the Preview Tab with the file's text (ADR 0018).
#[test]
fn files_sidebar_lists_the_root_unfolds_a_directory_and_opens_one_preview_tab() {
    let _serial_guard = acquire_visual_test_lock();
    let root = std::env::temp_dir().join(format!(
        "condr-gui-files-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("src/app")).unwrap();
    std::fs::write(root.join("README.md"), "# Files\n").unwrap();
    std::fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
    std::fs::write(root.join("src/app/mod.rs"), "pub mod app;\n").unwrap();

    let mut cx = TestAppContext::single();
    cx.update(gpui_kit::init);
    let (view, window, _server) = connected_condr(&mut cx);
    window.update(|window, cx| _ = window.draw(cx));

    let button = window.debug_bounds("open-project").unwrap();
    window.simulate_click(button.center(), Modifiers::default());
    assert!(window.did_prompt_for_paths());
    let selected_root = root.clone();
    window.simulate_path_prompt_response(move |_| Some(vec![selected_root]));
    assert!(wait_until(window, |window| {
        window.read(|app| {
            view.read(app)
                .active_session()
                .is_some_and(|session| session.active_workspace().is_some())
        })
    }));
    window.update(|window, cx| _ = window.draw(cx));
    let toggle = window.debug_bounds("toggle-changes").unwrap();
    window.simulate_click(toggle.center(), Modifiers::default());
    window.run_until_parked();
    window.update(|window, cx| _ = window.draw(cx));
    assert!(window.debug_bounds("condr-right-sidebar").is_some());
    // Outside a repository the sidebar opens on Files, since Changes has nothing to show;
    // the root listing arrives from the Server with directories first.
    assert!(
        wait_until(window, |window| {
            window.update(|window, cx| _ = window.draw(cx));
            window.debug_bounds("file-README.md").is_some()
                && window.debug_bounds("file-dir-src").is_some()
        }),
        "a Workspace outside a repository opens the sidebar on Files"
    );
    // The header tabs still reach Changes, which says why it is empty, and back.
    let changes_tab = window
        .debug_bounds("sidebar-view-changes")
        .expect("the header should offer the Changes view");
    window.simulate_click(changes_tab.center(), Modifiers::default());
    window.run_until_parked();
    window.update(|window, cx| _ = window.draw(cx));
    assert!(window.debug_bounds("condr-files-list").is_none());
    let files_tab = window
        .debug_bounds("sidebar-view-files")
        .expect("the header should offer the Files view");
    window.simulate_click(files_tab.center(), Modifiers::default());
    window.run_until_parked();
    window.update(|window, cx| _ = window.draw(cx));
    assert!(window.debug_bounds("condr-files-list").is_some());

    // The Server watches the root, repository or not: a new file shows up on its own.
    std::fs::write(root.join("NEW.txt"), "new\n").unwrap();
    assert!(
        wait_until(window, |window| {
            window.update(|window, cx| _ = window.draw(cx));
            window.debug_bounds("file-NEW.txt").is_some()
        }),
        "a file written after the listing should appear without a click"
    );
    let src = window.debug_bounds("file-dir-src").unwrap();
    let readme = window.debug_bounds("file-README.md").unwrap();
    assert!(src.top() < readme.top(), "directories sort before files");
    assert!(
        window.debug_bounds("file-src/main.rs").is_none(),
        "directories start folded"
    );

    // Unfolding a directory asks the Server for that level only.
    window.simulate_click(src.center(), Modifiers::default());
    assert!(
        wait_until(window, |window| {
            window.update(|window, cx| _ = window.draw(cx));
            window.debug_bounds("file-src/main.rs").is_some()
                && window.debug_bounds("file-dir-src/app").is_some()
        }),
        "the unfolded directory should list its entries"
    );
    assert!(window.debug_bounds("file-src/app/mod.rs").is_none());
    let main = window.debug_bounds("file-src/main.rs").unwrap();
    assert!(
        main.top() > src.top(),
        "children follow their directory row"
    );

    // Clicking a file opens the Preview Tab on the Server and the text reaches the Editor.
    window.simulate_click(main.center(), Modifiers::default());
    assert!(
        wait_until(window, |window| {
            window.read(|app| {
                view.read(app).active_session().is_some_and(|session| {
                    session.active_workspace().is_some_and(|workspace| {
                        workspace
                            .active_tab()
                            .file()
                            .is_some_and(|file| file.path() == std::path::Path::new("src/main.rs"))
                    })
                })
            })
        }),
        "the Preview Tab should become the active Tab"
    );
    assert!(
        wait_until(window, |window| {
            window.update(|window, cx| _ = window.draw(cx));
            window.debug_bounds("file-body").is_some()
                && window.update(|_, cx| {
                    view.read(cx)
                        .file_editors
                        .values()
                        .any(|editor| editor.state.read(cx).value().contains("fn main()"))
                })
        }),
        "the Server's file should reach the Editor"
    );
    let workspace_id = window.read(|app| {
        view.read(app)
            .active_session()
            .unwrap()
            .active_workspace()
            .unwrap()
            .id()
    });

    // A second file retargets the same Tab rather than adding one.
    let readme = window.debug_bounds("file-README.md").unwrap();
    window.simulate_click(readme.center(), Modifiers::default());
    assert!(wait_until(window, |window| {
        window.read(|app| {
            view.read(app).active_session().is_some_and(|session| {
                session
                    .file_tab(workspace_id)
                    .and_then(|tab| tab.file())
                    .is_some_and(|file| file.path() == std::path::Path::new("README.md"))
                    && session.workspace(workspace_id).unwrap().tabs().len() == 2
            })
        })
    }));
    assert!(
        wait_until(window, |window| {
            window.update(|window, cx| _ = window.draw(cx));
            window.update(|_, cx| {
                view.read(cx).file_editors.values().any(|editor| {
                    let text = editor.state.read(cx).value();
                    text.contains("# Files") && !text.contains("fn main()")
                })
            })
        }),
        "the retargeted Preview Tab should show the second file"
    );

    // Folding the directory hides its rows again, and switching back to Changes keeps the
    // Files state for the next visit.
    let src = window.debug_bounds("file-dir-src").unwrap();
    window.simulate_click(src.center(), Modifiers::default());
    window.run_until_parked();
    window.update(|window, cx| _ = window.draw(cx));
    assert!(window.debug_bounds("file-src/main.rs").is_none());
    let changes_tab = window.debug_bounds("sidebar-view-changes").unwrap();
    window.simulate_click(changes_tab.center(), Modifiers::default());
    window.run_until_parked();
    window.update(|window, cx| _ = window.draw(cx));
    assert!(window.debug_bounds("condr-files-list").is_none());
    assert!(window.debug_bounds("file-README.md").is_none());

    let _ = std::fs::remove_dir_all(&root);
}
