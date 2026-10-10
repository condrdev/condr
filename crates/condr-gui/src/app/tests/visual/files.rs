use super::*;

/// A Server answer for `README.md` of `workspace_id`, fed the way the connection task does.
fn readme_answer(this: &mut Condr, workspace_id: WorkspaceId, text: &str) -> Incoming {
    let connection = this.connection_mut(1).unwrap();
    let request_id = connection.next_layout_request_id;
    connection.next_layout_request_id += 1;
    connection
        .resources
        .pending_files
        .insert((workspace_id, "README.md".into()), request_id);
    Incoming::Message(condr_core::protocol::ServerMessage::FileContent {
        request_id,
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
                .is_some_and(|session| !session.workspaces().is_empty())
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
                view.read(app)
                    .presented()
                    .is_some_and(|(_, session, _, tab_id)| {
                        session.tab(tab_id).unwrap().file().is_some_and(|file| {
                            file.path() == relative_path::RelativePath::new("src/main.rs")
                        })
                    })
            })
        }),
        "the Preview Tab should become the shown Tab"
    );
    assert!(
        wait_until(window, |window| {
            window.update(|window, cx| _ = window.draw(cx));
            window.debug_bounds("file-body").is_some()
                && window.update(|_, cx| {
                    view.read(cx)
                        .files_view
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
            .workspaces()
            .first()
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
                view.read(cx)
                    .files_view
                    .file_editors
                    .values()
                    .any(|editor| {
                        let text = editor.state.read(cx).value();
                        text.contains("# Files") && !text.contains("fn main()")
                    })
            })
        }),
        "the retargeted Preview Tab should show the second file"
    );
    // Markdown opens rendered (ADR 0037); the header switches it to its text and back.
    window.update(|window, cx| _ = window.draw(cx));
    assert!(window.debug_bounds("file-markdown").is_some());
    assert!(window.debug_bounds("file-body").is_none());
    let source = window.debug_bounds("file-view-source").unwrap();
    window.simulate_click(source.center(), Modifiers::default());
    window.run_until_parked();
    window.update(|window, cx| _ = window.draw(cx));
    assert!(window.debug_bounds("file-markdown").is_none());
    assert!(window.debug_bounds("file-body").is_some());

    // The Server's watcher re-answers every cached file after a working-tree batch. An
    // answer with the same content must not count as a change: the Editor would drop its
    // highlighter and scroll back to the top on every refetch. Different content does land.
    window.update(|window, cx| {
        view.update(cx, |this, cx| {
            let connection = this.connection(1).unwrap();
            let generation = connection.resources.files_generation;
            let connect_generation = connection.connect_generation;

            // Applied the way the connection task applies an answer: a rebuild.
            let answer = readme_answer(this, workspace_id, "# Files\n");
            this.handle_incoming(1, connect_generation, answer, cx);
            this.rebuild_dock(window, cx);
            assert_eq!(
                this.connection(1).unwrap().resources.files_generation,
                generation,
                "identical content is not a change"
            );
            let answer = readme_answer(this, workspace_id, "# Files\n\nMore.\n");
            this.handle_incoming(1, connect_generation, answer, cx);
            this.rebuild_dock(window, cx);
            assert_eq!(
                this.connection(1).unwrap().resources.files_generation,
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
                    .files_view
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
                .resources
                .files
                .keys()
                .filter(|(cached_workspace, _)| *cached_workspace == workspace_id)
                .map(|(_, path)| path.to_string())
                .collect();
            cached.sort();
            cached
        })
    };
    // Not asserted beforehand: on Windows the indexer or antivirus may touch NEW.txt's
    // attributes seconds after the write, and that stray batch drops src/main.rs early.
    std::fs::write(root.join("TOUCH.txt"), "touch\n").unwrap();
    assert!(
        wait_until(window, |window| {
            window.update(|window, cx| _ = window.draw(cx));
            cached_files(window) == ["README.md"]
                && window.update(|_, cx| {
                    view.read(cx)
                        .files_view
                        .file_editors
                        .values()
                        .any(|editor| {
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
                let answer = readme_answer(this, workspace_id, &text);
                this.handle_incoming(1, connect_generation, answer, cx);
                this.rebuild_dock(window, cx);
            });
            _ = window.draw(cx);
        });
    };
    feed(window, long_readme(300));
    let editor = window.read(|app| {
        view.read(app)
            .files_view
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
    let rendered = window.debug_bounds("file-view-rendered").unwrap();
    window.simulate_click(rendered.center(), Modifiers::default());
    window.run_until_parked();
    window.update(|window, cx| _ = window.draw(cx));
    assert!(
        window.debug_bounds("file-markdown").is_some(),
        "an edit while the text shows keeps the choice, and Preview renders again"
    );
    // So does the rendered view: an edit that adds a block keeps where the reader was,
    // though Kit resets its list whenever the block count changes.
    let paragraphs = |count: usize| {
        (0..count)
            .map(|n| format!("Paragraph {n}.\n\n"))
            .collect::<String>()
    };
    feed(window, paragraphs(200));
    let markdown = window.read(|app| {
        view.read(app)
            .files_view
            .file_editors
            .values()
            .next()
            .unwrap()
            .markdown
            .clone()
            .expect("README renders")
    });
    let list = window.read(|app| markdown.read(app).list_state().clone());
    assert!(wait_until(window, |window| {
        window.update(|window, cx| _ = window.draw(cx));
        list.item_count() == 200
    }));
    list.scroll_to(gpui_kit::ListOffset {
        item_ix: 120,
        offset_in_item: px(0.),
    });
    window.update(|window, cx| _ = window.draw(cx));
    feed(window, paragraphs(201));
    assert!(wait_until(window, |window| {
        window.update(|window, cx| _ = window.draw(cx));
        list.item_count() == 201
    }));
    assert_eq!(
        list.logical_scroll_top().item_ix,
        120,
        "an edit to the rendered file must keep the viewport"
    );
    // The wheel over the gutter beside the centred text scrolls it too.
    let before = list.scroll_px_offset_for_scrollbar().y;
    let rendered = window.debug_bounds("file-markdown").unwrap();
    window.simulate_event(gpui_kit::ScrollWheelEvent {
        position: gpui_kit::point(rendered.origin.x + px(4.), rendered.center().y),
        delta: gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-100.))),
        ..Default::default()
    });
    window.update(|window, cx| _ = window.draw(cx));
    assert!(
        list.scroll_px_offset_for_scrollbar().y < before,
        "the wheel over the gutter should scroll the rendered file"
    );

    // A reconnect while a listing is in flight: the old connection's answer never comes
    // and the Bootstrap empties the caches, so the request must be forgotten with it, or
    // the root would say "Loading…" forever. The same-authority Bootstrap is fed directly:
    // a real reconnect ends in exactly this message, and nothing else may run in between.
    window.update(|_, cx| {
        view.update(cx, |this, cx| {
            this.connection_mut(1)
                .unwrap()
                .resources
                .pending_directories
                .insert((workspace_id, relative_path::RelativePathBuf::new()), 42);
            let connection = this.connection(1).unwrap();
            let bootstrap = SessionBootstrap {
                server_id: connection.model.server_id.unwrap(),
                runtime_epoch: connection.model.runtime_epoch.unwrap(),
                session_id: connection.model.session_id.unwrap(),
                sequence: connection.model.sequence,
                snapshot: connection.session().unwrap().snapshot(),
                settings: connection.model.settings.clone(),
                terminals: Vec::new(),
                agents: Vec::new(),
                workspace_git: Vec::new(),
                zoomed_panes: Vec::new(),
            };
            let generation = connection.connect_generation;
            this.handle_incoming(1, generation, Incoming::Bootstrap(bootstrap), cx);
            assert!(
                this.connection(1)
                    .unwrap()
                    .resources
                    .pending_directories
                    .is_empty()
                    && this
                        .connection(1)
                        .unwrap()
                        .resources
                        .pending_files
                        .is_empty(),
                "a request from before the Bootstrap must be forgotten with it"
            );
            assert!(
                this.connection(1).unwrap().resources.directories.is_empty(),
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

/// The Preview Tab renders what it can (ADR 0037): an image file scaled to fit with its
/// size in the header, an SVG that switches to its text, and Markdown that fetches the
/// images it draws from the Server, keeps them through a working-tree refresh without
/// decoding them again, and lets them go once the Tab shows another file.
#[test]
fn preview_tab_renders_images_and_the_images_markdown_draws() {
    let _serial_guard = acquire_visual_test_lock();
    let root = std::env::temp_dir().join(format!(
        "condr-gui-preview-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("docs")).unwrap();
    image::RgbaImage::new(3, 2)
        .save(root.join("shot.png"))
        .unwrap();
    let mut gif = Vec::new();
    {
        let mut encoder = image::codecs::gif::GifEncoder::new(&mut gif);
        for _ in 0..2 {
            encoder
                .encode_frame(image::Frame::from_parts(
                    image::RgbaImage::new(2, 1),
                    0,
                    0,
                    image::Delay::from_numer_denom_ms(50, 1),
                ))
                .unwrap();
        }
    }
    std::fs::write(root.join("anim.gif"), gif).unwrap();
    std::fs::write(
        root.join("logo.svg"),
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="5"><rect width="4" height="5"/></svg>"#,
    )
    .unwrap();
    std::fs::write(
        root.join("docs/guide.md"),
        "# Guide\n\n![shot](../shot.png)\n\n![badge](https://example.com/badge.svg)\n\n\
         ```rust\nfn main() {}\n```\n",
    )
    .unwrap();

    let mut cx = TestAppContext::single();
    cx.update(gpui_kit::init);
    let (view, window, _server) = connected_condr(&mut cx);
    window.update(|window, cx| _ = window.draw(cx));
    let button = window.debug_bounds("open-project").unwrap();
    window.simulate_click(button.center(), Modifiers::default());
    let selected_root = root.clone();
    window.simulate_path_prompt_response(move |_| Some(vec![selected_root]));
    // The Server answers the new Workspace, which this Client then shows, as it frees the
    // connection for the next layout command; a ShowFile sent earlier would be refused.
    assert!(wait_until(window, |window| {
        window.read(|app| {
            view.read(app)
                .connection(1)
                .unwrap()
                .view_workspace
                .is_some()
        })
    }));
    let workspace_id =
        window.read(|app| view.read(app).active_session().unwrap().workspaces()[0].id());
    let show = |window: &mut VisualTestContext, path: &str| {
        window.update(|window, cx| {
            view.update(cx, |this, cx| {
                this.show_file_on(1, workspace_id, path.into(), window, cx);
            });
        });
        assert!(
            wait_until(window, |window| window.read(|app| {
                view.read(app)
                    .active_session()
                    .and_then(|session| session.file_tab(workspace_id)?.file())
                    .is_some_and(|file| file.path() == relative_path::RelativePath::new(path))
            })),
            "the Preview Tab should show {path}"
        );
    };
    let image = |window: &mut VisualTestContext, path: &str| {
        window.read(|app| {
            view.read(app)
                .files_view
                .images
                .get(&(1, workspace_id, path.into()))
        })
    };
    let drawn = |window: &mut VisualTestContext, selector: &'static str| {
        window.update(|window, cx| _ = window.draw(cx));
        window.debug_bounds(selector).is_some()
    };

    show(window, "shot.png");
    assert!(
        wait_until(window, |window| {
            drawn(window, "file-image") && drawn(window, "file-image-summary")
        }),
        "an image file renders, its size in the header"
    );
    assert_eq!(image(window, "shot.png").unwrap().unwrap().size, size(3, 2));

    show(window, "anim.gif");
    assert!(wait_until(window, |window| drawn(window, "file-image")
        && image(window, "anim.gif").is_some_and(|image| image.is_ok())));
    assert!(
        image(window, "shot.png").is_none(),
        "the image shown before goes with it"
    );

    show(window, "logo.svg");
    assert!(
        wait_until(window, |window| drawn(window, "file-image")
            && image(window, "logo.svg").is_some_and(|image| image.is_ok())),
        "an SVG renders"
    );
    assert_eq!(
        image(window, "logo.svg").unwrap().unwrap().size,
        size(4, 5),
        "in its own units, not GPUI's doubled raster"
    );
    let source = window.debug_bounds("file-view-source").unwrap();
    window.simulate_click(source.center(), Modifiers::default());
    window.run_until_parked();
    assert!(drawn(window, "file-body") && !drawn(window, "file-image"));

    // Markdown asks the Server for the images it draws, relative to its own directory;
    // a badge on the web is left as its alt text and never asked for.
    show(window, "docs/guide.md");
    assert!(
        wait_until(window, |window| drawn(window, "file-markdown")
            && image(window, "shot.png").is_some_and(|image| image.is_ok())),
        "a Markdown image is fetched and decoded"
    );
    assert!(
        image(window, "logo.svg").is_none(),
        "a new file starts rendered, and the last one's images go"
    );
    assert!(window.read(|app| {
        let resources = &view.read(app).connection(1).unwrap().resources;
        resources
            .files
            .keys()
            .chain(resources.pending_files.keys())
            .all(|(_, path)| !path.as_str().contains("badge"))
    }));
    let decoded = image(window, "shot.png").unwrap().unwrap();
    let cached_generation = |window: &mut VisualTestContext| {
        window.read(|app| {
            view.read(app)
                .connection(1)
                .unwrap()
                .resources
                .files
                .get(&(workspace_id, "shot.png".into()))
                .map(|(generation, _)| *generation)
        })
    };
    let fetched = cached_generation(window).unwrap();

    // A working-tree batch drops the cached bytes; drawing asks again, and the same
    // picture keeps its decode on screen rather than flashing or decoding again.
    std::fs::write(root.join("TOUCH.txt"), "touch\n").unwrap();
    assert!(
        wait_until(window, |window| {
            window.update(|window, cx| _ = window.draw(cx));
            cached_generation(window).is_some_and(|generation| generation != fetched)
        }),
        "the drawn image is asked for again after the refresh"
    );
    window.run_until_parked();
    assert!(std::sync::Arc::ptr_eq(
        &decoded,
        &image(window, "shot.png").unwrap().unwrap()
    ));

    let _ = std::fs::remove_dir_all(&root);
}
