#[path = "../../packaging/windows.rs"]
mod windows;

fn main() {
    windows::embed_icon("condr");
}
