use super::*;

impl Condr {
    /// Records the pointer's half over `target` (`half` is `None` once it left that
    /// item; only the current target clears itself then).
    pub(in crate::app) fn set_drop_target(
        &mut self,
        target: DropTarget,
        half: Option<bool>,
        cx: &mut Context<Self>,
    ) {
        let next = match half {
            Some(after) => Some(target.with_after(after)),
            None if self.drop_target.map(|current| current.with_after(false))
                == Some(target.with_after(false)) =>
            {
                None
            }
            None => return,
        };
        if self.drop_target != next {
            self.drop_target = next;
            cx.notify();
        }
    }

    /// The side recorded for `target` when the drop happens, clearing the indicator.
    pub(in crate::app) fn take_drop_after(&mut self, target: DropTarget) -> bool {
        self.drop_target
            .take()
            .filter(|current| current.with_after(false) == target.with_after(false))
            .is_some_and(DropTarget::after)
    }

    /// Which side of `target` the indicator shows, if it is the current target and a
    /// drag is still in progress.
    pub(in crate::app) fn drop_indicator_for(&self, target: DropTarget, cx: &App) -> Option<bool> {
        self.drop_target
            .filter(|_| cx.has_active_drag())
            .filter(|current| current.with_after(false) == target.with_after(false))
            .map(DropTarget::after)
    }
}

/// Where a drag would land if released now: the item under the pointer and which side
/// of it. The item gets the theme's drop tint and a 2px line on that side, as Zed does.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::app) enum DropTarget {
    Tab {
        key: ConnectionKey,
        tab_id: TabId,
        after: bool,
    },
    Workspace {
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        after: bool,
    },
    Server {
        key: ConnectionKey,
        after: bool,
    },
}

impl DropTarget {
    fn after(self) -> bool {
        match self {
            Self::Tab { after, .. }
            | Self::Workspace { after, .. }
            | Self::Server { after, .. } => after,
        }
    }

    fn with_after(self, after: bool) -> Self {
        match self {
            Self::Tab { key, tab_id, .. } => Self::Tab { key, tab_id, after },
            Self::Workspace {
                key, workspace_id, ..
            } => Self::Workspace {
                key,
                workspace_id,
                after,
            },
            Self::Server { key, .. } => Self::Server { key, after },
        }
    }
}

/// Which half of `bounds` the pointer is in (`true` = right or bottom), `None` outside.
pub(in crate::app) fn drop_half(
    position: Point<Pixels>,
    bounds: Bounds<Pixels>,
    horizontal: bool,
) -> Option<bool> {
    bounds.contains(&position).then(|| {
        if horizontal {
            position.x >= bounds.center().x
        } else {
            position.y >= bounds.center().y
        }
    })
}

/// The index a dragged item (at `source`) moves to when dropped on the near or far side
/// of the item at `target`, in the list after the source is removed. `None` when the
/// drop leaves the order as it is.
pub(in crate::app) fn drop_index(source: usize, target: usize, after: bool) -> Option<usize> {
    let mut index = target + usize::from(after);
    if source < index {
        index -= 1;
    }
    (index != source).then_some(index)
}

/// The insertion line, positioned on the edge of a `relative()` parent.
pub(super) fn drop_indicator(horizontal: bool, after: bool, cx: &App) -> Div {
    let line = div().absolute().rounded_full().bg(cx.theme().primary);
    match (horizontal, after) {
        (true, false) => line.top_0().bottom_0().w(px(2.)).left(px(-1.)),
        (true, true) => line.top_0().bottom_0().w(px(2.)).right(px(-1.)),
        (false, false) => line.left_0().right_0().h(px(2.)).top(px(-1.)),
        (false, true) => line.left_0().right_0().h(px(2.)).bottom(px(-1.)),
    }
}

/// Makes `element` a drop target for drags of type `D`: it reports which half the
/// pointer is over while a drag moves (`None` once it leaves) and draws the insertion
/// line when `indicator` says it is the current target.
pub(in crate::app) fn attach_drop_target<
    D: 'static,
    E: InteractiveElement + ParentElement + Styled,
>(
    element: E,
    horizontal: bool,
    indicator: Option<bool>,
    on_move: impl Fn(Option<bool>, &mut App) + 'static,
    cx: &App,
) -> E {
    let element = element
        .relative()
        // The hovered item takes the theme's drop tint; the line says which side.
        .drag_over::<D>(|style, _, _, cx| style.bg(cx.theme().tokens.drop_target))
        .on_drag_move::<D>(move |event, _, cx| {
            on_move(
                drop_half(event.event.position, event.bounds, horizontal),
                cx,
            )
        });
    match indicator {
        Some(after) => element.child(drop_indicator(horizontal, after, cx)),
        None => element,
    }
}

#[derive(Clone)]
pub(in crate::app) struct DraggedWorkspace {
    pub(in crate::app) key: ConnectionKey,
    pub(in crate::app) workspace_id: WorkspaceId,
    pub(super) name: SharedString,
}

#[derive(Clone)]
pub(in crate::app) struct DraggedServer {
    pub(in crate::app) key: ConnectionKey,
    pub(super) name: SharedString,
}

pub(in crate::app) struct DragPreview {
    pub(in crate::app) icon: Option<IconName>,
    pub(in crate::app) name: SharedString,
}

impl Render for DragPreview {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .px_2()
            .py_1()
            .gap_x_2()
            .rounded(cx.theme().radius)
            .bg(cx.theme().tokens.sidebar_accent)
            .text_sm()
            .text_color(cx.theme().sidebar_accent_foreground)
            .when_some(self.icon.clone(), |this, icon| {
                this.child(Icon::new(icon).size_4())
            })
            .child(self.name.clone())
    }
}
