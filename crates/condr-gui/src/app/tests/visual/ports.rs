use super::*;
use crate::app::ports::PortsScope;
use std::io::{Read as _, Write as _};

/// A loopback service on "the Device" that echoes each connection.
fn echo_service() -> u16 {
    let service = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = service.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for mut socket in service.incoming().flatten() {
            std::thread::spawn(move || {
                let mut buffer = [0; 64];
                while let Ok(read) = socket.read(&mut buffer) {
                    if read == 0 || socket.write_all(&buffer[..read]).is_err() {
                        break;
                    }
                }
            });
        }
    });
    port
}

/// Whether a connection to `local` reaches the echo service and back.
fn echoes(local: u16) -> bool {
    let Ok(mut socket) = std::net::TcpStream::connect(("127.0.0.1", local)) else {
        return false;
    };
    let _ = socket.set_read_timeout(Some(Duration::from_secs(2)));
    let mut echoed = [0; 4];
    socket.write_all(b"ping").is_ok()
        && socket.read_exact(&mut echoed).is_ok()
        && &echoed == b"ping"
}

/// The Ports page end to end (ADR 0061): the sidebar's button opens it over the content
/// area, its form forwards a port of a TCP Device, a connection to the local port reaches
/// the Device's loopback, Stop ends it, and choosing a Device or a Pane leaves the page.
#[test]
fn the_ports_page_forwards_a_devices_port_and_stops_it() {
    let _serial_guard = acquire_visual_test_lock();
    let remote = echo_service();

    let mut cx = TestAppContext::single();
    cx.update(gpui_kit::init);
    let (server, endpoint) = start_tcp_server();
    let (view, window, _server) = connected_condr_with(&mut cx, server, endpoint);
    window.update(|window, cx| _ = window.draw(cx));
    assert!(window.debug_bounds("ports-page").is_none());
    // A remote Device's header carries its own button.
    assert!(window.debug_bounds("device-ports-1").is_some());

    let open = window
        .debug_bounds("open-ports")
        .expect("the sidebar has a Ports button");
    window.simulate_click(open.center(), Modifiers::default());
    window.run_until_parked();
    window.update(|window, cx| _ = window.draw(cx));
    assert!(window.debug_bounds("ports-page").is_some());
    assert!(window.debug_bounds("condr-right-sidebar").is_none());
    // The only remote Device needs no choosing.
    assert!(window.debug_bounds("ports-device").is_none());
    window.read(|app| {
        let view = view.read(app);
        assert!(
            view.windows
                .iter()
                .any(|v| v.ports == Some(PortsScope::All))
        );
    });

    // A port that is not one is refused under the form.
    window
        .update(|window, cx| view.update(cx, |this, cx| this.fill_ports_form("0", "", window, cx)));
    let submit = window.debug_bounds("forward-port").unwrap();
    window.simulate_click(submit.center(), Modifiers::default());
    window.run_until_parked();
    assert!(
        window
            .read(|app| view.read(app).ports_form_error())
            .is_some()
    );

    window.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.fill_ports_form(&remote.to_string(), "", window, cx)
        })
    });
    window.update(|window, cx| _ = window.draw(cx));
    let submit = window.debug_bounds("forward-port").unwrap();
    window.simulate_click(submit.center(), Modifiers::default());
    assert!(
        wait_until(window, |window| {
            window.read(|app| !view.read(app).forward_local_ports(1).is_empty())
        }),
        "the forward did not start: {:?}",
        window.read(|app| view.read(app).ports_form_error())
    );
    let local = window.read(|app| view.read(app).forward_local_ports(1))[0];
    let mut socket = std::net::TcpStream::connect(("127.0.0.1", local)).unwrap();
    socket.set_read_timeout(Some(TEST_TIMEOUT)).unwrap();
    socket.write_all(b"through the GUI").unwrap();
    let mut echoed = [0; 15];
    socket.read_exact(&mut echoed).unwrap();
    assert_eq!(&echoed, b"through the GUI");
    drop(socket);

    window.update(|window, cx| _ = window.draw(cx));
    let stop = window
        .debug_bounds(leaked_selector(format!("forward-stop-1-{remote}")))
        .expect("the forward's card has Stop");
    window.simulate_click(stop.center(), Modifiers::default());
    window.run_until_parked();
    assert!(window.read(|app| view.read(app).forward_local_ports(1).is_empty()));
    assert!(std::net::TcpStream::connect(("127.0.0.1", local)).is_err());

    // Choosing a Device in the sidebar leaves the page.
    window.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.window_view_mut(window).ports = Some(PortsScope::Device(1));
            this.select_server(1, window, cx);
        })
    });
    window.read(|app| {
        assert!(view.read(app).windows.iter().all(|v| v.ports.is_none()));
    });

    // So does choosing an Agent's Pane.
    let root = TestDirectory::new("ports-leave-by-pane");
    let (_, _, pane_id) = super::dock::create_workspace(window, &view, &root);
    window.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.open_ports(PortsScope::All, window, cx);
            assert!(this.select_pane(1, pane_id, window, cx));
        })
    });
    window.read(|app| {
        assert!(view.read(app).windows.iter().all(|v| v.ports.is_none()));
    });
}

/// A forward outlives the terminal connection's reconnect and carries connections again
/// once it is back; Disconnect ends it and frees the port (ADR 0061).
#[test]
fn a_forward_survives_a_reconnect_and_disconnect_ends_it() {
    let _serial_guard = acquire_visual_test_lock();
    let remote = echo_service();
    let mut cx = TestAppContext::single();
    cx.update(gpui_kit::init);
    let (server, endpoint) = start_tcp_server();
    let (view, window, _server) = connected_condr_with(&mut cx, server, endpoint);
    window.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.open_ports(PortsScope::Device(1), window, cx);
            this.fill_ports_form(&remote.to_string(), "", window, cx);
        })
    });
    window.update(|window, cx| view.update(cx, |this, cx| this.submit_forward(1, window, cx)));
    assert!(wait_until(window, |window| {
        window.read(|app| !view.read(app).forward_local_ports(1).is_empty())
    }));
    let local = window.read(|app| view.read(app).forward_local_ports(1))[0];
    assert!(echoes(local));

    // The network drops and the GUI reconnects by itself.
    window.update(|_, cx| {
        view.update(cx, |this, _| {
            let _ = this.mark_disconnected(1, 0, "the network dropped".into());
            this.start_connect(1);
        })
    });
    assert!(wait_until(window, |window| {
        window.read(|app| {
            view.read(app)
                .connection(1)
                .is_some_and(ServerConnection::is_synchronized)
        })
    }));
    assert_eq!(
        window.read(|app| view.read(app).forward_local_ports(1)),
        vec![local]
    );
    assert!(
        wait_until(window, |_| echoes(local)),
        "the forward carries connections again after the reconnect"
    );

    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.disconnect_server(1);
        })
    });
    assert!(window.read(|app| view.read(app).forward_local_ports(1).is_empty()));
    assert!(std::net::TcpStream::connect(("127.0.0.1", local)).is_err());
}
