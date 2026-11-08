//! The screenshots on condr.dev, written into `assets/screenshots/`:
//!
//! ```sh
//! CONDR_GUI_WEBSITE_SCREENSHOTS=assets/screenshots cargo run -p condr-gui --features test-support
//! ```
//!
//! What they show is real wherever it can be. A worktree of this repository holds
//! [`DEMO`]'s change uncommitted, for Changes, the Diff and Preview Tabs and History, and
//! Claude Code, Codex and lazygit run in its Panes, so all three must be installed and the
//! first two must trust this repository. Two more Servers on loopback stand in for remote
//! Devices; the Agents listed on them and the transfers in flight are staged. Each picture
//! is a crop of the 1280 by 800 window, at the display's scale. The hero is the whole window
//! grown to 1600 by 1000, with the buttons macOS would draw painted in; the README's picture
//! puts the Companion's `phone-agents.png` in front of it.

use super::super::changes::SidebarView;
use super::super::history::DiffCommit;
use super::super::ports::PortsScope;
use super::super::transfers::TransferState;
use super::*;
use condr_core::protocol::{
    LayoutCommand, LayoutResult, RelativePathBuf, ServerMessage, SessionEvent,
};
use condr_core::{AgentKind, AgentSnapshot, AgentState, PaneId, SplitDirection};
use condr_core::{TabId, TerminalCommand, WorkspaceId};
use std::path::Path;
use std::process::Command;

/// The left sidebar's width, wide enough for an Agent's task and what it waits for.
const SIDEBAR: f32 = 300.;
/// The hero's window, in points.
const HERO: (f32, f32) = (1600., 1000.);

/// The commit whose change the worktree holds uncommitted: a small one to the Ports page.
const DEMO: &str = "bd89134";
const DEMO_FILE: &str = "crates/condr-gui/src/app/ports.rs";
/// The worktree's branch, made and deleted by each run.
const BRANCH: &str = "forward-port-form";

