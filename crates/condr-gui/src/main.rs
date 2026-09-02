#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod apca;
mod app;
mod color_scheme;
mod terminal_element;

pub(crate) use app::{Condr, ConnectionKey};

fn main() {
    app::run();
}
