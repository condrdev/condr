//! Offscreen screenshots of the GUI, for looking at a change without a display (built on
//! macOS with `test-support` only, since GPUI renders offscreen there alone). macOS renders
//! the window through Metal without showing it, which needs the main thread, so this runs
//! from `main` rather than the test harness:
//!
//! ```sh
//! CONDR_GUI_SCREENSHOTS=/tmp/condr-shots cargo run -p condr-gui --features test-support
//! ```
//!
//! It starts a TCP Server with one Workspace, opens Condr against it, and saves one PNG
//! per state: the Workspace, the Command Palette's pages, the Ports page empty, with forwards, for one Device, the right
//! sidebar's transfer strip, the left sidebar with its Device folded, and a finished
//! download's toast. The website's screenshots come from the same window, in `website`.

mod website;

pub(crate) use website::run as website;

use super::ports::PortsScope;
use super::*;
use crate::assets::CondrAssets;
use condr_core::protocol::{LayoutCommand, LayoutResult, RelativePathBuf};
use condr_server::{BoundServer, ServerConfig};
use image::RgbaImage;
use std::cell::RefCell;
use std::io::{Read as _, Write as _};
use std::rc::Rc;
use std::sync::Arc;

const WAIT: Duration = Duration::from_secs(20);

pub(crate) fn run(directory: PathBuf) {
    std::fs::create_dir_all(&directory).expect("the screenshot directory can be made");
    let platform = gpui_kit::platform::current_platform(false);
    let mut cx = VisualTestAppContext::with_asset_source(platform, Arc::new(CondrAssets::new()));
    cx.update(gpui_kit::init);

    let root = std::env::temp_dir().join(format!("condr-shots-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let workspace_root = root.join("dyson-web-monorepo");
    let local = root.join("local");
    for directory in [
        workspace_root.join("src"),
        workspace_root.join("public"),
        local.clone(),
    ] {
        std::fs::create_dir_all(directory).unwrap();
    }
    std::fs::write(workspace_root.join("package.json"), "{}\n").unwrap();
    std::fs::write(workspace_root.join("src/main.ts"), "export {}\n").unwrap();
    std::fs::write(workspace_root.join("README.md"), "# demo\n").unwrap();
    std::fs::write(local.join("hero.png"), vec![7; 900_000]).unwrap();
    std::fs::write(local.join("notes.txt"), "notes").unwrap();

    // A dev server on this machine's loopback, the forward's target; it also holds the
    // same local port, so the forward takes the next one and says so.
    let service = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let service_port = service.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for mut socket in service.incoming().flatten() {
            let mut request = [0; 512];
            let _ = socket.read(&mut request);
            let _ = socket.write_all(b"HTTP/1.0 200 OK\r\n\r\nhello");
        }
    });

    let server =
        BoundServer::bind(ServerConfig::ephemeral_tcp("127.0.0.1:0".parse().unwrap()).unwrap())
            .unwrap();
    let endpoint = server.endpoint().clone();
    let handle = server.handle();
    let server_thread = std::thread::spawn(move || server.run());
    let mut admin = ClientConnection::connect_overview(&endpoint, "screenshots").unwrap();
    let LayoutResult::WorkspaceCreated {
        workspace_id,
        pane_id,
        ..
    } = admin
        .layout(LayoutCommand::CreateWorkspace {
            root_directory: workspace_root.clone(),
            name: None,
        })
        .unwrap()
        .unwrap()
    else {
        panic!("expected a Workspace");
    };
    let initial = ClientConnection::connect(&endpoint, "condr-screenshots");

    let holder: Rc<RefCell<Option<Entity<Condr>>>> = Rc::default();
    let window = {
        let holder = Rc::clone(&holder);
        cx.open_offscreen_window(size(px(1280.), px(800.)), move |window, cx| {
            let view = cx.new(|cx| {
                Condr::new(
                    endpoint,
                    Some(initial.map_err(|error| error.to_string())),
                    config::LoadedConfig::read(None),
                    gui_state::LoadedState::default(),
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

    wait(&mut cx, "the GUI synchronized", |app| {
        view.read(app)
            .connection(1)
            .is_some_and(ServerConnection::is_synchronized)
            && view
                .read(app)
                .connection(1)
                .and_then(ServerConnection::session)
                .is_some_and(|session| !session.workspaces().is_empty())
    });
    shot(&mut cx, window, &directory, "01-workspace");

    // A kitty graphics image the shell prints from a file (ADR 0065), fetched over the
    // Device's Stream and drawn at the window's cells.
    let picture = local_path(&root, "kitty.png");
    RgbaImage::from_fn(96, 64, |x, y| {
        image::Rgba([(x * 255 / 95) as u8, (y * 255 / 63) as u8, 200, 255])
    })
    .save(&picture)
    .unwrap();
    let command = format!(
        "printf '\\033_Ga=T,f=100,t=f,q=2;%s\\033\\\\' \"$(printf %s '{}' | base64)\"; echo\r",
        picture.display()
    );
    admin
        .terminal(pane_id, condr_core::TerminalCommand::Text(command))
        .unwrap();
    wait(&mut cx, "the image drew", |app| {
        let Some(terminal) = view
            .read(app)
            .connection(1)
            .and_then(|connection| connection.model.terminals.get(&pane_id))
        else {
            return false;
        };
        terminal.view.images.first().is_some_and(|placement| {
            app.try_global::<crate::terminal_element::TerminalImages>()
                .is_some_and(|images| images.ready(&placement.image))
        })
    });
    shot(&mut cx, window, &directory, "01g-terminal-image");
    update(&mut cx, window, &view, |this, _, cx| {
        this.sidebar_folded_devices.insert(1);
        cx.notify();
    });
    shot(&mut cx, window, &directory, "01b-device-folded");
    update(&mut cx, window, &view, |this, _, cx| {
        this.sidebar_folded_devices.remove(&1);
        cx.notify();
    });

    // The Command Palette (ADR 0064): each page as it opens, and All with a query.
    use super::command_palette::{OpenCommandPalette, Page};
    for (page, name, query) in [
        (Page::All, "01c-palette-all", None),
        (Page::All, "01d-palette-query", Some("split")),
        (Page::Actions, "01e-palette-actions", None),
        (Page::GoTo, "01f-palette-go-to", None),
    ] {
        update(&mut cx, window, &view, |this, window, cx| {
            this.action_open_command_palette(&OpenCommandPalette { page }, window, cx)
        });
        if let Some(query) = query {
            cx.simulate_input(window, query);
        }
        shot(&mut cx, window, &directory, name);
        update(&mut cx, window, &view, |_, window, cx| {
            window.close_dialog(cx)
        });
        settle(&mut cx, window);
    }

    update(&mut cx, window, &view, |this, window, cx| {
        this.open_ports(PortsScope::All, window, cx)
    });
    shot(&mut cx, window, &directory, "02-ports-empty");

    for (expected, port) in [(1, service_port.to_string()), (2, "3000".to_owned())] {
        update(&mut cx, window, &view, |this, window, cx| {
            this.fill_ports_form(&port, "", window, cx);
        });
        update(&mut cx, window, &view, |this, window, cx| {
            this.submit_forward(1, window, cx)
        });
        wait(&mut cx, "the forward started", |app| {
            view.read(app).forward_local_ports(1).len() == expected
        });
    }
    // One connection through the first forward, so its card counts and answers.
    let local = cx.read(|app| view.read(app).forward_local_ports(1))[0];
    let mut browser = std::net::TcpStream::connect(("127.0.0.1", local)).unwrap();
    browser.write_all(b"GET / HTTP/1.0\r\n\r\n").unwrap();
    let mut answer = String::new();
    let _ = browser.read_to_string(&mut answer);
    shot(&mut cx, window, &directory, "03-ports-forwards");

    update(&mut cx, window, &view, |this, window, cx| {
        this.open_ports(PortsScope::Device(1), window, cx)
    });
    shot(&mut cx, window, &directory, "04-ports-one-device");

    update(&mut cx, window, &view, |this, window, cx| {
        this.close_ports(window, cx);
        this.toggle_changes_sidebar(cx);
    });
    let files = vec![
        local_path(&root, "hero.png"),
        local_path(&root, "notes.txt"),
    ];
    update(&mut cx, window, &view, move |this, window, cx| {
        this.upload_files(
            1,
            workspace_id,
            RelativePathBuf::new(),
            files.clone(),
            window,
            cx,
        );
    });
    wait(&mut cx, "the uploads finished", |app| {
        let states = view.read(app).transfer_states(1);
        states.len() == 2
            && states
                .iter()
                .all(|state| *state == super::transfers::TransferState::Done)
    });
    update(&mut cx, window, &view, |this, _, cx| {
        this.transfer_strip_open = true;
        cx.notify();
    });
    shot(&mut cx, window, &directory, "05-transfer-strip");

    // Transfers in flight, drawn in the states the dispatcher would run past: a download
    // under way, an upload waiting its turn, one cancelled and one failed.
    update(&mut cx, window, &view, move |this, _, cx| {
        use super::transfers::TransferState;
        this.show_transfer(
            1,
            workspace_id,
            "dist/bundle.js",
            false,
            (31 << 20, Some(48 << 20)),
            TransferState::Running,
        );
        this.show_transfer(
            1,
            workspace_id,
            "dataset.bin",
            true,
            (0, Some(4 << 30)),
            TransferState::Queued,
        );
        this.show_transfer(
            1,
            workspace_id,
            "logs.tar",
            false,
            (2 << 20, Some(9 << 20)),
            TransferState::Cancelled,
        );
        this.show_transfer(
            1,
            workspace_id,
            "build.zip",
            true,
            (0, None),
            TransferState::Failed("disk full".into()),
        );
        cx.notify();
    });
    shot(&mut cx, window, &directory, "06-transfers-in-flight");
    update(&mut cx, window, &view, |this, _, cx| {
        this.transfer_strip_open = false;
        cx.notify();
    });
    shot(&mut cx, window, &directory, "07-transfer-strip-folded");

    // The Device drops: its forwards wait, dimmed, on the Ports page.
    update(&mut cx, window, &view, |this, window, cx| {
        let _ = this.mark_disconnected(1, 0, "the network dropped".into());
        this.open_ports(PortsScope::All, window, cx);
    });
    shot(&mut cx, window, &directory, "08-ports-device-lost");

    // A finished download's toast, with its reveal action.
    let downloaded = local_path(&root, "notes.txt");
    update(&mut cx, window, &view, move |this, _, cx| {
        this.report_downloaded("biome.json", downloaded, cx)
    });
    settle(&mut cx, window);
    shot(&mut cx, window, &directory, "09-download-toast");

    drop(browser);
    let _ = cx.update_window(window, |_, window, _| window.remove_window());
    handle.stop();
    let _ = server_thread.join();
    let _ = std::fs::remove_dir_all(&root);
    eprintln!("screenshots saved in {}", directory.display());
}

fn local_path(root: &std::path::Path, name: &str) -> PathBuf {
    root.join("local").join(name)
}

fn update(
    cx: &mut VisualTestAppContext,
    window: AnyWindowHandle,
    view: &Entity<Condr>,
    change: impl FnOnce(&mut Condr, &mut Window, &mut Context<Condr>),
) {
    cx.update_window(window, |_, window, cx| {
        view.update(cx, |this, cx| change(this, window, cx))
    })
    .expect("the window is open");
    cx.run_until_parked();
}

fn wait(cx: &mut VisualTestAppContext, what: &str, until: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + WAIT;
    loop {
        cx.run_until_parked();
        if cx.read(|app| until(app)) {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting until {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn shot(
    cx: &mut VisualTestAppContext,
    window: AnyWindowHandle,
    directory: &std::path::Path,
    name: &str,
) {
    let path = directory.join(format!("{name}.png"));
    capture(cx, window)
        .save(&path)
        .expect("the screenshot is written");
    eprintln!("{}", path.display());
}

/// A toast fades in from its first frame on the wall clock: draw it once, then let it land.
fn settle(cx: &mut VisualTestAppContext, window: AnyWindowHandle) {
    let _ = capture(cx, window);
    std::thread::sleep(Duration::from_millis(500));
}

fn capture(cx: &mut VisualTestAppContext, window: AnyWindowHandle) -> RgbaImage {
    // Let the repaint the change asked for land, and a frame or two of layout settle.
    for _ in 0..3 {
        cx.run_until_parked();
        cx.advance_clock(Duration::from_millis(120));
        std::thread::sleep(Duration::from_millis(30));
    }
    cx.capture_screenshot(window)
        .expect("the window renders offscreen")
}