pub(crate) fn run(directory: PathBuf) {
    std::fs::create_dir_all(&directory).expect("the screenshot directory can be made");
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    // Under the repository, so Claude Code and Codex trust the worktree as they trust it.
    let scratch = repository.join("target/website-screenshots");
    let worktree = scratch.join("condr");
    if worktree.exists() {
        // A run that stopped half way left its worktree and branch behind.
        git(
            &repository,
            &["worktree", "remove", "--force", text(&worktree)],
        );
        git(&repository, &["branch", "-D", BRANCH]);
    }
    let _ = std::fs::remove_dir_all(&scratch);
    assert!(
        git(
            &repository,
            &[
                "worktree",
                "add",
                "--quiet",
                "-b",
                BRANCH,
                text(&worktree),
                DEMO
            ],
        ),
        "the worktree on a new branch {BRANCH} could not be made"
    );
    assert!(git(&worktree, &["reset", "--quiet", &format!("{DEMO}~1")]));

    let platform = gpui_kit::platform::current_platform(false);
    let mut cx = VisualTestAppContext::with_asset_source(platform, Arc::new(CondrAssets::new()));
    cx.update(gpui_kit::init);

    let local = Running::start();
    let build_box = Running::start();
    let gpu_box = Running::start();

    // This machine: the worktree, with three programs side by side in its first Tab.
    let mut admin = local.admin();
    let (condr, claude, condr_tab) = workspace(&mut admin, &worktree);
    let codex = split(&mut admin, claude, SplitDirection::Horizontal);
    let lazygit = split(&mut admin, codex, SplitDirection::Vertical);
    // Run from inside Claude Code, the Panes inherit its session, which Claude Code there
    // would warn about.
    let unset = "for v in $(env | grep -o '^CLAUDE[A-Z_]*'); do unset $v; done";
    for (pane, program) in [
        (claude, format!("{unset}; claude")),
        (codex, "codex -c check_for_update_on_startup=false".into()),
        (lazygit, "lazygit".into()),
    ] {
        type_line(&mut admin, pane, &program);
    }
    workspace(
        &mut admin,
        &project(&scratch, "dotfiles", &[".zshrc", "install.sh"]),
    );

    // The remote Devices, each with a project and two staged Agents.
    let mut remote = build_box.admin();
    let dashboard_files = [
        "package.json",
        "pnpm-lock.yaml",
        "tsconfig.json",
        "vite.config.ts",
        "public/favicon.svg",
        "src/App.tsx",
        "src/main.tsx",
        "src/pages/Orders.tsx",
        "src/pages/Statistics.tsx",
        "tests/orders.test.ts",
    ];
    let (dashboard, statistics, _) = workspace(
        &mut remote,
        &project(&scratch, "admin-dashboard", &dashboard_files),
    );
    let export = split(&mut remote, statistics, SplitDirection::Horizontal);
    title(&mut remote, statistics, "Implement statistics page");
    title(&mut remote, export, "Add CSV export");
    let mut gpu = gpu_box.admin();
    let (_, sweep, _) = workspace(
        &mut gpu,
        &project(&scratch, "ml-pipeline", &["pyproject.toml", "train.py"]),
    );
    let loader = split(&mut gpu, sweep, SplitDirection::Horizontal);
    title(&mut gpu, sweep, "Sweep batch sizes");
    title(&mut gpu, loader, "Profile the data loader");
    let staged = [
        (
            2,
            statistics,
            AgentKind::Claude,
            AgentState::Blocked,
            Some("Bash: pnpm test --filter dashboard"),
        ),
        (2, export, AgentKind::OpenCode, AgentState::Working, None),
        (3, sweep, AgentKind::Codex, AgentState::Done, None),
        (
            3,
            loader,
            AgentKind::Claude,
            AgentState::Error,
            Some("API Error: 529 Overloaded"),
        ),
    ];

    let devices = [&local, &build_box, &gpu_box];
    let (window, view) = open(&mut cx, (1280., 800.), devices);
    update(&mut cx, window, &view, |this, window, cx| {
        this.select_workspace(1, condr, window, cx)
    });
    wait_for(&mut admin, claude, "Claude Code");
    wait_for(&mut admin, codex, "OpenAI Codex");
    wait_for(&mut admin, lazygit, "Status");

    stage(&mut cx, window, &view, &staged);
    let image = capture(&mut cx, window);
    // Down to the end of build-box: Needs you and the Devices it names, not the whole column.
    save(&image, &directory, "needs-you", (0., 0., SIDEBAR, 578.));
    save(
        &image,
        &directory,
        "panes",
        (SIDEBAR, 0., 1280. - SIDEBAR, 800.),
    );

    // Review, with the left sidebar folded for the code's room: the Diff, Preview and
    // History of the worktree's change.
    let path = RelativePathBuf::from(DEMO_FILE);
    update(&mut cx, window, &view, |this, window, cx| {
        this.toggle_sidebar(window, cx);
        this.toggle_changes_sidebar(cx);
    });
    let review = (48., 0., 1280. - 48., 800.);
    {
        let path = path.clone();
        update(&mut cx, window, &view, move |this, window, cx| {
            this.show_diff_on(1, condr, path, window, cx)
        });
    }
    wait(&mut cx, "the diff loaded", |app| {
        view.read(app).connection(1).is_some_and(|connection| {
            connection
                .resources
                .diffs
                .get(&(condr, path.clone()))
                .is_some()
        })
    });
    let image = capture(&mut cx, window);
    save(&image, &directory, "diff", review);

    {
        let path = path.clone();
        update(&mut cx, window, &view, move |this, window, cx| {
            this.files_view
                .sidebar_views
                .insert((1, condr), SidebarView::Files);
            for directory in ["crates", "crates/condr-gui", "crates/condr-gui/src"] {
                this.files_view
                    .expanded_dirs
                    .insert((1, condr, RelativePathBuf::from(directory)));
            }
            // The Tab scrolls this line to its bottom, under the form's rewritten rows.
            this.files_view.pending_file_line = Some((1, condr, path.clone(), 532));
            this.show_file_on(1, condr, path, window, cx)
        });
    }
    let image = capture(&mut cx, window);
    save(&image, &directory, "preview", review);

    update(&mut cx, window, &view, |this, _, cx| {
        this.files_view
            .sidebar_views
            .insert((1, condr), SidebarView::History);
        cx.notify();
    });
    wait(&mut cx, "History loaded", |app| {
        view.read(app)
            .connection(1)
            .is_some_and(|connection| connection.resources.logs.contains_key(&condr))
    });
    // The commit before the change, selected, and one of its files in the Diff Tab.
    update(&mut cx, window, &view, |this, window, cx| {
        let commit = this.connection(1).unwrap().resources.logs[&condr].commits[0].clone();
        let path = RelativePathBuf::from("crates/condr-gui/src/app/sidebar.rs");
        this.files_view
            .selected_commits
            .insert((1, condr), commit.id.clone());
        this.show_diff_on(1, condr, path.clone(), window, cx);
        this.files_view
            .diff_commits
            .insert((1, condr), DiffCommit { path, commit });
        this.rebuild_dock(window, cx);
        cx.notify();
    });
    let image = capture(&mut cx, window);
    save(&image, &directory, "history", review);

    update(&mut cx, window, &view, |this, window, cx| {
        this.toggle_sidebar(window, cx)
    });

    // Ports forwarded from both remote Devices.
    for (key, port, forwards) in [(2, "5173", 1), (2, "8080", 2), (3, "6006", 1)] {
        update(&mut cx, window, &view, move |this, window, cx| {
            this.open_ports(PortsScope::Device(key), window, cx);
        });
        update(&mut cx, window, &view, move |this, window, cx| {
            this.fill_ports_form(port, "", window, cx);
        });
        update(&mut cx, window, &view, move |this, window, cx| {
            this.submit_forward(key, window, cx)
        });
        wait(&mut cx, "the forward started", |app| {
            view.read(app).forward_local_ports(key).len() == forwards
        });
    }
    update(&mut cx, window, &view, |this, window, cx| {
        this.open_ports(PortsScope::All, window, cx)
    });
    let image = capture(&mut cx, window);
    save(
        &image,
        &directory,
        "ports",
        (SIDEBAR, 0., 1280. - SIDEBAR, 680.),
    );

    // Files moving to and from build-box.
    let uploads = scratch.join("uploads");
    std::fs::create_dir_all(&uploads).unwrap();
    std::fs::write(uploads.join("mockup.png"), vec![7; 2_400_000]).unwrap();
    std::fs::write(uploads.join("seed.sql"), vec![b'-'; 180_000]).unwrap();
    update(&mut cx, window, &view, |this, window, cx| {
        this.close_ports(window, cx);
        this.select_workspace(2, dashboard, window, cx);
    });
    update(&mut cx, window, &view, move |this, window, cx| {
        this.toggle_changes_sidebar(cx);
        this.files_view
            .sidebar_views
            .insert((2, dashboard), SidebarView::Files);
        for directory in ["public", "src", "src/pages", "tests"] {
            this.files_view
                .expanded_dirs
                .insert((2, dashboard, RelativePathBuf::from(directory)));
        }
        this.upload_files(
            2,
            dashboard,
            RelativePathBuf::new(),
            vec![uploads.join("mockup.png"), uploads.join("seed.sql")],
            window,
            cx,
        );
    });
    wait(&mut cx, "the uploads finished", |app| {
        let states = view.read(app).transfer_states(2);
        states.len() == 2 && states.iter().all(|state| *state == TransferState::Done)
    });
    update(&mut cx, window, &view, move |this, _, cx| {
        this.show_transfer(
            2,
            dashboard,
            "dist/assets/index.js",
            false,
            (31 << 20, Some(48 << 20)),
            TransferState::Running,
        );
        this.show_transfer(
            2,
            dashboard,
            "fixtures/orders.csv",
            true,
            (0, Some(212 << 20)),
            TransferState::Queued,
        );
        this.transfer_strip_open = true;
        cx.notify();
    });
    wait(&mut cx, "Files lists the uploads", |app| {
        view.read(app)
            .connection(2)
            .and_then(|connection| {
                let root = (dashboard, RelativePathBuf::new());
                connection.resources.directories.get(&root)?.as_ref().ok()
            })
            .is_some_and(|listing| {
                listing
                    .entries
                    .iter()
                    .any(|entry| entry.name == "mockup.png")
            })
    });
    let image = capture(&mut cx, window);
    save(
        &image,
        &directory,
        "transfers",
        (1280. - 320., 36., 320., 764.),
    );

    // The hero: the whole window, larger so the Panes between the sidebars have room. It is a
    // window of its own, opened once this one is gone: a Server sizes a terminal for one client.
    let _ = cx.update_window(window, |_, window, _| window.remove_window());
    let (window, view) = open(&mut cx, HERO, devices);
    update(&mut cx, window, &view, |this, window, cx| {
        this.select_workspace(1, condr, window, cx);
        this.show_view(1, condr, Some(condr_tab));
        this.toggle_changes_sidebar(cx);
        this.files_view
            .sidebar_views
            .insert((1, condr), SidebarView::Files);
        this.rebuild_dock(window, cx);
    });
    settle(&mut cx);
    stage(&mut cx, window, &view, &staged);
    let mut hero = capture(&mut cx, window);
    let scale = hero.width() as f32 / HERO.0;
    traffic_lights(&mut hero, scale);
    save_whole(&hero, &directory, "hero");
    let phone = image::open(repository.join("assets/screenshots/phone-agents.png"))
        .expect("the Companion's Agents page is in assets/screenshots")
        .to_rgba8();
    save_whole(&with_phone(&hero, &phone), &directory, "hero-with-phone");

    let _ = cx.update_window(window, |_, window, _| window.remove_window());
    for server in [local, build_box, gpu_box] {
        server.stop();
    }
    git(
        &repository,
        &["worktree", "remove", "--force", text(&worktree)],
    );
    git(&repository, &["branch", "-D", BRANCH]);
    let _ = std::fs::remove_dir_all(&scratch);
    eprintln!("screenshots saved in {}", directory.display());
}

