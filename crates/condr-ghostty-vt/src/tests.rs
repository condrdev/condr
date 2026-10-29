use super::*;
use render::{Color, Underline};

fn terminal(cols: u16, rows: u16) -> Terminal {
    Terminal::new(Options {
        cols,
        rows,
        cell_width: 8,
        cell_height: 16,
        scrollback_bytes: 1 << 20,
        version: "Condr test".into(),
        terminfo_name: "xterm-256color".into(),
        resize_pulls_scrollback: true,
        colors: colors(0x0d1117),
    })
    .unwrap()
}

fn colors(background: u32) -> Colors {
    Colors {
        foreground: Rgb::from_u32(0xc9d1d9),
        background: Rgb::from_u32(background),
        cursor: Rgb::from_u32(0xf0f6fc),
        palette: std::array::from_fn(|index| Rgb::from_u32(index as u32 * 0x010101)),
    }
}

fn screen(render: &mut RenderState) -> Vec<String> {
    let mut rows = Vec::new();
    render
        .for_each_row(|row| {
            let mut text = String::new();
            row.for_each_cell(|cell| {
                text.push_str(if cell.text.is_empty() { " " } else { cell.text })
            })
            .unwrap();
            rows.push(text.trim_end().to_owned());
        })
        .unwrap();
    rows
}

#[test]
fn a_terminal_shows_what_its_program_wrote() {
    let mut terminal = terminal(20, 5);
    terminal.write(b"hello\r\n\x1b[1mworld\x1b[0m");
    assert_eq!(terminal.plain_text().unwrap(), "hello\nworld");
}

#[test]
fn queries_are_answered_in_stream_order_with_condrs_identity() {
    let mut terminal = terminal(20, 5);
    let events = terminal.write(b"\x1b]7501;?\x1b\\\x1b[c\x1b[>0q\x1b[18t");
    let [Event::WritePty(reply)] = events.as_slice() else {
        panic!("{events:?}")
    };
    let reply = String::from_utf8_lossy(reply);
    let (status, rest) = reply
        .split_once("\x1b[?62;22c")
        .expect("DA1 after the 7501 answer");
    assert!(status.starts_with("\x1b]7501;?"), "{reply:?}");
    assert!(rest.contains("Condr test"), "XTVERSION: {reply:?}");
    assert!(rest.ends_with("\x1b[8;5;20t"), "size report: {reply:?}");
}

#[test]
fn program_reports_arrive_as_events() {
    let mut terminal = terminal(20, 5);
    let events = terminal.write(
        b"\x07\x1b]0;a title\x07\x1b]7501;state=blocked:kind=question:progress=75:app=claude:msg=d2hpY2g/\x1b\\\x1b]777;notify;condr://agent;body\x07\x1b]133;A\x07\x1b]52;c;aGk=\x07",
    );
    assert_eq!(
        events,
        [
            Event::Bell,
            Event::TitleChanged("a title".into()),
            Event::ProgramStatus(ProgramStatus {
                state: ProgramState::Blocked,
                kind: Some(BlockedKind::Question),
                progress: Some(75),
                id: String::new(),
                app: "claude".into(),
                message: "which?".into(),
            }),
            Event::Notification {
                title: "condr://agent".into(),
                body: "body".into()
            },
            Event::PromptStart,
            Event::ClipboardWrite("hi".into()),
        ]
    );
}

#[test]
fn the_render_state_reads_cells_styles_and_wide_characters() {
    let mut terminal = terminal(10, 3);
    terminal.write("\x1b[1;4:3;38;5;9;48;2;1;2;3mA\x1b[0m中x".as_bytes());
    let mut render = RenderState::new().unwrap();
    render.update(&terminal).unwrap();
    assert_eq!(render.dirty(), Dirty::Full);
    let mut cells = Vec::new();
    render
        .for_each_row(|row| {
            if row.y == 0 {
                row.for_each_cell(|cell| {
                    cells.push((cell.text.to_owned(), cell.style, cell.width))
                })
                .unwrap();
            }
        })
        .unwrap();
    let (text, style, _) = &cells[0];
    assert_eq!(text, "A");
    assert!(style.bold);
    assert_eq!(style.underline, Underline::Curly);
    assert_eq!(style.foreground, Color::Palette(9));
    assert_eq!(style.background, Color::Rgb(Rgb { r: 1, g: 2, b: 3 }));
    assert_eq!((cells[1].0.as_str(), cells[1].2), ("中", CellWidth::Wide));
    assert_eq!(cells[2].2, CellWidth::SpacerTail);
    assert_eq!(cells[3].0, "x");
    assert_eq!(render.cursor().position, Some((4, 0)));

    // The caller clears every row it read as well as the state.
    render.for_each_row(|row| row.mark_clean()).unwrap();
    render.mark_clean();
    // The cursor leaves row 0 and lands on row 2; row 1 is untouched.
    terminal.write(b"\x1b[3;1Hz");
    render.update(&terminal).unwrap();
    let mut dirty = Vec::new();
    render.for_each_row(|row| dirty.push(row.dirty())).unwrap();
    assert_eq!(dirty, [true, false, true]);
}

