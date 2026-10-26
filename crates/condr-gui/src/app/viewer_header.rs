//! The bar along the top of the Diff Tab and the Preview Tab. "Show File" and "Show
//! Changes" switch between the two, so both bars read the same way.

use super::change_tree::status_glyph;
use super::*;
use condr_core::GitChangeStatus;
use gpui_kit::component::button::ButtonGroup;

/// The bar: the file's type icon, its path cut short to fit and, when it changed, the
/// Changes view's glyph for that. The caller adds what trails them.
pub(super) fn viewer_header(
    selector: &'static str,
    path: &RelativePath,
    status: Option<GitChangeStatus>,
    cx: &App,
) -> Div {
    let theme = cx.theme();
    let icon = file_icons::file_icon_for_path(std::path::Path::new(path.as_str()), theme.is_dark());
    h_flex()
        .debug_selector(move || selector.into())
        .flex_none()
        .h_9()
        .w_full()
        .px_3()
        .gap_2()
        .items_center()
        .border_b_1()
        .border_color(theme.border)
        .child(img(icon).size_4().flex_shrink_0())
        .child(
            div()
                .min_w_0()
                .flex_1()
                .truncate()
                .text_sm()
                .font_medium()
                .child(path.to_string()),
        )
        .children(status.map(|status| status_glyph(status, cx)))
}

/// A short fact in the bar, muted unless it warns.
pub(super) fn header_meta(
    selector: &'static str,
    text: impl Into<SharedString>,
    color: Hsla,
) -> Div {
    div()
        .debug_selector(move || selector.into())
        .flex_none()
        .text_xs()
        .text_color(color)
        .child(text.into())
}

/// Two ways to show the same file; `on_change` hears whether the second is chosen.
pub(super) fn mode_toggle(
    id: &'static str,
    first: Button,
    second: Button,
    second_selected: bool,
    on_change: impl Fn(bool, &mut App) + 'static,
) -> ButtonGroup {
    ButtonGroup::new(id)
        .ghost()
        .xsmall()
        .child(first.selected(!second_selected))
        .child(second.selected(second_selected))
        .on_click(move |selected, _, cx| on_change(selected.first() == Some(&1), cx))
}