struct Running {
    endpoint: Endpoint,
    handle: condr_server::ServerHandle,
    thread: std::thread::JoinHandle<std::io::Result<()>>,
}

impl Running {
    fn start() -> Self {
        let server =
            BoundServer::bind(ServerConfig::ephemeral_tcp("127.0.0.1:0".parse().unwrap()).unwrap())
                .unwrap();
        let endpoint = server.endpoint().clone();
        let handle = server.handle();
        let thread = std::thread::spawn(move || server.run());
        Self {
            endpoint,
            handle,
            thread,
        }
    }

    fn admin(&self) -> ClientConnection {
        ClientConnection::connect_overview(&self.endpoint, "screenshots").unwrap()
    }

    fn stop(self) {
        self.handle.stop();
        let _ = self.thread.join();
    }
}

/// A Workspace on `root`, with its first Tab and Pane.
fn workspace(admin: &mut ClientConnection, root: &Path) -> (WorkspaceId, PaneId, TabId) {
    let LayoutResult::WorkspaceCreated {
        workspace_id,
        tab_id,
        pane_id,
    } = admin
        .layout(LayoutCommand::CreateWorkspace {
            root_directory: root.to_path_buf(),
            name: None,
        })
        .unwrap()
        .unwrap()
    else {
        panic!("expected a Workspace");
    };
    (workspace_id, pane_id, tab_id)
}

