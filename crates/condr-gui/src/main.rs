// A Windows-subsystem binary so double-clicking never opens a console. Not for the test
// harness: a windowless test process has no console, so every console child it starts
// (git in the test helpers) would flash its own window.
#![cfg_attr(all(target_os = "windows", not(test)), windows_subsystem = "windows")]

mod apca;
mod app;
mod assets;
mod color_scheme;
mod terminal_element;

pub(crate) use app::{Condr, ConnectionKey};

fn main() {
    // The one flag the GUI answers on the command line, so a package can be asked
    // which build it holds without a display.
    if std::env::args().nth(1).as_deref() == Some("--version") {
        println!("condr-gui {}", condr_core::version_text());
        return;
    }
    app::run();
}
