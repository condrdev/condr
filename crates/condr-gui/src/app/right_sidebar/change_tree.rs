//! The directory tree Changes and History draw changed files in (ADR 0017, ADR 0046).
//! Rows here only draw: what a click does is the caller's.

use super::*;
use crate::app::git_marks::{diff_stat, status_glyph};
use crate::app::list_row::{RowKind, tree_row};
use condr_core::{GitChangeEntry, GitChangeStatus};
use gpui_kit::component::tag::Tag;

/// Changed files as a directory tree, the way Zed's git panel shows them: a
/// directory that holds nothing but one directory folds into it (`src/app`), files sort after
/// directories, and both sort by name. A repository's directory under the Root stays a row
/// of its own, its node; the Root's own repository, named by the empty path, is a node
/// first that holds what lies in no other (ADR 0073).
#[derive(Debug, PartialEq)]
enum ChangeNode<'a> {
    Directory {
        /// The folded segments joined with `/`.
        label: String,
        /// The directory's Root-relative path, the key of its collapsed state.
        path: RelativePathBuf,
        children: Vec<ChangeNode<'a>>,
    },
    File {
        name: String,
        entry: &'a GitChangeEntry,
    },
}

fn change_tree<'a>(
    entries: &[&'a GitChangeEntry],
    repositories: &[&RelativePath],
) -> Vec<ChangeNode<'a>> {
    let nested: Vec<&RelativePath> = repositories
        .iter()
        .copied()
        .filter(|directory| !directory.as_str().is_empty())
        .collect();
    if nested.len() < repositories.len() {
        let (theirs, own): (Vec<&GitChangeEntry>, Vec<&GitChangeEntry>) =
            entries.iter().partition(|entry| {
                nested
                    .iter()
                    .any(|directory| entry.path.starts_with(directory))
            });
        // Its label is the caller's: the Workspace's name.
        let own = (!own.is_empty()).then(|| ChangeNode::Directory {
            label: String::new(),
            path: RelativePathBuf::new(),
            children: change_tree(&own, &[]),
        });
        return own
            .into_iter()
            .chain(change_tree(&theirs, &nested))
            .collect();
    }
    #[derive(Default)]
    struct Builder<'a> {
        directories: std::collections::BTreeMap<String, Builder<'a>>,
        files: Vec<(String, &'a GitChangeEntry)>,
    }

    fn insert<'a>(builder: &mut Builder<'a>, components: &[String], entry: &'a GitChangeEntry) {
        match components {
            [] => {}
            [name] => builder.files.push((name.clone(), entry)),
            [directory, rest @ ..] => insert(
                builder.directories.entry(directory.clone()).or_default(),
                rest,
                entry,
            ),
        }
    }

    fn finish<'a>(
        builder: Builder<'a>,
        parent: &RelativePath,
        repositories: &[&RelativePath],
    ) -> Vec<ChangeNode<'a>> {
        let mut nodes = Vec::new();
        for (name, mut child) in builder.directories {
            let mut label = name.clone();
            let mut path = parent.join(&name);
            // Fold a chain of lone directories into one row.
            while !repositories.contains(&path.as_relative_path())
                && child.files.is_empty()
                && child.directories.len() == 1
            {
                let (next_name, next) = child.directories.pop_first().expect("one entry");
                label.push('/');
                label.push_str(&next_name);
                path.push(&next_name);
                child = next;
            }
            nodes.push(ChangeNode::Directory {
                label,
                children: finish(child, &path, repositories),
                path,
            });
        }
        let mut files = builder.files;
        files.sort_by(|(a, _), (b, _)| a.cmp(b));
        nodes.extend(
            files
                .into_iter()
                .map(|(name, entry)| ChangeNode::File { name, entry }),
        );
        nodes
    }

    let mut root = Builder::default();
    for entry in entries {
        let components: Vec<String> = entry
            .path
            .components()
            .map(|component| component.as_str().to_owned())
            .collect();
        insert(&mut root, &components, entry);
    }
    finish(root, RelativePath::new(""), repositories)
}

/// One visible row of a change tree, in display order.
pub(in crate::app) enum TreeRow<'a> {
    Directory {
        label: String,
        /// The directory's Root-relative path, the key of its folded state.
        path: RelativePathBuf,
        depth: usize,
        folded: bool,
    },
    File {
        name: String,
        entry: &'a GitChangeEntry,
        depth: usize,
    },
}

/// The rows `entries` show as a tree, `repositories` as nodes of their own, leaving out what
/// the directories `folded` names hold.
pub(in crate::app) fn tree_rows<'a>(
    entries: &[&'a GitChangeEntry],
    repositories: &[&RelativePath],
    folded: impl Fn(&RelativePath) -> bool,
) -> Vec<TreeRow<'a>> {
    fn walk<'a>(
        nodes: Vec<ChangeNode<'a>>,
        depth: usize,
        folded: &dyn Fn(&RelativePath) -> bool,
        rows: &mut Vec<TreeRow<'a>>,
    ) {
        for node in nodes {
            match node {
                ChangeNode::Directory {
                    label,
                    path,
                    children,
                } => {
                    let is_folded = folded(&path);
                    rows.push(TreeRow::Directory {
                        label,
                        path,
                        depth,
                        folded: is_folded,
                    });
                    if !is_folded {
                        walk(children, depth + 1, folded, rows);
                    }
                }
                ChangeNode::File { name, entry } => {
                    rows.push(TreeRow::File { name, entry, depth });
                }
            }
        }
    }
    let mut rows = Vec::new();
    walk(change_tree(entries, repositories), 0, &folded, &mut rows);
    rows
}

