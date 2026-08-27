use std::path::PathBuf;

use gpui::*;
use gpui_component::Root;

struct Murmur;

impl Render for Murmur {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(murmur_core::APP_NAME)
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|arg| arg == "--server") {
        run_server(&args);
        return;
    }

    let endpoint =
        murmur_server::ensure_local_server().expect("failed to discover or start murmur-server");
    let _client = murmur_server::ClientConnection::connect(&endpoint, "murmur-gui")
        .expect("failed to connect to murmur-server");
    let app = gpui_platform::application();

    app.run(move |cx| {
        gpui_component::init(cx);

        cx.spawn(async move |cx| {
            cx.open_window(WindowOptions::default(), |window, cx| {
                let view = cx.new(|_| Murmur);
                cx.new(|cx| Root::new(view, window, cx))
            })
            .expect("failed to open Murmur window");
        })
        .detach();
    });
}

fn run_server(args: &[String]) {
    let endpoint = args
        .windows(2)
        .find(|pair| pair[0] == "--endpoint")
        .map(|pair| murmur_server::Endpoint::local(PathBuf::from(&pair[1])))
        .unwrap_or_else(|| murmur_server::ServerConfig::default().endpoint);
    if let Err(error) = murmur_server::run(murmur_server::ServerConfig { endpoint }) {
        eprintln!("murmur-gui server: {error}");
        std::process::exit(1);
    }
}
