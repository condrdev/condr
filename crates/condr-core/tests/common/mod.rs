//! Session Snapshots built field by field, including ones `Session::snapshot` could never
//! produce, encoded as the protobuf `SessionSnapshot` of `proto/condr/v1/session.proto`.

#![allow(dead_code, reason = "each test file uses a different part")]

use std::path::PathBuf;

use condr_core::SplitDirection;
use prost::Message as _;

pub struct EncodedSession {
    pub workspaces: Vec<EncodedWorkspace>,
}

pub struct EncodedWorkspace {
    pub id: u64,
    pub name: String,
    pub root_directory: PathBuf,
    pub worktree: Option<EncodedWorktree>,
    pub tabs: Vec<EncodedTab>,
}

pub struct EncodedWorktree {
    pub parent_workspace_id: u64,
    pub parent_root_directory: PathBuf,
    pub managed: bool,
}

pub struct EncodedTab {
    pub id: u64,
    pub name: String,
    pub content: EncodedTabContent,
}

pub enum EncodedTabContent {
    Terminals {
        panes: Vec<EncodedPane>,
        focused_pane: u64,
        focus_history: Vec<u64>,
        layout: EncodedLayout,
    },
    Diff {
        path: PathBuf,
    },
}

pub struct EncodedPane {
    pub id: u64,
    pub cwd: Option<PathBuf>,
}

pub struct EncodedLayout {
    pub root: u32,
    pub nodes: Vec<EncodedLayoutNode>,
}

impl EncodedLayout {
    pub fn pane(pane_id: u64) -> Self {
        Self {
            root: 0,
            nodes: vec![EncodedLayoutNode::Pane(pane_id)],
        }
    }
}

pub enum EncodedLayoutNode {
    Pane(u64),
    Split {
        direction: SplitDirection,
        ratio: f32,
        first: u32,
        second: u32,
    },
}

impl EncodedLayoutNode {
    pub fn split(first: u32, second: u32) -> Self {
        Self::Split {
            direction: SplitDirection::Horizontal,
            ratio: 0.5,
            first,
            second,
        }
    }
}

pub fn encode(session: &EncodedSession) -> Vec<u8> {
    let text = |path: &PathBuf| path.to_str().unwrap().to_owned();
    wire::Session {
        workspaces: session
            .workspaces
            .iter()
            .map(|workspace| wire::Workspace {
                id: workspace.id,
                name: workspace.name.clone(),
                root_directory: text(&workspace.root_directory),
                worktree: workspace.worktree.as_ref().map(|worktree| wire::Worktree {
                    parent_workspace_id: worktree.parent_workspace_id,
                    parent_root_directory: text(&worktree.parent_root_directory),
                    managed: worktree.managed,
                }),
                tabs: workspace
                    .tabs
                    .iter()
                    .map(|tab| wire::Tab {
                        id: tab.id,
                        name: tab.name.clone(),
                        content: Some(match &tab.content {
                            EncodedTabContent::Terminals {
                                panes,
                                focused_pane,
                                focus_history,
                                layout,
                            } => wire::Content::Terminals(wire::Terminals {
                                panes: panes
                                    .iter()
                                    .map(|pane| wire::Pane {
                                        id: pane.id,
                                        cwd: pane.cwd.as_ref().map(text),
                                    })
                                    .collect(),
                                focused_pane: *focused_pane,
                                focus_history: focus_history.clone(),
                                layout: Some(wire::Layout {
                                    root: layout.root,
                                    nodes: layout
                                        .nodes
                                        .iter()
                                        .map(|node| wire::LayoutNode {
                                            node: Some(match node {
                                                EncodedLayoutNode::Pane(pane) => {
                                                    wire::Node::Pane(*pane)
                                                }
                                                EncodedLayoutNode::Split {
                                                    direction,
                                                    ratio,
                                                    first,
                                                    second,
                                                } => wire::Node::Split(wire::Split {
                                                    direction: match direction {
                                                        SplitDirection::Horizontal => 1,
                                                        SplitDirection::Vertical => 2,
                                                    },
                                                    ratio: *ratio,
                                                    first: *first,
                                                    second: *second,
                                                }),
                                            }),
                                        })
                                        .collect(),
                                }),
                            }),
                            EncodedTabContent::Diff { path } => {
                                wire::Content::Diff(wire::PathTab { path: text(path) })
                            }
                        }),
                    })
                    .collect(),
            })
            .collect(),
    }
    .encode_to_vec()
}

/// The field numbers of `session.proto`, only as far as these fixtures reach.
mod wire {
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Session {
        #[prost(message, repeated, tag = "1")]
        pub workspaces: Vec<Workspace>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Workspace {
        #[prost(uint64, tag = "1")]
        pub id: u64,
        #[prost(string, tag = "2")]
        pub name: String,
        #[prost(string, tag = "3")]
        pub root_directory: String,
        #[prost(message, optional, tag = "4")]
        pub worktree: Option<Worktree>,
        #[prost(message, repeated, tag = "5")]
        pub tabs: Vec<Tab>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Worktree {
        #[prost(uint64, tag = "1")]
        pub parent_workspace_id: u64,
        #[prost(string, tag = "2")]
        pub parent_root_directory: String,
        #[prost(bool, tag = "3")]
        pub managed: bool,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Tab {
        #[prost(uint64, tag = "1")]
        pub id: u64,
        #[prost(string, tag = "2")]
        pub name: String,
        #[prost(oneof = "Content", tags = "3, 4")]
        pub content: Option<Content>,
    }

    #[derive(Clone, PartialEq, prost::Oneof)]
    pub enum Content {
        #[prost(message, tag = "3")]
        Terminals(Terminals),
        #[prost(message, tag = "4")]
        Diff(PathTab),
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Terminals {
        #[prost(message, repeated, tag = "1")]
        pub panes: Vec<Pane>,
        #[prost(uint64, tag = "2")]
        pub focused_pane: u64,
        #[prost(uint64, repeated, tag = "3")]
        pub focus_history: Vec<u64>,
        #[prost(message, optional, tag = "4")]
        pub layout: Option<Layout>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct PathTab {
        #[prost(string, tag = "1")]
        pub path: String,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Pane {
        #[prost(uint64, tag = "1")]
        pub id: u64,
        #[prost(string, optional, tag = "2")]
        pub cwd: Option<String>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Layout {
        #[prost(uint32, tag = "1")]
        pub root: u32,
        #[prost(message, repeated, tag = "2")]
        pub nodes: Vec<LayoutNode>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct LayoutNode {
        #[prost(oneof = "Node", tags = "1, 2")]
        pub node: Option<Node>,
    }

    #[derive(Clone, PartialEq, prost::Oneof)]
    pub enum Node {
        #[prost(uint64, tag = "1")]
        Pane(u64),
        #[prost(message, tag = "2")]
        Split(Split),
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Split {
        #[prost(int32, tag = "1")]
        pub direction: i32,
        #[prost(float, tag = "2")]
        pub ratio: f32,
        #[prost(uint32, tag = "3")]
        pub first: u32,
        #[prost(uint32, tag = "4")]
        pub second: u32,
    }
}