fn split(admin: &mut ClientConnection, pane_id: PaneId, direction: SplitDirection) -> PaneId {
    let LayoutResult::PaneCreated { pane_id } = admin
        .layout(LayoutCommand::SplitPane {
            pane_id,
            direction,
            focus: false,
        })
        .unwrap()
        .unwrap()
    else {
        panic!("expected a Pane");
    };
    pane_id
}

/// A Git repository with `files`, empty, standing in for a project.
fn project(scratch: &Path, name: &str, files: &[&str]) -> PathBuf {
    let root = scratch.join("projects").join(name);
    for file in files.iter().chain(&["README.md"]) {
        let path = root.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "").unwrap();
    }
    assert!(git(&root, &["init", "--quiet", "--initial-branch", "main"]));
    root
}

/// Types `line` into a Pane's shell once its prompt shows, and runs it.
fn type_line(admin: &mut ClientConnection, pane_id: PaneId, line: &str) {
    wait_for(admin, pane_id, "%");
    admin
        .terminal(pane_id, TerminalCommand::Text(format!("{line}\r")))
        .unwrap();
}

/// Names a Pane the way an Agent names its task, and keeps the name, on a clear screen,
/// while it waits.
fn title(admin: &mut ClientConnection, pane_id: PaneId, name: &str) {
    type_line(
        admin,
        pane_id,
        &format!("printf '\\033]2;{name}\\007\\033[H\\033[2J'; cat"),
    );
}

fn wait_for(admin: &mut ClientConnection, pane_id: PaneId, needle: &str) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let text = admin.read_pane(pane_id, 200).unwrap();
        if text.contains(needle) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {needle:?}, the Pane shows:\n{text}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// The staged Agents' states, as their Servers would report them. Put back before each
/// picture, since a resynchronization since the last one drops them.
fn stage(
    cx: &mut VisualTestAppContext,
    window: AnyWindowHandle,
    view: &Entity<Condr>,
    staged: &[(ConnectionKey, PaneId, AgentKind, AgentState, Option<&str>)],
) {
    for &(key, pane_id, kind, state, detail) in staged {
        update(cx, window, view, move |this, _, cx| {
            let connection = this.connection(key).unwrap();
            let generation = connection.connect_generation;
            let message = ServerMessage::Event {
                server_id: connection.model.server_id.unwrap(),
                session_id: connection.model.session_id.unwrap(),
                sequence: connection.model.sequence + 1,
                event: SessionEvent::AgentChanged {
                    pane_id,
                    agent: Some(AgentSnapshot {
                        session_id: None,
                        kind,
                        state,
                        detail: detail.map(str::to_owned),
                    }),
                },
            };
            this.handle_incoming(key, generation, Incoming::Message(message), cx);
            cx.notify();
        });
    }
}

