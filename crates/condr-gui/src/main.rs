mod app;
mod terminal_element;

pub(crate) use app::{Condr, ConnectionKey};

fn main() {
    app::run();
}