/// A directory row: a chevron, a folder and the label.
pub(in crate::app) fn directory_row(
    id: String,
    label: String,
    depth: usize,
    folded: bool,
    cx: &App,
) -> Stateful<Div> {
    tree_row(id.into(), depth, RowKind::Directory { open: !folded }, cx)
        .child(
            Icon::new(IconName::Folder)
                .size_4()
                .text_color(cx.theme().muted_foreground),
        )
        .child(div().min_w_0().flex_1().truncate().child(label))
}

/// A repository's node in Changes (ADR 0073): a directory row that also says how many files
/// it holds there and, as IntelliJ does, its branch as a tag.
pub(in crate::app) fn repository_row(
    id: String,
    label: String,
    folded: bool,
    count: usize,
    branch: Option<&str>,
    cx: &App,
) -> Stateful<Div> {
    let files = if count == 1 {
        t!("changes.repository_files.one", count = count)
    } else {
        t!("changes.repository_files.other", count = count)
    };
    let branch = branch.map_or_else(|| t!("sidebar.detached"), |branch| branch.to_owned().into());
    directory_row(id, label, 0, folded, cx)
        .child(
            div()
                .flex_none()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(files),
        )
        .child(
            Tag::secondary()
                .xsmall()
                .flex_none()
                .max_w(rems(8.))
                .child(div().truncate().child(branch)),
        )
}

/// A file row: its status glyph, its name, cut short to fit, and `+N −N`. A deleted file's
/// name is struck through, and muted unless it is the one shown.
pub(in crate::app) fn file_row(
    id: String,
    name: String,
    entry: &GitChangeEntry,
    depth: usize,
    selected: bool,
    cx: &App,
) -> Stateful<Div> {
    let muted = cx.theme().muted_foreground;
    tree_row(id.into(), depth, RowKind::File { selected }, cx)
        .child(status_glyph(entry.status, cx))
        .child(
            div()
                .min_w_0()
                .flex_1()
                .truncate()
                .when(entry.status == GitChangeStatus::Deleted, |this| {
                    this.line_through()
                        .when(!selected, |this| this.text_color(muted))
                })
                .child(name),
        )
        .when_some(entry.stat, |this, stat| this.child(diff_stat(stat, cx)))
}

#[cfg(test)]
mod tests {
    use super::{ChangeNode, change_tree};
    use condr_core::{GitChangeEntry, GitChangeStatus};
    use relative_path::{RelativePath, RelativePathBuf};
    fn entry(path: &str) -> GitChangeEntry {
        GitChangeEntry {
            path: RelativePathBuf::from(path),
            old_path: None,
            status: GitChangeStatus::Modified,
            stat: None,
        }
    }

    #[test]
    fn the_tree_folds_lone_directories_and_puts_files_after_them() {
        let entries = [
            entry("crates/gui/src/app.rs"),
            entry("crates/gui/src/dock.rs"),
            entry("crates/core/lib.rs"),
            entry("README.md"),
            entry("Cargo.toml"),
        ];
        let refs: Vec<&GitChangeEntry> = entries.iter().collect();
        let tree = change_tree(&refs, &[]);
        let ChangeNode::Directory {
            label,
            path,
            children,
        } = &tree[0]
        else {
            panic!("directories come first: {tree:?}");
        };
        assert_eq!(label, "crates");
        assert_eq!(path, RelativePath::new("crates"));
        let labels: Vec<&str> = children
            .iter()
            .map(|node| match node {
                ChangeNode::Directory { label, .. } => label.as_str(),
                ChangeNode::File { name, .. } => name.as_str(),
            })
            .collect();
        assert_eq!(
            labels,
            ["core", "gui/src"],
            "a lone chain folds into one row"
        );
        let ChangeNode::Directory { path, children, .. } = &children[1] else {
            panic!("gui/src is a directory");
        };
        assert_eq!(path, RelativePath::new("crates/gui/src"));
        assert!(matches!(&children[0], ChangeNode::File { name, .. } if name == "app.rs"));
        assert!(matches!(&children[1], ChangeNode::File { name, .. } if name == "dock.rs"));
        assert!(matches!(&tree[1], ChangeNode::File { name, .. } if name == "Cargo.toml"));
        assert!(matches!(&tree[2], ChangeNode::File { name, .. } if name == "README.md"));
    }

    #[test]
    fn a_repository_under_the_root_stays_a_node_of_its_own() {
        let entries = [entry("api/src/lib.rs"), entry("web/app.ts")];
        let refs: Vec<&GitChangeEntry> = entries.iter().collect();
        let labels = |tree: &[ChangeNode<'_>]| -> Vec<String> {
            tree.iter()
                .map(|node| match node {
                    ChangeNode::Directory { label, .. } => label.clone(),
                    ChangeNode::File { name, .. } => name.clone(),
                })
                .collect()
        };
        assert_eq!(labels(&change_tree(&refs, &[])), ["api/src", "web"]);
        let tree = change_tree(&refs, &[RelativePath::new("api"), RelativePath::new("web")]);
        assert_eq!(labels(&tree), ["api", "web"]);
        let ChangeNode::Directory { children, .. } = &tree[0] else {
            panic!("api is a directory");
        };
        assert_eq!(labels(children), ["src"]);

        // The Root's own repository is a node first, holding what lies in no other.
        let entries = [
            entry("web/app.ts"),
            entry("src/main.rs"),
            entry("README.md"),
        ];
        let refs: Vec<&GitChangeEntry> = entries.iter().collect();
        let tree = change_tree(&refs, &[RelativePath::new(""), RelativePath::new("web")]);
        assert_eq!(labels(&tree), ["", "web"]);
        let ChangeNode::Directory { path, children, .. } = &tree[0] else {
            panic!("the Root's own repository is a node");
        };
        assert_eq!(path, RelativePath::new(""));
        assert_eq!(labels(children), ["src", "README.md"]);
    }
}
