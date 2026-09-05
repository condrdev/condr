use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SidebarIconTone {
    Muted,
    Success,
    Warning,
    Danger,
}

impl SidebarIconTone {
    pub(super) fn color(self, cx: &App) -> Hsla {
        match self {
            Self::Muted => cx.theme().muted_foreground,
            Self::Success => cx.theme().success,
            Self::Warning => cx.theme().warning,
            Self::Danger => cx.theme().danger,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CondrIconName {
    Circle,
    /// Lucide `circle` with a `currentColor` fill.
    CircleFilled,
    CircleAlert,
    /// Lucide's `server` with a plus in the corner; Lucide itself has no server-plus.
    ServerPlus,
}

impl IconNamed for CondrIconName {
    fn path(self) -> SharedString {
        match self {
            Self::Circle => "icons/circle.svg",
            Self::CircleFilled => "icons/circle-filled.svg",
            Self::CircleAlert => "icons/circle-alert.svg",
            Self::ServerPlus => "icons/server-plus.svg",
        }
        .into()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SidebarGlyph {
    Info,
    Circle,
    CircleFilled,
    CircleAlert,
}

impl SidebarGlyph {
    pub(super) fn icon(self) -> Icon {
        match self {
            Self::Info => Icon::new(IconName::Info),
            Self::Circle => Icon::new(CondrIconName::Circle),
            Self::CircleFilled => Icon::new(CondrIconName::CircleFilled),
            Self::CircleAlert => Icon::new(CondrIconName::CircleAlert),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct SidebarStatusVisual {
    pub(super) glyph: SidebarGlyph,
    pub(super) tone: SidebarIconTone,
    pub(super) key: &'static str,
    pub(super) label: &'static str,
}

/// Overrides the agent state while a BEL from this Pane is unseen.
pub(super) const BELL_SIDEBAR_STATUS: SidebarStatusVisual = SidebarStatusVisual {
    glyph: SidebarGlyph::CircleAlert,
    tone: SidebarIconTone::Warning,
    key: "bell",
    label: "Bell",
};

pub(super) fn agent_sidebar_status(state: AgentDisplayState) -> SidebarStatusVisual {
    match state {
        AgentDisplayState::Unknown => SidebarStatusVisual {
            glyph: SidebarGlyph::Info,
            tone: SidebarIconTone::Muted,
            key: "unknown",
            label: "Unknown",
        },
        AgentDisplayState::Idle => SidebarStatusVisual {
            glyph: SidebarGlyph::Circle,
            tone: SidebarIconTone::Muted,
            key: "idle",
            label: "Idle",
        },
        AgentDisplayState::Working => SidebarStatusVisual {
            glyph: SidebarGlyph::CircleFilled,
            tone: SidebarIconTone::Warning,
            key: "working",
            label: "Working",
        },
        AgentDisplayState::Blocked => SidebarStatusVisual {
            glyph: SidebarGlyph::CircleFilled,
            tone: SidebarIconTone::Danger,
            key: "blocked",
            label: "Blocked",
        },
        AgentDisplayState::Done => SidebarStatusVisual {
            glyph: SidebarGlyph::CircleFilled,
            tone: SidebarIconTone::Success,
            key: "done",
            label: "Done",
        },
    }
}

/// Identity palette: muted fills that all land in one contrast band under a
/// white letter, so a Workspace's color identifies it without ranking it. The order
/// is load-bearing: the hash indexes into it.
const IDENTITY_COLORS: [u32; 10] = [
    0x7a6aa8, // violet
    0x3d7ea6, // sky
    0x388068, // emerald
    0xa4673a, // orange
    0xb05c80, // pink
    0x6a70b8, // indigo
    0x368080, // teal
    0xb06260, // red
    0x8f7838, // amber
    0x5179b0, // blue
];

/// A stable color for `key`. `DefaultHasher::new()` is deterministic within one Rust
/// release; a toolchain upgrade may recolor Workspaces once, which is fine.
pub(super) fn identity_color(key: &str) -> Hsla {
    use std::hash::{Hash as _, Hasher as _};
    let mut hasher = std::hash::DefaultHasher::new();
    key.hash(&mut hasher);
    rgb(IDENTITY_COLORS[hasher.finish() as usize % IDENTITY_COLORS.len()]).into()
}

/// The letter on a Workspace avatar: the first character of the name, upper-cased.
pub(super) fn avatar_initial(name: &str) -> SharedString {
    name.trim()
        .chars()
        .next()
        .map(|character| character.to_uppercase().collect::<String>())
        .unwrap_or_default()
        .into()
}

#[derive(Clone)]
enum SidebarIconGraphic {
    Glyph(SidebarGlyph, SidebarIconTone),
    /// A colored square with an initial.
    Avatar {
        initial: SharedString,
        color: Hsla,
    },
}

#[derive(Clone)]
pub(super) struct CondrSidebarIcon {
    graphic: SidebarIconGraphic,
    selector: SharedString,
    tooltip: Option<SharedString>,
}

impl CondrSidebarIcon {
    /// `key` picks the color and should outlive renames (a Workspace's root path).
    pub(super) fn avatar(name: &str, key: &str, selector: impl Into<SharedString>) -> Self {
        Self {
            graphic: SidebarIconGraphic::Avatar {
                initial: avatar_initial(name),
                color: identity_color(key),
            },
            selector: selector.into(),
            tooltip: None,
        }
    }

    pub(super) fn status(
        visual: SidebarStatusVisual,
        selector: impl Into<SharedString>,
        tooltip: impl Into<SharedString>,
    ) -> Self {
        Self {
            graphic: SidebarIconGraphic::Glyph(visual.glyph, visual.tone),
            selector: selector.into(),
            tooltip: Some(tooltip.into()),
        }
    }

    pub(super) fn render(self, cx: &mut App) -> AnyElement {
        let graphic = match self.graphic {
            SidebarIconGraphic::Glyph(glyph, tone) => glyph
                .icon()
                .size_3()
                .text_color(tone.color(cx))
                .into_any_element(),
            SidebarIconGraphic::Avatar { initial, color } => div()
                .size_4()
                .rounded_sm()
                .bg(color)
                .flex()
                .items_center()
                .justify_center()
                .text_xs()
                .font_medium()
                .text_color(gpui::white())
                .child(initial)
                .into_any_element(),
        };
        let id = self.selector.clone();
        let debug_selector = self.selector;

        div()
            .id(id)
            .debug_selector(move || debug_selector.to_string())
            .size_4()
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .when_some(self.tooltip, |this, tooltip| {
                this.tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
            })
            .child(graphic)
            .into_any_element()
    }
}

/// Where a drag would land if released now: the item under the pointer and which side
/// of it. The item gets the theme's drop tint and a 2px line on that side, as Zed does.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DropTarget {
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
pub(super) fn drop_half(
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
pub(super) fn drop_index(source: usize, target: usize, after: bool) -> Option<usize> {
    let mut index = target + usize::from(after);
    if source < index {
        index -= 1;
    }
    (index != source).then_some(index)
}

/// The insertion line, positioned on the edge of a `relative()` parent.
fn drop_indicator(horizontal: bool, after: bool, cx: &App) -> Div {
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
pub(super) fn attach_drop_target<D: 'static, E: InteractiveElement + ParentElement + Styled>(
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

impl Condr {
    /// Records the pointer's half over `target` (`half` is `None` once it left that
    /// item; only the current target clears itself then).
    pub(super) fn set_drop_target(
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
    pub(super) fn take_drop_after(&mut self, target: DropTarget) -> bool {
        self.drop_target
            .take()
            .filter(|current| current.with_after(false) == target.with_after(false))
            .is_some_and(DropTarget::after)
    }

    /// Which side of `target` the indicator shows, if it is the current target and a
    /// drag is still in progress.
    pub(super) fn drop_indicator_for(&self, target: DropTarget, cx: &App) -> Option<bool> {
        self.drop_target
            .filter(|_| cx.has_active_drag())
            .filter(|current| current.with_after(false) == target.with_after(false))
            .map(DropTarget::after)
    }
}

#[derive(Clone)]
pub(super) struct DraggedWorkspace {
    pub(super) key: ConnectionKey,
    pub(super) workspace_id: WorkspaceId,
    name: SharedString,
}

#[derive(Clone)]
pub(super) struct DraggedServer {
    pub(super) key: ConnectionKey,
    name: SharedString,
}

pub(super) struct DragPreview {
    pub(super) icon: Option<IconName>,
    pub(super) name: SharedString,
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

type SidebarClickHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;
type SidebarDropHandler = Rc<dyn Fn(&DraggedWorkspace, &mut Window, &mut App)>;
type SidebarServerDropHandler = Rc<dyn Fn(&DraggedServer, &mut Window, &mut App)>;
type SidebarSuffixBuilder = Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>;
type SidebarContextMenuBuilder = Rc<dyn Fn(PopupMenu, &mut Window, &mut App) -> PopupMenu>;

#[derive(Clone)]
pub(super) struct CondrSidebarTreeItem {
    id: SharedString,
    row_selector: SharedString,
    label_selector: SharedString,
    toggle_selector: Option<SharedString>,
    label: SharedString,
    icon: Option<CondrSidebarIcon>,
    handler: SidebarClickHandler,
    active: bool,
    default_open: bool,
    reserve_toggle_space: bool,
    children: Vec<Self>,
    suffix: Option<SidebarSuffixBuilder>,
    disabled: bool,
    context_menu: Option<SidebarContextMenuBuilder>,
    drag: Option<DraggedWorkspace>,
    on_drop: Option<SidebarDropHandler>,
    /// Which side the insertion line shows on, when this item is the drop target.
    drop_indicator: Option<bool>,
    on_drop_move: Option<SidebarDropMoveHandler>,
}

pub(super) type SidebarDropMoveHandler = Rc<dyn Fn(Option<bool>, &mut App)>;

impl FluentBuilder for CondrSidebarTreeItem {}

impl CondrSidebarTreeItem {
    pub(super) fn new(
        id: impl Into<SharedString>,
        row_selector: impl Into<SharedString>,
        label_selector: impl Into<SharedString>,
        label: impl Into<SharedString>,
    ) -> Self {
        Self {
            id: id.into(),
            row_selector: row_selector.into(),
            label_selector: label_selector.into(),
            toggle_selector: None,
            label: label.into(),
            icon: None,
            handler: Rc::new(|_, _, _| {}),
            active: false,
            default_open: false,
            reserve_toggle_space: false,
            children: Vec::new(),
            suffix: None,
            disabled: false,
            context_menu: None,
            drag: None,
            on_drop: None,
            drop_indicator: None,
            on_drop_move: None,
        }
    }

    pub(super) fn draggable(mut self, drag: DraggedWorkspace) -> Self {
        self.drag = Some(drag);
        self
    }

    /// Shows the insertion line on `indicator`'s side and reports pointer halves.
    pub(super) fn drop_target(
        mut self,
        indicator: Option<bool>,
        on_move: impl Fn(Option<bool>, &mut App) + 'static,
    ) -> Self {
        self.drop_indicator = indicator;
        self.on_drop_move = Some(Rc::new(on_move));
        self
    }

    pub(super) fn on_drop(
        mut self,
        handler: impl Fn(&DraggedWorkspace, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_drop = Some(Rc::new(handler));
        self
    }

    pub(super) fn icon(mut self, icon: CondrSidebarIcon) -> Self {
        self.icon = Some(icon);
        self
    }

    pub(super) fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    fn subtree_active(&self) -> bool {
        self.active || self.children.iter().any(Self::subtree_active)
    }

    pub(super) fn tree_parent(mut self, toggle_selector: impl Into<SharedString>) -> Self {
        self.toggle_selector = Some(toggle_selector.into());
        self.reserve_toggle_space = true;
        self
    }

    pub(super) fn default_open(mut self, open: bool) -> Self {
        self.default_open = open;
        self
    }

    pub(super) fn children(mut self, children: impl IntoIterator<Item = Self>) -> Self {
        self.children = children.into_iter().collect();
        self
    }

    pub(super) fn suffix<F, E>(mut self, builder: F) -> Self
    where
        F: Fn(&mut Window, &mut App) -> E + 'static,
        E: IntoElement,
    {
        self.suffix = Some(Rc::new(move |window, cx| {
            builder(window, cx).into_any_element()
        }));
        self
    }

    pub(super) fn disable(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub(super) fn context_menu(
        mut self,
        builder: impl Fn(PopupMenu, &mut Window, &mut App) -> PopupMenu + 'static,
    ) -> Self {
        self.context_menu = Some(Rc::new(builder));
        self
    }

    pub(super) fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.handler = Rc::new(handler);
        self
    }

    pub(super) fn render(self, window: &mut Window, cx: &mut App) -> AnyElement {
        let Self {
            id,
            row_selector,
            label_selector,
            toggle_selector,
            label,
            icon,
            handler,
            active,
            default_open,
            reserve_toggle_space,
            children,
            suffix,
            disabled,
            context_menu,
            drag,
            on_drop,
            drop_indicator,
            on_drop_move,
        } = self;
        let is_submenu = !children.is_empty();
        let open_state = reserve_toggle_space.then(|| {
            window.use_keyed_state(
                SharedString::from(format!("condr-sidebar-open-{id}")),
                cx,
                |_, _| default_open,
            )
        });
        let is_open = open_state.as_ref().is_some_and(|state| *state.read(cx));
        let show_children = is_open && is_submenu;
        let rendered_children = if show_children {
            children
                .into_iter()
                .map(|child| child.render(window, cx))
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let row_debug_selector = row_selector.clone();
        let label_debug_selector = label_selector;
        let row = h_flex()
            .id(id.clone())
            .debug_selector(move || row_debug_selector.to_string())
            .w_full()
            .h_8()
            .min_w_0()
            .overflow_x_hidden()
            .flex_shrink_0()
            .px_1()
            .gap_x_2()
            .rounded(cx.theme().radius)
            .text_sm()
            .when(!active && !disabled, |this| {
                this.hover(|this| {
                    this.bg(cx.theme().sidebar_accent.opacity(0.8))
                        .text_color(cx.theme().sidebar_accent_foreground)
                })
            })
            .when(active, |this| {
                this.font_medium()
                    .bg(cx.theme().tokens.sidebar_accent)
                    .text_color(cx.theme().sidebar_accent_foreground)
            })
            // Hover reveals the disclosure chevron over the icon and the actions menu over
            // the trailing text; at rest the row is
            // just icon, name and detail.
            .group(id.clone())
            .when(reserve_toggle_space, |this| {
                let toggle_debug_selector = toggle_selector
                    .clone()
                    .unwrap_or_else(|| format!("{id}-toggle").into());
                let button = Button::new(format!("{id}-toggle"))
                    .debug_selector(move || toggle_debug_selector.to_string())
                    .xsmall()
                    .ghost()
                    .icon(
                        Icon::new(IconName::ChevronRight)
                            .size_3p5()
                            .when(is_open, |icon| icon.rotate(percentage(90. / 360.))),
                    );
                let toggle_tooltip = if is_open {
                    format!("Collapse {label}")
                } else {
                    format!("Expand {label}")
                };
                let open_state = open_state
                    .clone()
                    .expect("tree parents always own disclosure state");
                let toggle = button.tooltip(toggle_tooltip).on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    open_state.update(cx, |open, cx| {
                        *open = !*open;
                        cx.notify();
                    });
                });
                // The icon and the chevron share one slot; hovering the row swaps them.
                this.child(
                    div()
                        .relative()
                        .flex_none()
                        .size_5()
                        .flex()
                        .items_center()
                        .justify_center()
                        .when_some(icon.clone(), |this, icon| {
                            this.child(
                                div()
                                    .group_hover(id.clone(), |style| style.opacity(0.))
                                    .child(icon.render(cx)),
                            )
                        })
                        .child(
                            div()
                                .absolute()
                                .inset_0()
                                .flex()
                                .items_center()
                                .justify_center()
                                .when(icon.is_some(), |this| {
                                    this.opacity(0.)
                                        .group_hover(id.clone(), |style| style.opacity(1.))
                                })
                                .child(toggle),
                        ),
                )
            })
            .when_some(icon.filter(|_| !reserve_toggle_space), |this, icon| {
                this.child(icon.render(cx))
            })
            .child(
                div()
                    .debug_selector(move || label_debug_selector.to_string())
                    .min_w_0()
                    .flex_1()
                    .truncate()
                    .child(label),
            )
            .when(suffix.is_some() || context_menu.is_some(), |this| {
                let menu = context_menu.clone().map(|context_menu| {
                    Button::new(format!("{id}-menu"))
                        .debug_selector({
                            let id = id.clone();
                            move || format!("{id}-menu")
                        })
                        .xsmall()
                        .ghost()
                        .tab_stop(false)
                        .icon(IconName::EllipsisVertical)
                        .tooltip("Actions")
                        .accessibility_label("Actions")
                        .dropdown_menu(move |menu, window, cx| context_menu(menu, window, cx))
                });
                this.child(
                    h_flex()
                        .relative()
                        .flex_none()
                        .items_center()
                        .when_some(suffix, |this, suffix| {
                            this.child(
                                div()
                                    .when(menu.is_some(), |this| {
                                        this.group_hover(id.clone(), |style| style.opacity(0.))
                                    })
                                    .child(suffix(window, cx).into_any_element()),
                            )
                        })
                        .when_some(menu, |this, menu| {
                            this.child(
                                div()
                                    .absolute()
                                    .right_0()
                                    .top_0()
                                    .bottom_0()
                                    .flex()
                                    .items_center()
                                    .opacity(0.)
                                    .group_hover(id.clone(), |style| style.opacity(1.))
                                    .child(menu),
                            )
                        }),
                )
            })
            .when(disabled, |this| {
                this.text_color(cx.theme().muted_foreground)
            })
            .when(!disabled, |this| {
                // A double click on a parent toggles it; a single click keeps selecting.
                let toggle_state = open_state.clone();
                this.on_click(move |event, window, cx| {
                    if event.click_count() == 2
                        && let Some(open_state) = &toggle_state
                    {
                        open_state.update(cx, |open, cx| {
                            *open = !*open;
                            cx.notify();
                        });
                        return;
                    }
                    handler(event, window, cx)
                })
            })
            .when_some(drag, |this, drag| {
                let name = drag.name.clone();
                this.on_drag(drag, move |_, _, _, cx| {
                    cx.stop_propagation();
                    cx.new(|_| DragPreview {
                        icon: Some(IconName::Folder),
                        name: name.clone(),
                    })
                })
            })
            .when_some(on_drop, |this, on_drop| {
                this.on_drop(move |dragged: &DraggedWorkspace, window, cx| {
                    on_drop(dragged, window, cx)
                })
            });
        let row = match on_drop_move {
            Some(on_move) => attach_drop_target::<DraggedWorkspace, _>(
                row,
                false,
                drop_indicator,
                move |half, cx| on_move(half, cx),
                cx,
            ),
            None => row,
        };
        let row = if let Some(context_menu) = context_menu {
            row.context_menu(move |menu, window, cx| context_menu(menu, window, cx))
                .into_any_element()
        } else {
            row.into_any_element()
        };

        v_flex()
            .w_full()
            .child(row)
            .when(show_children, |this| {
                this.child(
                    v_flex()
                        .border_l_1()
                        .border_color(cx.theme().sidebar_border)
                        .gap_1()
                        .ml_3p5()
                        .pl_2p5()
                        .py_0p5()
                        .children(rendered_children),
                )
            })
            .into_any_element()
    }
}

#[derive(Clone)]
pub(super) struct CondrSidebarSection {
    label: SharedString,
    heading_selector: SharedString,
    action: SidebarSuffixBuilder,
    items: Vec<CondrSidebarTreeItem>,
    collapsed: bool,
    active: bool,
    on_click: Option<SidebarClickHandler>,
    context_menu: Option<SidebarContextMenuBuilder>,
    drag: Option<DraggedServer>,
    on_drop: Option<SidebarServerDropHandler>,
    drop_indicator: Option<bool>,
    on_drop_move: Option<SidebarDropMoveHandler>,
}

impl CondrSidebarSection {
    pub(super) fn new(
        label: impl Into<SharedString>,
        heading_selector: impl Into<SharedString>,
        action: impl Fn(&mut Window, &mut App) -> AnyElement + 'static,
        items: impl IntoIterator<Item = CondrSidebarTreeItem>,
    ) -> Self {
        Self {
            label: label.into(),
            heading_selector: heading_selector.into(),
            action: Rc::new(action),
            items: items.into_iter().collect(),
            collapsed: false,
            active: false,
            on_click: None,
            context_menu: None,
            drag: None,
            on_drop: None,
            drop_indicator: None,
            on_drop_move: None,
        }
    }

    pub(super) fn draggable(mut self, drag: DraggedServer) -> Self {
        self.drag = Some(drag);
        self
    }

    /// Shows the insertion line above or below the whole section and reports halves.
    pub(super) fn drop_target(
        mut self,
        indicator: Option<bool>,
        on_move: impl Fn(Option<bool>, &mut App) + 'static,
    ) -> Self {
        self.drop_indicator = indicator;
        self.on_drop_move = Some(Rc::new(on_move));
        self
    }

    pub(super) fn on_drop(
        mut self,
        handler: impl Fn(&DraggedServer, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_drop = Some(Rc::new(handler));
        self
    }

    pub(super) fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    pub(super) fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Rc::new(handler));
        self
    }

    pub(super) fn context_menu(
        mut self,
        builder: impl Fn(PopupMenu, &mut Window, &mut App) -> PopupMenu + 'static,
    ) -> Self {
        self.context_menu = Some(Rc::new(builder));
        self
    }
}

impl Collapsible for CondrSidebarSection {
    fn collapsed(mut self, collapsed: bool) -> Self {
        self.collapsed = collapsed;
        self
    }

    fn is_collapsed(&self) -> bool {
        self.collapsed
    }
}

impl SidebarItem for CondrSidebarSection {
    fn render(
        self,
        _id: impl Into<ElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> impl IntoElement {
        let heading_row_debug_selector =
            SharedString::from(format!("{}-row", self.heading_selector));
        let heading_debug_selector = self.heading_selector.clone();
        let rendered_items = self
            .items
            .into_iter()
            .map(|item| item.render(window, cx))
            .collect::<Vec<_>>();

        let heading = h_flex()
            .id(self.heading_selector)
            .debug_selector(move || heading_row_debug_selector.to_string())
            .h_9()
            .w_full()
            .flex_shrink_0()
            .items_center()
            .justify_between()
            .pl_1()
            .text_xs()
            .text_color(cx.theme().sidebar_foreground.opacity(0.7))
            .when(self.active, |this| {
                this.font_medium().text_color(cx.theme().sidebar_foreground)
            })
            .child(
                div()
                    .debug_selector(move || heading_debug_selector.to_string())
                    .child(self.label),
            )
            .child((self.action)(window, cx))
            .when_some(self.on_click, |this, handler| {
                this.on_click(move |event, window, cx| handler(event, window, cx))
            })
            .when_some(self.drag, |this, drag| {
                let name = drag.name.clone();
                this.on_drag(drag, move |_, _, _, cx| {
                    cx.stop_propagation();
                    cx.new(|_| DragPreview {
                        icon: None,
                        name: name.clone(),
                    })
                })
            });
        let heading = if let Some(context_menu) = self.context_menu {
            heading
                .context_menu(move |menu, window, cx| context_menu(menu, window, cx))
                .into_any_element()
        } else {
            heading.into_any_element()
        };

        // The whole section is the drop zone for another Server, so the line lands
        // above its heading or below its last Workspace.
        let section = v_flex().relative().pb_3();
        let section = match self.on_drop_move {
            Some(on_move) => attach_drop_target::<DraggedServer, _>(
                section,
                false,
                self.drop_indicator,
                move |half, cx| on_move(half, cx),
                cx,
            ),
            None => section,
        };
        section
            .when_some(self.on_drop, |this, on_drop| {
                this.on_drop(move |dragged: &DraggedServer, window, cx| {
                    on_drop(dragged, window, cx)
                })
            })
            .when(!self.collapsed, |this| {
                this.child(heading)
                    .child(v_flex().w_full().gap_1().children(rendered_items))
            })
    }
}

/// Moves the dragged connection before or after the target connection. Returns
/// whether the order changed.
pub(super) fn reorder_connection(
    connections: &mut Vec<ServerConnection>,
    dragged: ConnectionKey,
    target: ConnectionKey,
    after: bool,
) -> bool {
    let Some(from) = connections.iter().position(|c| c.key == dragged) else {
        return false;
    };
    let Some(to) = connections.iter().position(|c| c.key == target) else {
        return false;
    };
    let Some(to) = drop_index(from, to, after) else {
        return false;
    };
    let connection = connections.remove(from);
    connections.insert(to, connection);
    true
}

impl Condr {
    fn move_server(&mut self, dragged: ConnectionKey, target: ConnectionKey, after: bool) {
        if reorder_connection(&mut self.connections, dragged, target, after) {
            // ponytail: the saved order only covers TCP servers, so the local
            // server always loads first again after a restart.
            self.save_servers();
        }
    }

    pub(super) fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let owner = cx.weak_entity();
        let items = self.connections.iter().map(|connection| {
            let key = connection.key;
            let server_target = DropTarget::Server { key, after: false };
            let active_server = key == self.active_connection;
            let connected = connection.can_mutate();
            let workspaces = Session::restore(connection.snapshot.clone())
                .ok()
                .map(|session| {
                    let active_workspace = self.presented_workspace_id(key, &session);
                    // Drop handlers need the source index of the dragged Workspace.
                    let workspace_ids = Rc::new(
                        session
                            .workspaces()
                            .iter()
                            .map(|workspace| workspace.id())
                            .collect::<Vec<_>>(),
                    );
                    session
                        .workspaces()
                        .iter()
                        .enumerate()
                        .map(|(workspace_index, workspace)| {
                            let workspace_id = workspace.id();
                            let workspace_name = workspace.name().to_owned();
                            let git = connection.workspace_git.get(&workspace_id);
                            let branch = git.and_then(|git| git.branch.clone());
                            let supports_worktrees = git.is_some_and(|git| !git.linked_worktree)
                                && workspace.worktree().is_none();
                            let managed_worktree = workspace
                                .worktree()
                                .is_some_and(|association| association.is_managed());
                            let agents = workspace
                                .tabs()
                                .iter()
                                .flat_map(|tab| tab.panes())
                                .filter_map(|pane| {
                                    let pane_id = pane.id();
                                    let agent = connection.agents.get(&pane_id)?;
                                    let state = connection
                                        .agent_trackers
                                        .get(&pane_id)
                                        .map(|tracker| tracker.display_state())
                                        .unwrap_or_else(|| {
                                            AgentTracker::new(agent.state).display_state()
                                        });
                                    let status = if connection.controlling
                                        && connection.attention.contains(&pane_id)
                                    {
                                        BELL_SIDEBAR_STATUS
                                    } else {
                                        agent_sidebar_status(state)
                                    };
                                    let agent_label = agent.kind.label();
                                    let agent_owner = owner.clone();
                                    Some(
                                        CondrSidebarTreeItem::new(
                                            format!("sidebar-agent-{key}-{}", pane_id.as_u64()),
                                            format!("agent-{key}-{}", pane_id.as_u64()),
                                            format!("agent-label-{key}-{}", pane_id.as_u64()),
                                            agent_label,
                                        )
                                        .icon(CondrSidebarIcon::status(
                                            status,
                                            format!(
                                                "agent-status-{key}-{}-{}",
                                                pane_id.as_u64(),
                                                status.key
                                            ),
                                            format!("{agent_label}: {}", status.label),
                                        ))
                                        .active(
                                            active_server
                                                && self.target_pane == Some((key, pane_id)),
                                        )
                                        .disable(!connected)
                                        .on_click(
                                            move |_, window, cx| {
                                                let _ = agent_owner.update(cx, |this, cx| {
                                                    this.select_pane(key, pane_id, window, cx)
                                                });
                                            },
                                        ),
                                    )
                                })
                                .collect::<Vec<_>>();
                            let owner = owner.clone();
                            let menu_owner = owner.clone();
                            let agent_selected = agents.iter().any(|agent| agent.active);
                            CondrSidebarTreeItem::new(
                                format!("sidebar-workspace-{key}-{}", workspace_id.as_u64()),
                                format!("workspace-{key}-{}", workspace_id.as_u64()),
                                format!("workspace-label-{key}-{}", workspace_id.as_u64()),
                                workspace_name.clone(),
                            )
                            .icon(CondrSidebarIcon::avatar(
                                &workspace_name,
                                &workspace.root_directory().to_string_lossy(),
                                format!("workspace-icon-{key}-{}", workspace_id.as_u64()),
                            ))
                            .active(
                                active_server
                                    && active_workspace == Some(workspace_id)
                                    && !agent_selected,
                            )
                            .tree_parent(format!(
                                "workspace-toggle-{key}-{}",
                                workspace_id.as_u64()
                            ))
                            .default_open(active_server && active_workspace == Some(workspace_id))
                            .children(agents)
                            .disable(!connected)
                            .when(connected, |item| {
                                let drop_owner = owner.clone();
                                let move_owner = owner.clone();
                                let target = DropTarget::Workspace {
                                    key,
                                    workspace_id,
                                    after: false,
                                };
                                let workspace_ids = workspace_ids.clone();
                                item.draggable(DraggedWorkspace {
                                    key,
                                    workspace_id,
                                    name: workspace_name.clone().into(),
                                })
                                .drop_target(
                                    self.drop_indicator_for(target, cx),
                                    move |half, cx| {
                                        let _ = move_owner.update(cx, |this, cx| {
                                            this.set_drop_target(target, half, cx)
                                        });
                                    },
                                )
                                .on_drop(move |dragged, _, cx| {
                                    let _ = drop_owner.update(cx, |this, _| {
                                        let after = this.take_drop_after(target);
                                        if dragged.key != key {
                                            return;
                                        }
                                        let Some(source) = workspace_ids
                                            .iter()
                                            .position(|id| *id == dragged.workspace_id)
                                        else {
                                            return;
                                        };
                                        if let Some(index) =
                                            drop_index(source, workspace_index, after)
                                        {
                                            this.send_layout_to(
                                                key,
                                                LayoutCommand::MoveWorkspace {
                                                    workspace_id: dragged.workspace_id,
                                                    target_index: index as u32,
                                                },
                                            );
                                        }
                                    });
                                })
                            })
                            .context_menu(move |menu, _, _| {
                                let rename_owner = menu_owner.clone();
                                let create_owner = menu_owner.clone();
                                let open_owner = menu_owner.clone();
                                let remove_owner = menu_owner.clone();
                                let close_owner = menu_owner.clone();
                                let rename_name = workspace_name.clone();
                                let create_name = workspace_name.clone();
                                let open_name = workspace_name.clone();
                                let remove_name = workspace_name.clone();
                                let menu = menu.item(
                                    PopupMenuItem::new("Rename Workspace…")
                                        .disabled(!connected)
                                        .on_click(move |_, window, cx| {
                                            let name = rename_name.clone();
                                            let _ = rename_owner.update(cx, |this, cx| {
                                                this.prompt_rename_workspace_on(
                                                    key,
                                                    workspace_id,
                                                    name,
                                                    window,
                                                    cx,
                                                )
                                            });
                                        }),
                                );
                                let menu = if supports_worktrees {
                                    menu.separator()
                                        .item(
                                            PopupMenuItem::new("Create Worktree…")
                                                .disabled(!connected)
                                                .on_click(move |_, window, cx| {
                                                    let name = create_name.clone();
                                                    let _ = create_owner.update(cx, |this, cx| {
                                                        this.prompt_create_worktree_on(
                                                            key,
                                                            workspace_id,
                                                            name,
                                                            window,
                                                            cx,
                                                        )
                                                    });
                                                }),
                                        )
                                        .item(
                                            PopupMenuItem::new("Open Existing Worktree…")
                                                .disabled(!connected)
                                                .on_click(move |_, window, cx| {
                                                    let name = open_name.clone();
                                                    let _ = open_owner.update(cx, |this, cx| {
                                                        this.choose_worktree_directory_on(
                                                            key,
                                                            workspace_id,
                                                            name,
                                                            window,
                                                            cx,
                                                        )
                                                    });
                                                }),
                                        )
                                } else {
                                    menu
                                };
                                let menu = if managed_worktree {
                                    menu.separator().item(
                                        PopupMenuItem::new("Remove Worktree…")
                                            .disabled(!connected)
                                            .on_click(move |_, window, cx| {
                                                let name = remove_name.clone();
                                                let _ = remove_owner.update(cx, |this, cx| {
                                                    this.confirm_remove_worktree_on(
                                                        key,
                                                        workspace_id,
                                                        name,
                                                        window,
                                                        cx,
                                                    )
                                                });
                                            }),
                                    )
                                } else {
                                    menu
                                };
                                menu.separator().item(
                                    PopupMenuItem::new("Close Workspace")
                                        .disabled(!connected)
                                        .on_click(move |_, window, cx| {
                                            let _ = close_owner.update(cx, |this, cx| {
                                                this.close_workspace_id(
                                                    key,
                                                    workspace_id,
                                                    window,
                                                    cx,
                                                )
                                            });
                                        }),
                                )
                            })
                            .when_some(branch, |item, branch| {
                                item.suffix(move |_, cx| {
                                    div()
                                        .max_w(px(84.0))
                                        .truncate()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(branch.clone())
                                })
                            })
                            .on_click(move |_, window, cx| {
                                let _ = owner.update(cx, |this, cx| {
                                    this.select_workspace(key, workspace_id, window, cx)
                                });
                            })
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let select_owner = owner.clone();
            let new_workspace_owner = owner.clone();
            let server_menu_owner = owner.clone();
            let server_name = connection.label.clone();
            let status = connection.status;
            let is_local = connection.endpoint.as_local_path().is_some();
            let new_workspace_label = connection.label.clone();
            let descendant_selected = workspaces.iter().any(CondrSidebarTreeItem::subtree_active);
            CondrSidebarSection::new(
                connection.label.clone(),
                format!("server-heading-{key}"),
                move |_, _| {
                    let owner = new_workspace_owner.clone();
                    let tooltip = format!("New Workspace on {new_workspace_label}…");
                    Button::new(("new-workspace", key))
                        .debug_selector(move || format!("new-workspace-server-{key}"))
                        .ghost()
                        .xsmall()
                        .icon(IconName::Plus)
                        .tooltip(tooltip)
                        .disabled(!connected)
                        .on_click(move |_, window, cx| {
                            cx.stop_propagation();
                            let _ = owner.update(cx, |this, cx| {
                                this.choose_workspace_directory_on(key, window, cx)
                            });
                        })
                        .into_any_element()
                },
                workspaces,
            )
            .active(active_server && !descendant_selected)
            .draggable(DraggedServer {
                key,
                name: connection.label.clone().into(),
            })
            .drop_target(self.drop_indicator_for(server_target, cx), {
                let move_owner = owner.clone();
                move |half, cx| {
                    let _ = move_owner
                        .update(cx, |this, cx| this.set_drop_target(server_target, half, cx));
                }
            })
            .on_drop({
                let drop_owner = owner.clone();
                move |dragged, _, cx| {
                    let _ = drop_owner.update(cx, |this, cx| {
                        let after = this.take_drop_after(server_target);
                        this.move_server(dragged.key, key, after);
                        cx.notify();
                    });
                }
            })
            .context_menu(move |menu, _, _| {
                let edit_owner = server_menu_owner.clone();
                let connection_owner = server_menu_owner.clone();
                let delete_owner = server_menu_owner.clone();
                let delete_name = server_name.clone();
                let (connection_label, connection_disabled) = match status {
                    ConnectionStatus::Connected => ("Disconnect", false),
                    ConnectionStatus::Disconnected => ("Connect", false),
                    ConnectionStatus::Connecting => ("Connecting…", true),
                };
                // The Local Server is this machine's own: nothing to edit, nothing to
                // delete, only its connection to toggle.
                let menu = if is_local {
                    menu
                } else {
                    menu.item(PopupMenuItem::new("Edit").on_click(move |_, window, cx| {
                        let _ = edit_owner
                            .update(cx, |this, cx| this.prompt_edit_server_on(key, window, cx));
                    }))
                };
                let menu = menu.item(
                    PopupMenuItem::new(connection_label)
                        .disabled(connection_disabled)
                        .on_click(move |_, window, cx| {
                            let _ = connection_owner.update(cx, |this, cx| {
                                match status {
                                    ConnectionStatus::Connected => {
                                        if this.disconnect_server(key) {
                                            this.refresh_target_pane(this.active_connection);
                                            this.rebuild_dock(window, cx);
                                        }
                                    }
                                    ConnectionStatus::Disconnected => {
                                        if this.start_connect(key) {
                                            this.refresh_target_pane(this.active_connection);
                                            this.rebuild_dock(window, cx);
                                        }
                                    }
                                    ConnectionStatus::Connecting => {}
                                }
                                cx.notify();
                            });
                        }),
                );
                if is_local {
                    menu
                } else {
                    menu.separator()
                        .item(
                            PopupMenuItem::new("Delete Server").on_click(move |_, window, cx| {
                                let name = delete_name.clone();
                                let _ = delete_owner.update(cx, |this, cx| {
                                    this.confirm_delete_server_on(key, name, window, cx)
                                });
                            }),
                        )
                }
            })
            .on_click(move |_, window, cx| {
                let _ = select_owner.update(cx, |this, cx| this.select_server(key, window, cx));
            })
        });

        let add_owner = cx.weak_entity();
        let settings_owner = cx.weak_entity();
        Sidebar::new("condr-sidebar")
            .collapsible(SidebarCollapsible::None)
            .w_full()
            .children(items)
            // The app name lives in the title bar; the actions sit at the bottom right,
            // Add Server left of Settings. Reconnect is the `ReconnectServer`
            // action; it has no button here.
            .footer(
                h_flex()
                    .w_full()
                    .justify_end()
                    .gap_1()
                    .child(
                        Button::new("add-server")
                            .debug_selector(|| "add-server".into())
                            .ghost()
                            .small()
                            .icon(Icon::new(CondrIconName::ServerPlus))
                            .tooltip("Add Server")
                            .accessibility_label("Add Server")
                            .on_click(move |_, window, cx| {
                                let _ = add_owner
                                    .update(cx, |this, cx| this.prompt_add_server(window, cx));
                            }),
                    )
                    .child(
                        Button::new("open-settings")
                            .debug_selector(|| "open-settings".into())
                            .ghost()
                            .small()
                            .icon(IconName::Settings)
                            .tooltip("Settings…")
                            .accessibility_label("Settings…")
                            .on_click(move |_, window, cx| {
                                let _ = settings_owner
                                    .update(cx, |this, cx| this.open_settings(window, cx));
                            }),
                    ),
            )
    }
}
