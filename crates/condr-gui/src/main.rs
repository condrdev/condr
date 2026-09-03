// A Windows-subsystem binary so double-clicking never opens a console; `cli` attaches to
// the parent console when arguments are given. Not for the test harness: a windowless
// test process has no console, so every console child it starts (git in the test
// helpers) would flash its own window.
#![cfg_attr(all(target_os = "windows", not(test)), windows_subsystem = "windows")]

mod apca;
mod app;
mod cli;
mod color_scheme;
mod terminal_element;

pub(crate) use app::{Condr, ConnectionKey};

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments.is_empty() {
        app::run();
    } else {
        cli::run(arguments);
    }
}