#[test]
fn a_selection_follows_its_text_and_copies_it() {
    let mut terminal = terminal(10, 3);
    terminal.write(b"one two\r\nthree");
    terminal.select_word(Point::Viewport { x: 5, y: 0 }, &[]);
    assert_eq!(terminal.selection_text(None, true).as_deref(), Some("two"));
    terminal.write(b"\r\nfour\r\nfive");
    let mut render = RenderState::new().unwrap();
    render.update(&terminal).unwrap();
    assert_eq!(screen(&mut render), ["three", "four", "five"]);
    assert_eq!(
        terminal.selection_text(None, true).as_deref(),
        Some("two"),
        "scrolled into history"
    );
    terminal.select(None);
    assert!(!terminal.has_selection());
    let range = (
        Point::Viewport { x: 0, y: 1 },
        Point::Viewport { x: 3, y: 2 },
    );
    assert_eq!(
        terminal.selection_text(Some(range), true).as_deref(),
        Some("four\nfive")
    );
}

#[test]
fn the_viewport_scrolls_through_history() {
    let mut terminal = terminal(10, 2);
    terminal.write(b"a\r\nb\r\nc\r\nd");
    assert_eq!(
        terminal.scrollbar(),
        Scrollbar {
            total: 4,
            offset: 2,
            len: 2
        }
    );
    terminal.scroll_viewport(Scroll::Delta(-1));
    assert_eq!(terminal.scrollbar().offset, 1);
    terminal.scroll_viewport(Scroll::Top);
    let mut render = RenderState::new().unwrap();
    render.update(&terminal).unwrap();
    assert_eq!(screen(&mut render), ["a", "b"]);
    terminal.scroll_viewport(Scroll::Bottom);
    assert_eq!(terminal.scrollbar().offset, 2);
}

#[test]
fn a_panicking_callback_answers_nothing_and_is_counted() {
    let mut terminal = terminal(10, 2);
    let calls = AssertUnwindSafe(|| -> bool { panic!("injected") });
    assert!(!guarded(terminal.callbacks.cast(), false, |_| calls()));
    assert_eq!(terminal.callback_panics(), 1);
    terminal.write(b"still parses");
    assert_eq!(terminal.plain_text().unwrap(), "still parses");
}

#[test]
fn mouse_reports_follow_the_programs_tracking_mode_and_format() {
    let mut terminal = terminal(20, 5);
    let mut mouse = MouseEncoder::new().unwrap();
    let event = |action, button, column| MouseEvent {
        action,
        button,
        mods: Mods::default(),
        column,
        row: 2,
    };
    let press = event(MouseAction::Press, Some(MouseButton::Left), 4);
    assert!(mouse.encode(&terminal, &press).is_empty(), "no tracking");

    terminal.write(b"\x1b[?1000h");
    assert_eq!(mouse.encode(&terminal, &press), b"\x1b[M %#");
    let release = event(MouseAction::Release, Some(MouseButton::Left), 4);
    assert_eq!(mouse.encode(&terminal, &release), b"\x1b[M#%#");

    terminal.write(b"\x1b[?1006h");
    assert_eq!(mouse.encode(&terminal, &release), b"\x1b[<0;5;3m");
    let wheel = event(MouseAction::Press, Some(MouseButton::WheelDown), 4);
    assert_eq!(mouse.encode(&terminal, &wheel), b"\x1b[<65;5;3M");

    terminal.write(b"\x1b[?1003h");
    let hover = event(MouseAction::Motion, None, 7);
    assert_eq!(mouse.encode(&terminal, &hover), b"\x1b[<35;8;3M");

    terminal.write(b"\x1b[?1006l\x1b[?1015h");
    assert_eq!(mouse.encode(&terminal, &press), b"\x1b[32;5;3M");
    // SGR pixels: the middle of column 4, row 2 in 8x16 cells.
    terminal.write(b"\x1b[?1016h");
    assert_eq!(mouse.encode(&terminal, &press), b"\x1b[<0;36;40M");
}

#[test]
fn pastes_drop_control_bytes_and_follow_bracketed_paste() {
    assert_eq!(paste("one\ntwo\x03", false), b"one\rtwo ");
    assert_eq!(paste("a\x1b[201~b", true), b"\x1b[200~a [201~b\x1b[201~");
    assert_eq!(focus_report(true), b"\x1b[I");
    assert_eq!(focus_report(false), b"\x1b[O");
}

