use super::*;

/// A Server answer for `README.md` of `workspace_id`, fed the way the connection task does.
fn readme_answer(workspace_id: WorkspaceId, text: &str) -> Incoming {
    Incoming::Message(condr_core::protocol::ServerMessage::FileContent {
        request_id: 0,
        workspace_id,
        path: relative_path::RelativePathBuf::from("README.md"),
        result: Ok(condr_core::FileContent::Text {
            text: text.to_owned(),
        }),
    })
}

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
                        workspace.active_tab().file().is_some_and(|file| {
                            file.path() == relative_path::RelativePath::new("src/main.rs")
                        })
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
                    .is_some_and(|file| {
                        file.path() == relative_path::RelativePath::new("README.md")
                    })
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

    // The Server's watcher re-answers every cached file after a working-tree batch. An
    // answer with the same content must not count as a change: the Editor would drop its
    // highlighter and scroll back to the top on every refetch. Different content does land.
    window.update(|window, cx| {
        view.update(cx, |this, cx| {
            let connection = this.connection(1).unwrap();
            let generation = connection.files_generation;
            let connect_generation = connection.connect_generation;
            let answer = |text| readme_answer(workspace_id, text);
            // Applied the way the connection task applies an answer: a rebuild.
            this.handle_incoming(1, connect_generation, answer("# Files\n"), cx);
            this.rebuild_dock(window, cx);
            assert_eq!(
                this.connection(1).unwrap().files_generation,
                generation,
                "identical content is not a change"
            );
            this.handle_incoming(1, connect_generation, answer("# Files\n\nMore.\n"), cx);
            this.rebuild_dock(window, cx);
            assert_eq!(
                this.connection(1).unwrap().files_generation,
                generation + 1,
                "new content is a change"
            );
        });
    });
    assert!(
        wait_until(window, |window| {
            window.update(|window, cx| _ = window.draw(cx));
            window.update(|_, cx| {
                view.read(cx)
                    .file_editors
                    .values()
                    .any(|editor| editor.state.read(cx).value().contains("More."))
            })
        }),
        "changed content should reach the Editor"
    );

    // A working-tree batch refetches only the file the Preview Tab shows; src/main.rs,
    // cached when it was shown earlier, is dropped rather than asked for again.
    let cached_files = |window: &mut VisualTestContext| {
        window.read(|app| {
            let mut cached: Vec<String> = view
                .read(app)
                .connection(1)
                .unwrap()
                .files
                .keys()
                .filter(|(cached_workspace, _)| *cached_workspace == workspace_id)
                .map(|(_, path)| path.to_string())
                .collect();
            cached.sort();
            cached
        })
    };
    assert_eq!(cached_files(window), ["README.md", "src/main.rs"]);
    std::fs::write(root.join("TOUCH.txt"), "touch\n").unwrap();
    assert!(
        wait_until(window, |window| {
            window.update(|window, cx| _ = window.draw(cx));
            cached_files(window) == ["README.md"]
                && window.update(|_, cx| {
                    view.read(cx).file_editors.values().any(|editor| {
                        let text = editor.state.read(cx).value();
                        text.contains("# Files") && !text.contains("More.")
                    })
                })
        }),
        "a working-tree change should refetch the shown file and drop the other cached one"
    );

    // New content for the file the Tab already shows is an edit to the open document:
    // the viewport stays where the user scrolled it instead of jumping to the top.
    let long_readme = |lines: usize| {
        let mut text = String::from("# Files\n");
        for line in 0..lines {
            text.push_str(&format!("line {line}\n"));
        }
        text
    };
    let feed = |window: &mut VisualTestContext, text: String| {
        window.update(|window, cx| {
            view.update(cx, |this, cx| {
                let connect_generation = this.connection(1).unwrap().connect_generation;
                this.handle_incoming(
                    1,
                    connect_generation,
                    readme_answer(workspace_id, &text),
                    cx,
                );
                this.rebuild_dock(window, cx);
            });
            _ = window.draw(cx);
        });
    };
    feed(window, long_readme(300));
    let editor = window.read(|app| {
        view.read(app)
            .file_editors
            .values()
            .next()
            .expect("the Preview Tab has an Editor")
            .state
            .clone()
    });
    window.update(|_, cx| {
        editor.update(cx, |state, cx| {
            state.set_scroll_offset(gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(-200.)), cx);
        });
    });
    window.update(|window, cx| _ = window.draw(cx));
    let scrolled = window.read(|app| editor.read(app).scroll_offset());
    assert!(
        scrolled.y < gpui_kit::px(0.),
        "the fixture should be scrolled down, got {scrolled:?}"
    );
    feed(window, long_readme(301));
    assert!(
        window.read(|app| editor.read(app).value().contains("line 300")),
        "the edit should land"
    );
    assert_eq!(
        window.read(|app| editor.read(app).scroll_offset()),
        scrolled,
        "an edit to the shown file must keep the viewport"
    );

    // A reconnect while a listing is in flight: the old connection's answer never comes
    // and the Bootstrap empties the caches, so the request must be forgotten with it, or
    // the root would say "Loading…" forever. The same-authority Bootstrap is fed directly:
    // a real reconnect ends in exactly this message, and nothing else may run in between.
    window.update(|_, cx| {
        view.update(cx, |this, cx| {
            this.pending_directories.insert((
                1,
                workspace_id,
                relative_path::RelativePathBuf::new(),
            ));
            let connection = this.connection(1).unwrap();
            let bootstrap = SessionBootstrap {
                server_id: connection.server_id.unwrap(),
                runtime_epoch: connection.runtime_epoch.unwrap(),
                session_id: connection.session_id.unwrap(),
                sequence: connection.sequence,
                snapshot: connection.snapshot.clone(),
                settings: connection.settings.clone(),
                terminals: Vec::new(),
                agents: Vec::new(),
                workspace_git: Vec::new(),
                zoomed_panes: Vec::new(),
            };
            let generation = connection.connect_generation;
            this.handle_incoming(1, generation, Incoming::Bootstrap(bootstrap), cx);
            assert!(
                this.pending_directories.is_empty() && this.pending_files.is_empty(),
                "a request from before the Bootstrap must be forgotten with it"
            );
            assert!(
                this.connection(1).unwrap().directories.is_empty(),
                "the Bootstrap replaces every listing"
            );
        });
    });
    assert!(
        wait_until(window, |window| {
            window.update(|window, cx| _ = window.draw(cx));
            window.debug_bounds("file-README.md").is_some()
                && window.debug_bounds("file-src/main.rs").is_some()
        }),
        "the listings should be asked for again after the reconnect"
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
