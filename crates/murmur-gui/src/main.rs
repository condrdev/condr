use gpui::*;
use gpui_component::Root;

struct Murmur;

impl Render for Murmur {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(murmur_core::APP_NAME)
    }
}

fn main() {
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
