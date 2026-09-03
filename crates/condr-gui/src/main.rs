//! `condr` is one console-subsystem binary, as herdr is: with arguments it is a CLI that
//! prints, waits and exits; without them it starts the GUI in a detached copy of itself
//! (see `launch`). No `windows_subsystem = "windows"`: that would stop shells from
//! waiting for the CLI and swallow its output.

mod apca;
mod app;
mod cli;
mod color_scheme;
mod launch;
mod terminal_element;

pub(crate) use app::{Condr, ConnectionKey};

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    match arguments.as_slice() {
        [] => launch::run_gui(false),
        // The detached copy `launch` starts, and the development escape hatch.
        [flag] if flag == "--detached" || flag == "--foreground" => launch::run_gui(true),
        _ => cli::run(arguments),
    }
}
