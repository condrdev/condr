//! Terminal frame wire benchmark (ADR 0028): bytes and encode/decode time for a full
//! 220×50 transcript-like screen, a one-row delta (a spinner) and a six-row delta
//! (streaming output), through the same public codec the Server and GUI use.
//!
//! `cargo test --release -p condr-core --test terminal_wire_bench -- --ignored --nocapture`

use condr_core::protocol::{
    PaneTerminalFrame, decode_pane_terminal_frame, encode_pane_terminal_frame,
};
use condr_core::{
    PaneId, TerminalCell, TerminalCellRun, TerminalColor, TerminalCursor, TerminalCursorShape,
    TerminalMouseTracking, TerminalSize, TerminalView, TerminalViewDelta, TerminalViewFrame,
};
use smol_str::SmolStr;
use std::hint::black_box;
use std::time::Instant;

const ROWS: usize = 50;
const COLUMNS: usize = 220;

struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
}

/// About a third of each row is prose with occasional color and bold, the rest blank.
fn cells() -> Vec<TerminalCell> {
    let mut random = Lcg(7);
    let blank = TerminalCell {
        text: SmolStr::new_static(" "),
        foreground: TerminalColor::Named(256),
        background: TerminalColor::Named(257),
        flags: 0,
        hyperlink: None,
    };
    let mut cells = Vec::with_capacity(ROWS * COLUMNS);
    for row in 0..ROWS {
        for column in 0..COLUMNS {
            if column >= 60 + (row * 7 % 90) {
                cells.push(blank.clone());
                continue;
            }
            let r = random.next();
            let character = if r.is_multiple_of(6) {
                ' '
            } else {
                (b'a' + (r % 26) as u8) as char
            };
            let foreground = match r % 20 {
                0 => TerminalColor::Indexed((r % 256) as u8),
                1 => TerminalColor::Rgb {
                    red: r as u8,
                    green: (r >> 8) as u8,
                    blue: (r >> 16) as u8,
                },
                _ => TerminalColor::Named(256),
            };
            cells.push(TerminalCell {
                text: SmolStr::new(character.to_string()),
                foreground,
                background: TerminalColor::Named(257),
                flags: u16::from(r.is_multiple_of(15)),
                hyperlink: None,
            });
        }
    }
    cells
}

fn cursor() -> Option<TerminalCursor> {
    Some(TerminalCursor {
        row: 49,
        column: 12,
        shape: TerminalCursorShape::Block,
        blinking: true,
    })
}

fn full(cells: &[TerminalCell]) -> PaneTerminalFrame {
    PaneTerminalFrame {
        pane_id: PaneId::from_u64(1),
        frame: TerminalViewFrame::Full(TerminalView {
            revision: 4242,
            size: TerminalSize {
                rows: ROWS as u16,
                columns: COLUMNS as u16,
                cell_width: 9,
                cell_height: 18,
            },
            display_offset: 0,
            mouse_tracking: TerminalMouseTracking::None,
            cells: cells.to_vec(),
            cursor: cursor(),
            selection: None,
        }),
    }
}

fn delta(cells: &[TerminalCell], rows: &[usize]) -> PaneTerminalFrame {
    PaneTerminalFrame {
        pane_id: PaneId::from_u64(1),
        frame: TerminalViewFrame::Delta(TerminalViewDelta {
            base_revision: 4242,
            revision: 4243,
            display_offset: 0,
            mouse_tracking: TerminalMouseTracking::None,
            cursor: cursor(),
            selection: None,
            runs: rows
                .iter()
                .map(|&row| TerminalCellRun {
                    start: (row * COLUMNS) as u32,
                    cells: cells[row * COLUMNS..(row + 1) * COLUMNS].to_vec(),
                })
                .collect(),
        }),
    }
}

/// Best of five runs of `iterations` calls, in microseconds per call.
fn time<R>(iterations: u32, mut run: impl FnMut() -> R) -> f64 {
    for _ in 0..iterations / 10 {
        black_box(run());
    }
    (0..5)
        .map(|_| {
            let started = Instant::now();
            for _ in 0..iterations {
                black_box(run());
            }
            started.elapsed().as_secs_f64() * 1e6 / f64::from(iterations)
        })
        .fold(f64::MAX, f64::min)
}

#[test]
#[ignore = "benchmark; run in release with --ignored --nocapture"]
fn terminal_frame_wire_benchmark() {
    let cells = cells();
    let workloads = [
        ("full screen", full(&cells), 300),
        ("delta: 1 row", delta(&cells, &[49]), 3000),
        (
            "delta: 6 rows",
            delta(&cells, &[44, 45, 46, 47, 48, 49]),
            3000,
        ),
    ];
    println!("{ROWS}x{COLUMNS} screen; best of 5, µs per call");
    for (label, frame, iterations) in workloads {
        let bytes = encode_pane_terminal_frame(&frame).unwrap();
        assert_eq!(
            decode_pane_terminal_frame(&bytes).unwrap(),
            Some(frame.clone())
        );
        let encode = time(iterations, || encode_pane_terminal_frame(&frame).unwrap());
        let decode = time(iterations, || decode_pane_terminal_frame(&bytes).unwrap());
        println!(
            "{label:<14} {:>8} B  encode {encode:>8.1}  decode {decode:>8.1}",
            bytes.len()
        );
    }
}
