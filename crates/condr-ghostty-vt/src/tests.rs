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
        foreground: Rgb::from_u32(0xc9d1d9),
        background: Rgb::from_u32(0x0d1117),
        cursor: Rgb::from_u32(0xf0f6fc),
        palette: std::array::from_fn(|index| Rgb::from_u32(index as u32 * 0x010101)),
    })
    .unwrap()
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
        b"\x07\x1b]0;a title\x07\x1b]7501;state=blocked:kind=question:app=claude:msg=d2hpY2g/\x1b\\\x1b]777;notify;condr://agent;body\x07\x1b]133;A\x07\x1b]52;c;aGk=\x07",
    );
    assert_eq!(
        events,
        [
            Event::Bell,
            Event::TitleChanged("a title".into()),
            Event::ProgramStatus(ProgramStatus {
                state: ProgramState::Blocked,
                kind: Some(BlockedKind::Question),
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
    assert_eq!(terminal.selection_text(None).as_deref(), Some("two"));
    terminal.write(b"\r\nfour\r\nfive");
    let mut render = RenderState::new().unwrap();
    render.update(&terminal).unwrap();
    assert_eq!(screen(&mut render), ["three", "four", "five"]);
    assert_eq!(
        terminal.selection_text(None).as_deref(),
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
        terminal.selection_text(Some(range)).as_deref(),
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
