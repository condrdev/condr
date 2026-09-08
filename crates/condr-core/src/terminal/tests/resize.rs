use super::*;

#[test]
fn pending_resize_keeps_only_the_latest_request() {
    let resize = ResizeControl::default();
    resize.request(TerminalSize::new(10, 40)).unwrap();
    let latest = TerminalSize::new(20, 80);
    resize.request(latest).unwrap();

    assert_eq!(resize.next(), Some(latest));
    resize.stop();
    assert_eq!(resize.next(), None);
    assert_eq!(
        resize
            .request(TerminalSize::new(30, 120))
            .unwrap_err()
            .kind(),
        io::ErrorKind::BrokenPipe
    );
}

#[test]
fn resize_worker_failure_stops_accepting_requests() {
    let initial_size = TerminalSize::new(5, 20);
    let requested_size = TerminalSize::new(10, 40);
    let control = Arc::new(ResizeControl::default());
    control.request(requested_size).unwrap();
    let current_size = Arc::new(Mutex::new(initial_size));
    let (event_proxy, _pending_replies, _notices) =
        TerminalEventProxy::new(Arc::clone(&current_size));
    let terminal = Arc::new(Mutex::new(Term::new(
        Config::default(),
        &initial_size,
        event_proxy,
    )));
    let (updates, _update_receiver) = mpsc::channel();

    let error = resize_loop(
        Arc::clone(&control),
        Weak::<Mutex<Box<dyn MasterPty + Send>>>::new(),
        terminal,
        current_size,
        Arc::new(AtomicU64::new(0)),
        updates,
    )
    .unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
    assert_eq!(
        control.request(requested_size).unwrap_err().kind(),
        io::ErrorKind::BrokenPipe
    );
}

#[test]
fn terminal_is_resized_before_the_pty() {
    let initial_size = TerminalSize::new(5, 20);
    let requested_size = TerminalSize::new(10, 40);
    let current_size = Arc::new(Mutex::new(initial_size));
    let (event_proxy, _pending_replies, _notices) =
        TerminalEventProxy::new(Arc::clone(&current_size));
    let terminal = Arc::new(Mutex::new(Term::new(
        Config::default(),
        &initial_size,
        event_proxy,
    )));
    let revision = AtomicU64::new(0);
    let (updates, _update_receiver) = mpsc::channel();
    let mut size_seen_by_pty = None;

    resize_terminal_and_pty(
        &terminal,
        &current_size,
        &revision,
        &updates,
        requested_size,
        |terminal, _| {
            size_seen_by_pty = Some((terminal.screen_lines(), terminal.columns()));
            Ok(())
        },
    )
    .unwrap();

    assert_eq!(
        size_seen_by_pty,
        Some((
            usize::from(requested_size.rows),
            usize::from(requested_size.columns)
        ))
    );
}

#[test]
fn failed_pty_resize_still_publishes_the_new_vt_size() {
    let initial_size = TerminalSize::new(5, 20);
    let requested_size = TerminalSize::new(10, 40);
    let current_size = Arc::new(Mutex::new(initial_size));
    let (event_proxy, _pending_replies, _notices) =
        TerminalEventProxy::new(Arc::clone(&current_size));
    let terminal = Arc::new(Mutex::new(Term::new(
        Config::default(),
        &initial_size,
        event_proxy,
    )));
    let revision = AtomicU64::new(0);
    let (updates, update_receiver) = mpsc::channel();

    let error = resize_terminal_and_pty(
        &terminal,
        &current_size,
        &revision,
        &updates,
        requested_size,
        |_, _| Err(io::Error::other("PTY resize failed")),
    )
    .unwrap_err();

    assert_eq!(error.to_string(), "PTY resize failed");
    assert_eq!(*current_size.lock().unwrap(), requested_size);
    let terminal = terminal.lock().unwrap();
    assert_eq!(terminal.screen_lines(), usize::from(requested_size.rows));
    assert_eq!(terminal.columns(), usize::from(requested_size.columns));
    drop(terminal);
    assert_eq!(revision.load(Ordering::Acquire), 1);
    assert_eq!(update_receiver.recv().unwrap(), TerminalUpdate::View(1));
}