/// An offscreen window of `(width, height)` points showing Condr connected to this machine's
/// Server and to the two standing in for remote Devices, once all three are synchronized.
fn open(
    cx: &mut VisualTestAppContext,
    (width, height): (f32, f32),
    [local, build_box, gpu_box]: [&Running; 3],
) -> (AnyWindowHandle, Entity<Condr>) {
    let mut config = config::LoadedConfig::read(None);
    config.appearance = Appearance::Dark;
    let initial = ClientConnection::connect(&local.endpoint, "condr-screenshots");
    let holder: Rc<RefCell<Option<Entity<Condr>>>> = Rc::default();
    let window = {
        let holder = Rc::clone(&holder);
        let endpoint = local.endpoint.clone();
        cx.open_offscreen_window(size(px(width), px(height)), move |window, cx| {
            let view = cx.new(|cx| {
                Condr::new(
                    endpoint,
                    Some(initial.map_err(|error| error.to_string())),
                    config,
                    gui_state::LoadedState {
                        state: gui_state::GuiState {
                            sidebar_width: Some(SIDEBAR),
                            ..Default::default()
                        },
                        ..Default::default()
                    },
                    window,
                    cx,
                )
            });
            holder.borrow_mut().replace(view.clone());
            cx.new(|cx| Root::new(view, window, cx))
        })
        .expect("an offscreen window opens")
    };
    let window: AnyWindowHandle = window.into();
    let view = holder.borrow_mut().take().expect("Condr was built");
    update(cx, window, &view, |this, _, _| {
        for (name, server) in [("build-box", build_box), ("gpu-box", gpu_box)] {
            let key = this.next_connection_key;
            this.next_connection_key += 1;
            this.connections.push(ServerConnection::new(
                key,
                name.into(),
                server.endpoint.clone(),
            ));
            this.start_connect(key);
        }
    });
    wait(cx, "every Device synchronized", |app| {
        (1..=3).all(|key| {
            view.read(app)
                .connection(key)
                .and_then(ServerConnection::session)
                .is_some_and(|session| !session.workspaces().is_empty())
        })
    });
    (window, view)
}

