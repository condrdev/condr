use super::*;
use crate::app::change_tree::row_highlight;

pub(super) type SidebarClickHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

pub(super) type SidebarDropHandler = Rc<dyn Fn(&DraggedWorkspace, &mut Window, &mut App)>;

/// The space between a section's Workspaces, which a drop target there reaches into.
const ROW_GAP: Rems = rems(0.25);

pub(super) type SidebarServerDropHandler = Rc<dyn Fn(&DraggedServer, &mut Window, &mut App)>;

pub(super) type SidebarSuffixBuilder = Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>;

pub(super) type SidebarContextMenuBuilder =
    Rc<dyn Fn(PopupMenu, &mut Window, &mut App) -> PopupMenu>;

#[derive(Clone)]
pub(in crate::app) struct CondrSidebarTreeItem {
    pub(super) id: SharedString,
    pub(super) row_selector: SharedString,
    pub(super) label_selector: SharedString,
    pub(super) toggle_selector: Option<SharedString>,
    pub(super) label: SharedString,
    /// A muted second line under the label; the row grows to hold it.
    pub(super) detail: Option<SharedString>,
    pub(super) icon: Option<CondrSidebarIcon>,
    pub(super) handler: SidebarClickHandler,
    pub(super) active: bool,
    pub(super) open_state: Option<Entity<bool>>,
    pub(super) reserve_toggle_space: bool,
    pub(super) children: Vec<Self>,
    pub(super) suffix: Option<SidebarSuffixBuilder>,
    pub(super) disabled: bool,
    pub(super) context_menu: Option<SidebarContextMenuBuilder>,
    pub(super) drag: Option<DraggedWorkspace>,
    /// Which side the insertion line shows on, when this item is the drop target.
    pub(super) drop_indicator: Option<bool>,
    pub(super) on_drop_move: Option<SidebarDropMoveHandler>,
}

pub(in crate::app) type SidebarDropMoveHandler = Rc<dyn Fn(Option<bool>, &mut App)>;

/// Heights of the label and detail lines of a two-line row. A suffix that wants to sit
/// beside both lines stacks two boxes of these heights.
pub(in crate::app) const DETAIL_LINE_HEIGHTS: (Rems, Rems) = (rems(1.25), rems(1.));

impl FluentBuilder for CondrSidebarTreeItem {}

