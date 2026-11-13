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

    /// Moves `dragged` to the slot the insertion line shows, wherever over its Device's
    /// list it is let go.
    pub(in crate::app) fn drop_workspace(&mut self, dragged: &DraggedWorkspace) {
        let Some(DropTarget::Workspace {
            key,
            workspace_id,
            after,
        }) = self.drop_target.take()
        else {
            return;
        };
        if dragged.key != key {
            return;
        }
        let Some(session) = self
            .connection(key)
            .and_then(|connection| connection.presented_session())
        else {
            return;
        };
        let place = |id| session.workspaces().iter().position(|w| w.id() == id);
        let (Some(source), Some(target)) = (place(dragged.workspace_id), place(workspace_id))
        else {
            return;
        };
        if let Some(index) = drop_index(source, target, after) {
            self.send_layout_to(
                key,
                LayoutCommand::MoveWorkspace {
                    workspace_id: dragged.workspace_id,
                    target_index: index as u32,
                },
            );
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
/// The item also claims half the `gap` to its neighbours, so no point between two items
/// is neither's.
pub(in crate::app) fn drop_half(
    position: Point<Pixels>,
    bounds: Bounds<Pixels>,
    horizontal: bool,
    gap: Pixels,
) -> Option<bool> {
    let half = gap / 2.;
    let reach = if horizontal {
        Edges {
            left: half,
            right: half,
            ..Edges::default()
        }
    } else {
        Edges {
            top: half,
            bottom: half,
            ..Edges::default()
        }
    };
    bounds.extend(reach).contains(&position).then(|| {
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

/// The insertion line of a `relative()` parent, centred in the `gap` past the edge, so
/// the slot between two items draws one line whichever of them the pointer is over.
pub(super) fn drop_indicator(horizontal: bool, after: bool, gap: Pixels, cx: &App) -> Div {
    let line = div()
        .debug_selector(|| "drop-indicator".into())
        .absolute()
        .rounded_full()
        .bg(cx.theme().primary);
    let edge = -(gap / 2.) - px(1.);
    match (horizontal, after) {
        (true, false) => line.top_0().bottom_0().w(px(2.)).left(edge),
        (true, true) => line.top_0().bottom_0().w(px(2.)).right(edge),
        (false, false) => line.left_0().right_0().h(px(2.)).top(edge),
        (false, true) => line.left_0().right_0().h(px(2.)).bottom(edge),
    }
}

/// Makes `element` a drop target for drags of type `D`: it reports which half the
/// pointer is over while a drag moves (`None` once it leaves) and draws the insertion
/// line when `indicator` says it is the current target. `gap` is the space between it and
/// its neighbours; a drop let go there lands on the list around them, which takes the
/// target recorded here.
pub(in crate::app) fn attach_drop_target<
    D: 'static,
    E: InteractiveElement + ParentElement + Styled,
>(
    element: E,
    horizontal: bool,
    gap: Pixels,
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
                drop_half(event.event.position, event.bounds, horizontal, gap),
                cx,
            )
        });
    match indicator {
        Some(after) => element.child(drop_indicator(horizontal, after, gap, cx)),
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
    /// Where the drag started inside its source, as GPUI hands it to `on_drag`.
    pub(in crate::app) offset: Point<Pixels>,
}

impl Render for DragPreview {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // GPUI places the preview where its source was grabbed, which leaves a small chip
        // far from the pointer when the source is wide, such as a Pane's header. Padding
        // by that offset puts a zero-size point at the pointer, and the chip centres on it,
        // held under the pointer.
        div().pl(self.offset.x).pt(self.offset.y).child(
            h_flex()
                .size_0()
                .justify_center()
                .items_center()
                .child(self.chip(cx)),
        )
    }
}

impl DragPreview {
    fn chip(&self, cx: &App) -> impl IntoElement {
        h_flex()
            .debug_selector(|| "drag-preview".into())
            .flex_none()
            .whitespace_nowrap()
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