/// Gives the programs in the Panes time to redraw at the size the window gave them.
fn settle(cx: &mut VisualTestAppContext) {
    let until = Instant::now() + Duration::from_millis(1500);
    while Instant::now() < until {
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// macOS draws a window's close, minimize and zoom buttons itself, so an offscreen window has
/// none. Paints them where Kit's title bar leaves room for them: 12 points across, 20 apart,
/// centred on its 34-point height. `scale` is pixels per point.
fn traffic_lights(image: &mut RgbaImage, scale: f32) {
    let radius = 6. * scale;
    for (i, color) in [[255., 95., 87.], [254., 188., 46.], [40., 200., 64.]]
        .into_iter()
        .enumerate()
    {
        let (cx, cy) = ((15. + 20. * i as f32) * scale, 17. * scale);
        for y in (cy - radius - 1.) as u32..=(cy + radius + 1.) as u32 {
            for x in (cx - radius - 1.) as u32..=(cx + radius + 1.) as u32 {
                let distance = (x as f32 + 0.5 - cx).hypot(y as f32 + 0.5 - cy);
                // Coverage of the pixel by the disc, for an antialiased edge.
                let cover = (radius + 0.5 - distance).clamp(0., 1.);
                let pixel = image.get_pixel_mut(x, y);
                for c in 0..3 {
                    pixel[c] =
                        (f32::from(pixel[c]) * (1. - cover) + color[c] * cover).round() as u8;
                }
            }
        }
    }
}

/// The README's picture: the hero window, 1820 pixels across, with the Companion on a phone
/// standing in front of its lower right corner, on transparency.
fn with_phone(hero: &RgbaImage, phone: &RgbaImage) -> RgbaImage {
    use image::imageops::{FilterType, resize};
    let (width, height) = (1820, 1820 * hero.height() / hero.width());
    let window = resize(hero, width, height, FilterType::Lanczos3);
    // The phone three quarters of the window's height; its screen keeps the screenshot's shape.
    let bezel = 18.;
    let screen_height = (0.74 * height as f32 - 2. * bezel).round();
    let screen_width = (screen_height * phone.width() as f32 / phone.height() as f32).round();
    let screen = resize(
        phone,
        screen_width as u32,
        screen_height as u32,
        FilterType::Lanczos3,
    );
    let mut out = RgbaImage::new(width + 100, height + 120);
    let body = (
        out.width() as f32 - screen_width - 2. * bezel,
        out.height() as f32 - screen_height - 2. * bezel,
        screen_width + 2. * bezel,
        screen_height + 2. * bezel,
    );
    let glass = (body.0 + bezel, body.1 + bezel, screen_width, screen_height);
    let frame = (0., 0., width as f32, height as f32);
    for (x, y, pixel) in out.enumerate_pixels_mut() {
        let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
        let mut color = [0.; 4];
        let shown = covered(px, py, frame, 14.);
        if let Some(p) = window.get_pixel_checked(x, y) {
            over(&mut color, p, shown);
        }
        // A hairline round the window, as macOS draws one on a dark window.
        let rim = shown - covered(px, py, inset(frame, 1.), 13.);
        over(&mut color, &image::Rgba([255, 255, 255, 36]), rim);
        let case = covered(px, py, body, 62.);
        over(&mut color, &image::Rgba([11, 11, 12, 255]), case);
        let edge = case - covered(px, py, inset(body, 2.), 60.);
        over(&mut color, &image::Rgba([72, 72, 76, 255]), edge);
        let lit = covered(px, py, glass, 46.);
        if let Some(p) =
            screen.get_pixel_checked((px - glass.0).max(0.) as u32, (py - glass.1).max(0.) as u32)
        {
            over(&mut color, p, lit);
        }
        *pixel = image::Rgba(color.map(|channel| channel.round() as u8));
    }
    out
}

/// How much of the pixel centred at `(px, py)` the rounded rectangle `(x, y, width, height)`
/// with corner `radius` covers, from its signed distance, for an antialiased edge.
fn covered(px: f32, py: f32, (x, y, width, height): (f32, f32, f32, f32), radius: f32) -> f32 {
    let dx = (px - x - width / 2.).abs() - (width / 2. - radius);
    let dy = (py - y - height / 2.).abs() - (height / 2. - radius);
    let distance = dx.max(0.).hypot(dy.max(0.)) + dx.max(dy).min(0.) - radius;
    (0.5 - distance).clamp(0., 1.)
}

fn inset((x, y, width, height): (f32, f32, f32, f32), by: f32) -> (f32, f32, f32, f32) {
    (x + by, y + by, width - 2. * by, height - 2. * by)
}

/// Lays `source`, `cover` of it, over the straight-alpha `color`.
fn over(color: &mut [f32; 4], source: &image::Rgba<u8>, cover: f32) {
    let alpha = cover * f32::from(source[3]) / 255.;
    if alpha <= 0. {
        return;
    }
    let below = color[3] / 255. * (1. - alpha);
    let total = alpha + below;
    for c in 0..3 {
        color[c] = (f32::from(source[c]) * alpha + color[c] * below) / total;
    }
    color[3] = total * 255.;
}

fn save_whole(image: &RgbaImage, directory: &Path, name: &str) {
    let path = directory.join(format!("{name}.png"));
    image.save(&path).expect("the screenshot is written");
    eprintln!("{}", path.display());
}

/// Saves the part of the window at `(x, y, width, height)`, in the window's points.
fn save(
    image: &RgbaImage,
    directory: &Path,
    name: &str,
    (x, y, width, height): (f32, f32, f32, f32),
) {
    let scale = image.width() as f32 / 1280.;
    let pixels = |points: f32| (points * scale).round() as u32;
    let path = directory.join(format!("{name}.png"));
    image::imageops::crop_imm(image, pixels(x), pixels(y), pixels(width), pixels(height))
        .to_image()
        .save(&path)
        .expect("the screenshot is written");
    eprintln!("{}", path.display());
}

/// The `git` this tool runs to set its scene, which Condr itself never does: it reads Git
/// through gix.
fn git(directory: &Path, args: &[&str]) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(directory)
        .args(args)
        .status()
        .is_ok_and(|status| status.success())
}

fn text(path: &Path) -> &str {
    path.to_str().expect("a UTF-8 path")
}
