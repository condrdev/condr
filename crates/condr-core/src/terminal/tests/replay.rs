//! Recorded PTY streams replayed through the VT and compared with the screens it showed
//! before (ADR 0051). A change of engine or Ghostty pin that moves them regenerates the
//! files with `CONDR_UPDATE_VT_FIXTURES=1`, and its diff is reviewed line by line.

use super::*;
use std::fmt::Write as _;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/vt");
/// The size of the Pane each session was recorded in.
const SIZE: TerminalSize = TerminalSize::new(24, 80);
/// About this many screens are recorded per session, at least `MIN_CHECKPOINT_BYTES` of
/// input apart.
const SCREENS: usize = 16;
const MIN_CHECKPOINT_BYTES: usize = 512;
const PIECE_BYTES: usize = 256;
const SYNCHRONIZED_BEGIN: &[u8] = b"\x1b[?2026h";
const SYNCHRONIZED_END: &[u8] = b"\x1b[?2026l";
/// A wide character's second cell and the cell a wide character wrapped away from.
const SPACER_FLAGS: u16 = 1 << 6 | 1 << 10;

#[test]
fn recorded_sessions_show_the_screens_they_showed_before() {
    let update = std::env::var_os("CONDR_UPDATE_VT_FIXTURES").is_some();
    let mut failures = Vec::new();
    for name in ["shell", "vim", "claude", "codex"] {
        let stream = std::fs::read(format!("{FIXTURES}/{name}.pty")).unwrap();
        let screens = replay(&stream);
        let path = format!("{FIXTURES}/{name}.screens");
        if update {
            std::fs::write(&path, &screens).unwrap();
        } else if let Some(difference) =
            first_difference(&std::fs::read_to_string(&path).unwrap(), &screens)
        {
            failures.push(format!("{name}: {difference}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

/// The screens after each checkpoint and at the end. A checkpoint only falls outside a
/// synchronized update (mode 2026), whose frame an engine may show at any point.
fn replay(stream: &[u8]) -> String {
    let checkpoint = (stream.len() / SCREENS).max(MIN_CHECKPOINT_BYTES);
    let mut terminal = ReplayTerminal::new(SIZE);
    let mut screens = String::new();
    let (mut fed, mut since_checkpoint) = (0, 0);
    for (piece, synchronized) in pieces(stream) {
        terminal.write(piece);
        fed += piece.len();
        since_checkpoint += piece.len();
        if !synchronized && since_checkpoint >= checkpoint {
            screens += &screen(fed, &terminal.view());
            since_checkpoint = 0;
        }
    }
    screens + &screen(fed, &terminal.view())
}

/// The stream cut after every mode 2026 switch and every `PIECE_BYTES`, each piece with
/// whether a synchronized update is open after it.
fn pieces(stream: &[u8]) -> Vec<(&[u8], bool)> {
    let mut pieces = Vec::new();
    let (mut start, mut at, mut synchronized) = (0, 0, false);
    while at < stream.len() {
        let rest = &stream[at..];
        if rest.starts_with(SYNCHRONIZED_BEGIN) || rest.starts_with(SYNCHRONIZED_END) {
            synchronized = rest.starts_with(SYNCHRONIZED_BEGIN);
            at += SYNCHRONIZED_BEGIN.len();
        } else {
            at += 1;
            if at - start < PIECE_BYTES {
                continue;
            }
        }
        pieces.push((&stream[start..at], synchronized));
        start = at;
    }
    if start < stream.len() {
        pieces.push((&stream[start..], synchronized));
    }
    pieces
}

fn screen(fed: usize, view: &TerminalView) -> String {
    let mut screen = format!("@ {fed} bytes\n");
    match view.cursor {
        Some(cursor) => {
            let shape = cursor.shape;
            writeln!(screen, "cursor {},{} {shape:?}", cursor.row, cursor.column).unwrap();
        }
        None => screen.push_str("cursor none\n"),
    }
    for row in 0..view.size.rows {
        let text: String = (0..view.size.columns)
            .filter_map(|column| view.cell(row, column))
            .filter(|cell| cell.flags & SPACER_FLAGS == 0)
            .map(|cell| cell.text.as_str())
            .collect();
        writeln!(screen, "|{}", text.trim_end()).unwrap();
    }
    screen
}

fn first_difference(expected: &str, actual: &str) -> Option<String> {
    let screens = |text: &str| -> Vec<String> {
        text.split("@ ")
            .filter(|screen| !screen.is_empty())
            .map(String::from)
            .collect()
    };
    let (expected, actual) = (screens(expected), screens(actual));
    if let Some((expected, actual)) = expected
        .iter()
        .zip(&actual)
        .find(|(expected, actual)| expected != actual)
    {
        return Some(format!("expected\n@ {expected}\nfound\n@ {actual}"));
    }
    (expected.len() != actual.len()).then(|| {
        format!(
            "{} screens expected, {} found",
            expected.len(),
            actual.len()
        )
    })
}

/// The engine under test, fed the stream with no PTY behind it.
struct ReplayTerminal {
    terminal: Terminal,
    parser: Processor,
}

impl ReplayTerminal {
    fn new(size: TerminalSize) -> Self {
        let (event_proxy, _replies, _notices) = TerminalEventProxy::new(Arc::new(Mutex::new(size)));
        Self {
            terminal: Term::new(terminal_config(), &size, event_proxy),
            parser: Processor::new(),
        }
    }

    fn write(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.terminal, bytes);
    }

    fn view(&self) -> TerminalView {
        snapshot_terminal(&self.terminal, SIZE, 0)
    }
}
