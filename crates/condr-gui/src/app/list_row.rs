//! A row of a list or tree as every sidebar and Tasks draw it: its indent and chevron, the
//! highlight of the row on screen, and the context menu one row at a time opens.

use super::*;
use gpui_kit::component::menu::ContextMenu;

/// A tree row's left padding: the section gutter plus one indent per level. A file row
/// also skips the chevron slot a directory row starts with, so names line up; both scale
/// with the font since they frame text.
pub(super) fn tree_indent(depth: usize, past_chevron: bool) -> Rems {
    let chevron = if past_chevron { 1.125 } else { 0. };
    rems(0.5 + 0.875 * depth as f32 + chevron)
}

/// What a tree row starts with: a directory's chevron, open or not, or a file's place
/// past it and whether it is the file shown.
#[derive(Clone, Copy)]
pub(super) enum RowKind {
    Directory { open: bool },
    File { selected: bool },
}

/// The shell every sidebar tree row shares, the Files tree's and the change trees': its
/// id, height, indent, highlight and, for a directory, the chevron. The caller adds the
/// icon, the name and what trails it.
pub(super) fn tree_row(id: SharedString, depth: usize, kind: RowKind, cx: &App) -> Stateful<Div> {
    let selector = id.clone();
    let (file, selected) = match kind {
        RowKind::Directory { .. } => (false, false),
        RowKind::File { selected } => (true, selected),
    };
    let row = h_flex()
        .id(id)
        .debug_selector(move || selector.to_string())
        .h_7()
        .w_full()
        .min_w_0()
        .pl(tree_indent(depth, file))
        .pr_2()
        .map(|row| if file { row.gap_2() } else { row.gap_1() })
        .items_center()
        .cursor_pointer()
        .rounded(cx.theme().radius)
        .text_sm();
    let row = row_highlight(row, selected, cx);
    match kind {
        RowKind::Directory { open } => row.child(disclosure(open, cx)),
        RowKind::File { .. } => row,
    }
}

/// The highlight of a sidebar row: the accent while selected, a softer one under the
/// pointer. Hover is always registered, in the selection's own colour on a selected row,
/// because one registered only sometimes can leave a stale hover style behind when the
/// row turns selected, as GPUI Kit's `ListItem` warns.
pub(super) fn row_highlight<E: Styled + InteractiveElement + FluentBuilder>(
    row: E,
    selected: bool,
    cx: &App,
) -> E {
    let theme = cx.theme();
    let (accent, soft, text) = (
        theme.tokens.sidebar_accent,
        theme.sidebar_accent.opacity(0.8),
        theme.sidebar_accent_foreground,
    );
    row.when(selected, |row| row.bg(accent).text_color(text))
        .hover(move |style| {
            let style = if selected {
                style.bg(accent)
            } else {
                style.bg(soft)
            };
            style.text_color(text)
        })
}

/// The row whose context menu is open, by its element id.
struct OpenRowMenu(Option<ElementId>);

impl Global for OpenRowMenu {}

/// `menu` as the row's context menu. The menu opens under the pointer and so takes the
/// hover off the row; while it is open the row keeps that highlight and gains a ring in
/// the focus colour, as VS Code and Zed mark the row a menu acts on.
pub(super) fn row_menu<E: ParentElement + Styled + InteractiveElement + FluentBuilder>(
    mut row: E,
    selected: bool,
    menu: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
    cx: &App,
) -> ContextMenu<E> {
    let id = row
        .interactivity()
        .element_id
        .clone()
        .expect("a row with a menu has an id");
    let open = cx
        .try_global::<OpenRowMenu>()
        .is_some_and(|OpenRowMenu(row)| row.as_ref() == Some(&id));
    let theme = cx.theme();
    let (soft, text, ring, radius) = (
        theme.sidebar_accent.opacity(0.8),
        theme.sidebar_accent_foreground,
        theme.ring,
        theme.radius,
    );
    row.relative()
        .when(open, |row| {
            row.when(!selected, |row| row.bg(soft))
                .text_color(text)
                // Drawn over the row rather than as its border, so nothing in it moves.
                .child(
                    div()
                        .debug_selector(|| "row-menu-ring".into())
                        .absolute()
                        .inset_0()
                        .rounded(radius)
                        .border_1()
                        .border_color(ring),
                )
        })
        .context_menu(move |popup, window, cx| {
            cx.set_global(OpenRowMenu(Some(id.clone())));
            let id = id.clone();
            cx.subscribe_self(move |_, _: &DismissEvent, cx| {
                let OpenRowMenu(row) = cx.global_mut::<OpenRowMenu>();
                // Another row's menu may have opened since.
                if row.as_ref() == Some(&id) {
                    *row = None;
                }
                cx.refresh_windows();
            })
            .detach();
            menu(popup, window, cx)
        })
}

/// A directory's chevron, pointing down while it is open.
pub(super) fn disclosure(open: bool, cx: &App) -> Icon {
    Icon::new(IconName::ChevronRight)
        .size_3p5()
        .text_color(cx.theme().muted_foreground)
        .when(open, |icon| icon.rotate(percentage(90. / 360.)))
}