impl CondrSidebarTreeItem {
    pub(in crate::app) fn new(
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
            detail: None,
            icon: None,
            handler: Rc::new(|_, _, _| {}),
            active: false,
            open_state: None,
            reserve_toggle_space: false,
            children: Vec::new(),
            suffix: None,
            disabled: false,
            context_menu: None,
            drag: None,
            drop_indicator: None,
            on_drop_move: None,
        }
    }

    pub(in crate::app) fn draggable(mut self, drag: DraggedWorkspace) -> Self {
        self.drag = Some(drag);
        self
    }

    /// Shows the insertion line on `indicator`'s side and reports pointer halves; the
    /// section takes the drop (`CondrSidebarSection::on_workspace_drop`).
    pub(in crate::app) fn drop_target(
        mut self,
        indicator: Option<bool>,
        on_move: impl Fn(Option<bool>, &mut App) + 'static,
    ) -> Self {
        self.drop_indicator = indicator;
        self.on_drop_move = Some(Rc::new(on_move));
        self
    }

    pub(in crate::app) fn icon(mut self, icon: CondrSidebarIcon) -> Self {
        self.icon = Some(icon);
        self
    }

    pub(in crate::app) fn detail(mut self, detail: impl Into<SharedString>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub(in crate::app) fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    pub(super) fn subtree_active(&self) -> bool {
        self.active || self.children.iter().any(Self::subtree_active)
    }

    pub(in crate::app) fn tree_parent(
        mut self,
        toggle_selector: impl Into<SharedString>,
        open_state: Entity<bool>,
    ) -> Self {
        self.toggle_selector = Some(toggle_selector.into());
        self.open_state = Some(open_state);
        self.reserve_toggle_space = true;
        self
    }

    pub(in crate::app) fn children(mut self, children: impl IntoIterator<Item = Self>) -> Self {
        self.children = children.into_iter().collect();
        self
    }

    pub(in crate::app) fn suffix<F, E>(mut self, builder: F) -> Self
    where
        F: Fn(&mut Window, &mut App) -> E + 'static,
        E: IntoElement,
    {
        self.suffix = Some(Rc::new(move |window, cx| {
            builder(window, cx).into_any_element()
        }));
        self
    }

    pub(in crate::app) fn disable(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub(in crate::app) fn context_menu(
        mut self,
        builder: impl Fn(PopupMenu, &mut Window, &mut App) -> PopupMenu + 'static,
    ) -> Self {
        self.context_menu = Some(Rc::new(builder));
        self
    }

    pub(in crate::app) fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.handler = Rc::new(handler);
        self
    }

    pub(in crate::app) fn render(self, window: &mut Window, cx: &mut App) -> AnyElement {
        let Self {
            id,
            row_selector,
            label_selector,
            toggle_selector,
            label,
            detail,
            icon,
            handler,
            active,
            open_state,
            reserve_toggle_space,
            children,
            suffix,
            disabled,
            context_menu,
            drag,
            drop_indicator,
            on_drop_move,
        } = self;
        let selected = active || children.iter().any(Self::subtree_active);
        let is_submenu = !children.is_empty();
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
            .when(detail.is_none(), |this| this.h_8())
            .when(detail.is_some(), |this| this.h_11())
            .min_w_0()
            .overflow_x_hidden()
            .flex_shrink_0()
            .px_1()
            .gap_x_2()
            .rounded(cx.theme().radius)
            .text_sm()
            // A disabled row answers no pointer, so only its selection shows.
            .when(active || !disabled, |this| row_highlight(this, active, cx))
            .when(active, |this| this.font_medium())
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
            // With a detail line the two text lines take fixed heights so a two-line
            // suffix can line up with them: `DETAIL_LINE_HEIGHTS`.
            .child(
                v_flex()
                    .min_w_0()
                    .flex_1()
                    .justify_center()
                    .child(
                        div()
                            .debug_selector(move || label_debug_selector.to_string())
                            .when(detail.is_some(), |this| {
                                this.h(DETAIL_LINE_HEIGHTS.0)
                                    .line_height(DETAIL_LINE_HEIGHTS.0)
                            })
                            .min_w_0()
                            .w_full()
                            .truncate()
                            .child(label),
                    )
                    .when_some(detail, |this, detail| {
                        this.child(
                            div()
                                .h(DETAIL_LINE_HEIGHTS.1)
                                .line_height(DETAIL_LINE_HEIGHTS.1)
                                .min_w_0()
                                .w_full()
                                .truncate()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(detail),
                        )
                    }),
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
                // A click on a parent that is selected, itself or through a child, toggles
                // it, as a folder row does in a file tree; on any other parent it only
                // selects, so switching never folds.
                let toggle_state = open_state.clone().filter(|_| selected);
                this.on_click(move |event, window, cx| {
                    if let Some(open_state) = &toggle_state {
                        open_state.update(cx, |open, cx| {
                            *open = !*open;
                            cx.notify();
                        });
                    }
                    handler(event, window, cx)
                })
            })
            .when_some(drag, |this, drag| {
                let name = drag.name.clone();
                this.on_drag(drag, move |_, offset, _, cx| {
                    cx.stop_propagation();
                    cx.new(|_| DragPreview {
                        icon: Some(IconName::Folder),
                        name: name.clone(),
                        offset,
                    })
                })
            });
        let row = if let Some(context_menu) = context_menu {
            row.context_menu(move |menu, window, cx| context_menu(menu, window, cx))
                .into_any_element()
        } else {
            row.into_any_element()
        };

        let item = v_flex().w_full().child(row).when(show_children, |this| {
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
        });
        // The row and its open Agents are one target, so the slot after an open Workspace
        // and before the next one is the same line.
        match on_drop_move {
            Some(on_move) => attach_drop_target::<DraggedWorkspace, _>(
                item,
                false,
                ROW_GAP.to_pixels(window.rem_size()),
                drop_indicator,
                move |half, cx| on_move(half, cx),
                cx,
            )
            .into_any_element(),
            None => item.into_any_element(),
        }
    }
}

#[derive(Clone)]
pub(in crate::app) struct CondrSidebarSection {
    pub(super) label: SharedString,
    pub(super) heading_selector: SharedString,
    pub(super) action: SidebarSuffixBuilder,
    pub(super) items: Vec<CondrSidebarTreeItem>,
    pub(super) collapsed: bool,
    pub(super) active: bool,
    pub(super) on_click: Option<SidebarClickHandler>,
    pub(super) context_menu: Option<SidebarContextMenuBuilder>,
    pub(super) drag: Option<DraggedServer>,
    pub(super) on_drop: Option<SidebarServerDropHandler>,
    /// Takes a Workspace let go anywhere over the section, gaps between rows included.
    pub(super) on_workspace_drop: Option<SidebarDropHandler>,
    pub(super) drop_indicator: Option<bool>,
    pub(super) on_drop_move: Option<SidebarDropMoveHandler>,
    /// Whether the heading folds its items away, and the handler that flips it.
    pub(super) fold: Option<(bool, SharedString, SidebarClickHandler)>,
    /// Shown beside the label while folded: the hidden agents counted by state.
    pub(super) summary: Vec<(SidebarStatusVisual, usize)>,
    /// Shown before the label, where hovering the heading reveals the fold chevron.
    pub(super) status: Option<SidebarSuffixBuilder>,
}

