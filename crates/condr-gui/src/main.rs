#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod app;
mod cli;
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