#[test]
fn the_key_encoder_tells_when_modify_other_keys_is_on() {
    let mut terminal = terminal(20, 5);
    let mut keys = KeyEncoder::new().unwrap();
    assert!(!keys.modify_other_keys(&terminal));
    terminal.write(b"\x1b[>4;2m");
    assert!(keys.modify_other_keys(&terminal));
    terminal.write(b"\x1b[>4;0m");
    assert!(!keys.modify_other_keys(&terminal));
}

#[test]
fn colour_queries_answer_the_colours_set_and_a_scheme_flip_is_reported() {
    let mut terminal = terminal(20, 5);
    let replies = |terminal: &mut Terminal, query: &[u8]| -> Vec<u8> {
        terminal
            .write(query)
            .into_iter()
            .filter_map(|event| match event {
                Event::WritePty(bytes) => Some(bytes),
                _ => None,
            })
            .flatten()
            .collect()
    };
    assert_eq!(replies(&mut terminal, b"\x1b[?996n"), b"\x1b[?997;1n");
    assert_eq!(
        replies(&mut terminal, b"\x1b]11;?\x1b\\"),
        b"\x1b]11;rgb:0d0d/1111/1717\x1b\\"
    );

    terminal.write(b"\x1b[?2031h");
    assert_eq!(
        terminal.set_colors(&colors(0x0e1118)).unwrap(),
        None,
        "still dark"
    );
    assert_eq!(
        terminal.set_colors(&colors(0xfafafa)).unwrap(),
        Some(b"\x1b[?997;2n".to_vec())
    );
    assert_eq!(replies(&mut terminal, b"\x1b[?996n"), b"\x1b[?997;2n");
    assert_eq!(
        replies(&mut terminal, b"\x1b]11;?\x1b\\"),
        b"\x1b]11;rgb:fafa/fafa/fafa\x1b\\"
    );
    // A program's own background outlives the next change of defaults.
    terminal.write(b"\x1b]11;#102030\x1b\\");
    terminal.set_colors(&colors(0xffffff)).unwrap();
    assert_eq!(
        replies(&mut terminal, b"\x1b]11;?\x1b\\"),
        b"\x1b]11;rgb:1010/2020/3030\x1b\\"
    );
}

#[test]
fn progress_reports_arrive_as_events() {
    let mut terminal = terminal(20, 5);
    let progress = |state, percent| Event::Progress(Some(Progress { state, percent }));
    assert_eq!(
        terminal.write(b"\x1b]9;4;1;42\x07\x1b]9;4;3\x07\x1b]9;4;2\x07\x1b]9;4;0\x07"),
        [
            progress(ProgressState::Normal, Some(42)),
            progress(ProgressState::Indeterminate, None),
            progress(ProgressState::Error, None),
            Event::Progress(None),
        ]
    );
}

#[test]
fn shell_marks_report_commands_and_the_viewport_jumps_between_prompts() {
    let mut terminal = terminal(20, 3);
    assert!(!terminal.cursor_at_prompt(), "no marks, no prompt");
    let mut events = Vec::new();
    for command in 0..6 {
        events.extend(terminal.write(
            format!("\x1b]133;A\x07$ \x1b]133;B\x07cmd{command}\r\n\x1b]133;C\x07out{command}\r\n\x1b]133;D;{command}\x07").as_bytes(),
        ));
    }
    events.extend(terminal.write(b"\x1b]133;A\x07$ \x1b]133;B\x07"));
    assert_eq!(
        &events[..3],
        [
            Event::PromptStart,
            Event::CommandStart,
            Event::CommandEnd(Some(0))
        ]
    );
    assert!(terminal.cursor_at_prompt());

    let bottom = terminal.scrollbar().offset;
    terminal.scroll_viewport(Scroll::Prompts(-2));
    let up_two = terminal.scrollbar().offset;
    assert!(up_two < bottom, "{up_two} < {bottom}");
    terminal.scroll_viewport(Scroll::Prompts(1));
    assert!(terminal.scrollbar().offset > up_two);

    terminal.write(b"\x1b[?1049h");
    assert!(
        !terminal.cursor_at_prompt(),
        "the alternate screen has no prompt"
    );
}

#[test]
fn the_librarys_log_reaches_tracing() {
    use std::sync::{Arc, Mutex};
    #[derive(Clone, Default)]
    struct Captured(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for Captured {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    tracing::subscriber::with_default(subscriber, || {
        let mut terminal = terminal(20, 5);
        terminal.write(b"\x1b[999;999z\x1b]9999;nothing\x07");
    });
    let log = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
    assert!(log.contains("libghostty"), "{log}");
}

#[test]
fn kitty_graphics_and_glyph_queries_go_unanswered() {
    let mut terminal = terminal(20, 5);
    let replies = terminal
        .write(b"\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\\x1b_25a1;s\x1b\\")
        .into_iter()
        .filter(|event| matches!(event, Event::WritePty(_)))
        .count();
    assert_eq!(replies, 0, "Condr shows neither images nor custom glyphs");
}
