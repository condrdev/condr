use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum SidebarIconTone {
    Muted,
    Success,
    Warning,
    Danger,
}

impl SidebarIconTone {
    pub(in crate::app) fn color(self, cx: &App) -> Hsla {
        match self {
            Self::Muted => cx.theme().muted_foreground,
            Self::Success => cx.theme().success,
            Self::Warning => cx.theme().warning,
            Self::Danger => cx.theme().danger,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum CondrIconName {
    Circle,
    /// Lucide `circle` with a `currentColor` fill.
    CircleFilled,
    CircleAlert,
    /// Lucide's `server` with a plus in the corner; Lucide itself has no server-plus.
    ServerPlus,
    /// Lucide `maximize-2` / `minimize-2`: a Pane's zoom toggle.
    Maximize2,
    Minimize2,
    /// The agent CLIs' marks (assets/icons/NOTICE-AGENT-ICONS), recolored to
    /// `currentColor` so they follow the text like every other icon.
    Claude,
    Codex,
    OpenCode,
}

impl CondrIconName {
    pub(in crate::app) fn agent(kind: AgentKind) -> Self {
        match kind {
            AgentKind::Claude => Self::Claude,
            AgentKind::Codex => Self::Codex,
            AgentKind::OpenCode => Self::OpenCode,
        }
    }
}

/// An agent's mark in its brand color where the brand has one; Codex and OpenCode
/// publish monochrome marks, which take `fallback`, the surrounding text color.
pub(in crate::app) fn agent_mark(kind: AgentKind, fallback: Hsla) -> Icon {
    let color = match kind {
        // Anthropic's terracotta, as the published mark carries it.
        AgentKind::Claude => rgb(0xD97757).into(),
        AgentKind::Codex | AgentKind::OpenCode => fallback,
    };
    Icon::new(CondrIconName::agent(kind)).text_color(color)
}

impl IconNamed for CondrIconName {
    fn path(self) -> SharedString {
        match self {
            Self::Circle => "icons/circle.svg",
            Self::CircleFilled => "icons/circle-filled.svg",
            Self::CircleAlert => "icons/circle-alert.svg",
            Self::ServerPlus => "icons/server-plus.svg",
            Self::Maximize2 => "icons/maximize-2.svg",
            Self::Minimize2 => "icons/minimize-2.svg",
            Self::Claude => "icons/claude.svg",
            Self::Codex => "icons/codex.svg",
            Self::OpenCode => "icons/opencode.svg",
        }
        .into()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum SidebarGlyph {
    Info,
    Circle,
    CircleFilled,
    CircleAlert,
}

impl SidebarGlyph {
    pub(in crate::app) fn icon(self) -> Icon {
        match self {
            Self::Info => Icon::new(IconName::Info),
            Self::Circle => Icon::new(CondrIconName::Circle),
            Self::CircleFilled => Icon::new(CondrIconName::CircleFilled),
            Self::CircleAlert => Icon::new(CondrIconName::CircleAlert),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) struct SidebarStatusVisual {
    pub(in crate::app) glyph: SidebarGlyph,
    pub(in crate::app) tone: SidebarIconTone,
    pub(in crate::app) key: &'static str,
    pub(in crate::app) label: &'static str,
}

/// Overrides the agent state while a BEL from this Pane is unseen.
pub(in crate::app) const BELL_SIDEBAR_STATUS: SidebarStatusVisual = SidebarStatusVisual {
    glyph: SidebarGlyph::CircleAlert,
    tone: SidebarIconTone::Warning,
    key: "bell",
    label: "Bell",
};

pub(in crate::app) fn agent_sidebar_status(state: AgentDisplayState) -> SidebarStatusVisual {
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
pub(in crate::app) fn identity_color(key: &str) -> Hsla {
    use std::hash::{Hash as _, Hasher as _};
    let mut hasher = std::hash::DefaultHasher::new();
    key.hash(&mut hasher);
    rgb(IDENTITY_COLORS[hasher.finish() as usize % IDENTITY_COLORS.len()]).into()
}

/// The letter on a Workspace avatar: the first character of the name, upper-cased.
pub(in crate::app) fn avatar_initial(name: &str) -> SharedString {
    name.trim()
        .chars()
        .next()
        .map(|character| character.to_uppercase().collect::<String>())
        .unwrap_or_default()
        .into()
}

#[derive(Clone)]
enum SidebarIconGraphic {
    /// A colored square with an initial.
    Avatar { initial: SharedString, color: Hsla },
    /// An agent's mark with its status glyph tucked into the corner, so a row of
    /// agents reads by agent first and by state second.
    Agent {
        kind: AgentKind,
        glyph: SidebarGlyph,
        tone: SidebarIconTone,
    },
}

#[derive(Clone)]
pub(in crate::app) struct CondrSidebarIcon {
    graphic: SidebarIconGraphic,
    pub(super) selector: SharedString,
    pub(super) tooltip: Option<SharedString>,
}

impl CondrSidebarIcon {
    /// `key` picks the color and should outlive renames (a Workspace's root path).
    pub(in crate::app) fn avatar(name: &str, key: &str, selector: impl Into<SharedString>) -> Self {
        Self {
            graphic: SidebarIconGraphic::Avatar {
                initial: avatar_initial(name),
                color: identity_color(key),
            },
            selector: selector.into(),
            tooltip: None,
        }
    }

    pub(in crate::app) fn agent(
        kind: AgentKind,
        visual: SidebarStatusVisual,
        selector: impl Into<SharedString>,
        tooltip: impl Into<SharedString>,
    ) -> Self {
        Self {
            graphic: SidebarIconGraphic::Agent {
                kind,
                glyph: visual.glyph,
                tone: visual.tone,
            },
            selector: selector.into(),
            tooltip: Some(tooltip.into()),
        }
    }

    pub(in crate::app) fn render(self, cx: &mut App) -> AnyElement {
        let graphic = match self.graphic {
            SidebarIconGraphic::Avatar { initial, color } => div()
                .size_4()
                .rounded_sm()
                .bg(color)
                .flex()
                .items_center()
                .justify_center()
                .text_xs()
                .font_medium()
                .text_color(gpui_kit::white())
                .child(initial)
                .into_any_element(),
            SidebarIconGraphic::Agent { kind, glyph, tone } => {
                // Past the mark's corner, on a disc of the sidebar's own color so it
                // reads over any mark. At this size a glyph is a smudge, so the state
                // is a plain dot in the tone's color; an unknown state shows nothing,
                // and only the bell keeps its alert glyph, which a dot could not say.
                let badge = match glyph {
                    SidebarGlyph::Info => None,
                    SidebarGlyph::CircleAlert => Some(
                        glyph
                            .icon()
                            .size_2()
                            .text_color(tone.color(cx))
                            .into_any_element(),
                    ),
                    _ => Some(
                        div()
                            .size(px(6.))
                            .rounded_full()
                            .bg(tone.color(cx))
                            .into_any_element(),
                    ),
                };
                div()
                    .relative()
                    .size_4()
                    .child(agent_mark(kind, cx.theme().sidebar_foreground).size_4())
                    .when_some(badge, |this, badge| {
                        this.child(
                            div()
                                .absolute()
                                .bottom(px(-2.))
                                .right(px(-2.))
                                .rounded_full()
                                .bg(cx.theme().sidebar)
                                .p(px(1.))
                                .child(badge),
                        )
                    })
                    .into_any_element()
            }
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