impl CondrSidebarSection {
    pub(in crate::app) fn new(
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
            on_workspace_drop: None,
            drop_indicator: None,
            on_drop_move: None,
            fold: None,
            summary: Vec::new(),
            status: None,
        }
    }

    /// A chevron before the label folds the items away. With a status it shows while the
    /// heading is hovered, in the status's place, as a Workspace row's does over its icon.
    pub(in crate::app) fn foldable(
        mut self,
        folded: bool,
        selector: impl Into<SharedString>,
        on_toggle: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.fold = Some((folded, selector.into(), Rc::new(on_toggle)));
        self
    }

    pub(in crate::app) fn status(
        mut self,
        status: impl Fn(&mut Window, &mut App) -> AnyElement + 'static,
    ) -> Self {
        self.status = Some(Rc::new(status));
        self
    }

    pub(in crate::app) fn summary(mut self, summary: Vec<(SidebarStatusVisual, usize)>) -> Self {
        self.summary = summary;
        self
    }

    pub(in crate::app) fn draggable(mut self, drag: DraggedServer) -> Self {
        self.drag = Some(drag);
        self
    }

    /// Shows the insertion line above or below the whole section and reports halves.
    pub(in crate::app) fn drop_target(
        mut self,
        indicator: Option<bool>,
        on_move: impl Fn(Option<bool>, &mut App) + 'static,
    ) -> Self {
        self.drop_indicator = indicator;
        self.on_drop_move = Some(Rc::new(on_move));
        self
    }

    pub(in crate::app) fn on_drop(
        mut self,
        handler: impl Fn(&DraggedServer, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_drop = Some(Rc::new(handler));
        self
    }

    pub(in crate::app) fn on_workspace_drop(
        mut self,
        handler: impl Fn(&DraggedWorkspace, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_workspace_drop = Some(Rc::new(handler));
        self
    }

    pub(in crate::app) fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    pub(in crate::app) fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Rc::new(handler));
        self
    }

    pub(in crate::app) fn context_menu(
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
        let folded = self.fold.as_ref().is_some_and(|(folded, ..)| *folded);
        let rendered_items = if folded {
            Vec::new()
        } else {
            self.items
                .into_iter()
                .map(|item| item.render(window, cx))
                .collect::<Vec<_>>()
        };
        let label = self.label.clone();
        let chevron = self.fold.map(|(folded, selector, on_toggle)| {
            let debug_selector = selector.clone();
            Button::new(ElementId::Name(selector))
                .debug_selector(move || debug_selector.to_string())
                .xsmall()
                .ghost()
                .icon(
                    Icon::new(IconName::ChevronRight)
                        .size_3p5()
                        .when(!folded, |icon| icon.rotate(percentage(90. / 360.))),
                )
                .tooltip(if folded {
                    format!("Expand {label}")
                } else {
                    format!("Collapse {label}")
                })
                .on_click(move |event, window, cx| {
                    cx.stop_propagation();
                    on_toggle(event, window, cx);
                })
        });
        let group = self.heading_selector.clone();
        // The status and the chevron share one slot; hovering the heading swaps them.
        let slot = match (self.status.map(|status| status(window, cx)), chevron) {
            (Some(status), Some(chevron)) => Some(
                div()
                    .relative()
                    .flex_none()
                    .size_5()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .group_hover(group.clone(), |style| style.opacity(0.))
                            .child(status),
                    )
                    .child(
                        div()
                            .absolute()
                            .inset_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .opacity(0.)
                            .group_hover(group.clone(), |style| style.opacity(1.))
                            .child(chevron),
                    )
                    .into_any_element(),
            ),
            (status, chevron) => status.or(chevron.map(IntoElement::into_any_element)),
        };
        let label = div()
            .min_w_0()
            .truncate()
            .debug_selector(move || heading_debug_selector.to_string())
            .child(self.label);
        let summary = (folded && !self.summary.is_empty())
            .then(|| super::status_counts(&self.summary, cx).font_normal());

        let heading = h_flex()
            .id(self.heading_selector)
            .debug_selector(move || heading_row_debug_selector.to_string())
            .h_9()
            .w_full()
            .flex_shrink_0()
            .items_center()
            .justify_between()
            .px_1()
            .rounded(cx.theme().radius)
            // A Device heading stands apart from the Workspace rows' detail lines, which
            // share its size, by weight alone.
            .text_xs()
            .font_semibold()
            .text_color(cx.theme().sidebar_foreground.opacity(0.7))
            // Selected and hovered as a Workspace row is.
            .map(|this| row_highlight(this, self.active, cx))
            .group(group)
            .child(
                h_flex()
                    .min_w_0()
                    .items_center()
                    .gap_1()
                    .children(slot)
                    .child(label)
                    .children(summary),
            )
            .child((self.action)(window, cx))
            .when_some(self.on_click, |this, handler| {
                this.on_click(move |event, window, cx| handler(event, window, cx))
            })
            .when_some(self.drag, |this, drag| {
                let name = drag.name.clone();
                this.on_drag(drag, move |_, offset, _, cx| {
                    cx.stop_propagation();
                    cx.new(|_| DragPreview {
                        icon: None,
                        name: name.clone(),
                        offset,
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
            // Sections follow one another with no space between.
            Some(on_move) => attach_drop_target::<DraggedServer, _>(
                section,
                false,
                px(0.),
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
            .when_some(self.on_workspace_drop, |this, on_drop| {
                this.on_drop(move |dragged: &DraggedWorkspace, window, cx| {
                    on_drop(dragged, window, cx)
                })
            })
            .when(!self.collapsed, |this| {
                this.child(heading)
                    .child(v_flex().w_full().gap(ROW_GAP).children(rendered_items))
            })
    }
}
