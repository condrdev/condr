//! Condr's own icons beside Kit's Lucide set: the glyphs Lucide lacks or Condr draws
//! filled, the editors "Open in" names, and each Agent CLI's mark.

use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CondrIconName {
    Circle,
    /// Lucide `circle` with a `currentColor` fill.
    CircleFilled,
    /// Lucide's `server` with a plus in the corner; Lucide itself has no server-plus.
    ServerPlus,
    /// Lucide `folder-plus`: create a Workspace from a directory.
    FolderPlus,
    /// Lucide `arrow-left-right`: forwarded ports (ADR 0061).
    ArrowLeftRight,
    /// Lucide `git-branch`: clone a repository, and Git History in the right sidebar's strip.
    GitBranch,
    /// Lucide `maximize-2` / `minimize-2`: a Pane's zoom toggle.
    Maximize2,
    Minimize2,
    /// Lucide `coffee`: the keep-awake toggle, as caffeinate and its kin draw it.
    Coffee,
    /// Lucide `square-plus` / `square-dot` / `square-minus`: a change's status in the
    /// Changes view, as Zed's git panel draws added, modified and deleted.
    SquarePlus,
    SquareDot,
    SquareMinus,
    /// Lucide `square-kanban`: Tasks mode (ADR 0068).
    SquareKanban,
    /// Lucide `file-text`: the Diff Pane's "Show File" button.
    FileText,
    /// Lucide `qr-code`: the pairing card before its code shows.
    QrCode,
    /// An agent CLI's mark, the file its spec names (assets/agents/NOTICE at the
    /// repository root), recolored to `currentColor` so it follows the text like every
    /// other icon. A kind a newer Server reported (ADR 0028) is a plain mark, no brand.
    Agent(AgentKind),
    /// The editor marks "Open in" launches with (same notice), recolored the same way.
    VsCode,
    Zed,
    IntellijIdea,
}

/// The agent's mark in its published brand colour, or else the surrounding text colour.
pub(super) fn agent_mark(kind: AgentKind, fallback: Hsla) -> Icon {
    let color = kind
        .spec()
        .brand_color
        .map_or(fallback, |color| rgb(color).into());
    Icon::new(CondrIconName::Agent(kind)).text_color(color)
}

impl IconNamed for CondrIconName {
    fn path(self) -> SharedString {
        match self {
            Self::Circle => "icons/circle.svg",
            Self::CircleFilled => "icons/circle-filled.svg",
            Self::ServerPlus => "icons/server-plus.svg",
            Self::FolderPlus => "icons/folder-plus.svg",
            Self::ArrowLeftRight => "icons/arrow-left-right.svg",
            Self::GitBranch => "icons/git-branch.svg",
            Self::Maximize2 => "icons/maximize-2.svg",
            Self::Minimize2 => "icons/minimize-2.svg",
            Self::Coffee => "icons/coffee.svg",
            Self::SquarePlus => "icons/square-plus.svg",
            Self::SquareDot => "icons/square-dot.svg",
            Self::SquareKanban => "icons/square-kanban.svg",
            Self::SquareMinus => "icons/square-minus.svg",
            Self::FileText => "icons/file-text.svg",
            Self::QrCode => "icons/qr-code.svg",
            Self::Agent(kind) => match kind.spec().mark {
                "" => "icons/circle-filled.svg",
                mark => return format!("icons/{mark}").into(),
            },
            Self::VsCode => "icons/vscode.svg",
            Self::Zed => "icons/zed.svg",
            Self::IntellijIdea => "icons/intellij.svg",
        }
        .into()
    }
}
