//! What a view shows instead of its content, so every view says it the same way.

use super::preview::file_size_text;
use super::*;
use rust_i18n::t;

/// Nothing to show, said in the middle of the view.
pub(super) fn empty_state(text: impl Into<SharedString>, cx: &App) -> AnyElement {
    centered(text.into(), cx.theme().muted_foreground)
}

/// Something went wrong, said in the middle of the view in red.
pub(super) fn failed_state(reason: impl Into<SharedString>, cx: &App) -> AnyElement {
    centered(reason.into(), cx.theme().danger)
}

fn centered(text: SharedString, color: Hsla) -> AnyElement {
    div()
        .flex_1()
        .flex()
        .items_center()
        .justify_center()
        .p_4()
        .text_sm()
        .text_color(color)
        .child(text)
        .into_any_element()
}

/// One line at the top of a list: why it holds less than it might.
pub(super) fn list_note(text: impl Into<SharedString>, color: Hsla) -> Div {
    div()
        .px_2()
        .py_1()
        .text_xs()
        .text_color(color)
        .child(text.into())
}

/// "Diff too large to show (3.0 MiB)". `what` names the view in English, Diff, File or
/// Image, and picks its sentence.
pub(super) fn too_large(what: &str, bytes: u64) -> String {
    let size = file_size_text(bytes as usize);
    match what {
        "Diff" => t!("app.too_large.diff", size = size),
        "File" => t!("app.too_large.file", size = size),
        "Image" => t!("app.too_large.image", size = size),
        _ => t!("app.too_large.other", what = what, size = size),
    }
    .into_owned()
}

#[cfg(test)]
mod tests {
    #[test]
    fn too_large_states_the_size_like_the_image_preview() {
        assert_eq!(
            super::too_large("Diff", 3 << 20),
            "Diff too large to show (3.0 MiB)"
        );
        assert_eq!(
            super::too_large("File", 1536),
            "File too large to show (1.5 KiB)"
        );
    }
}
