//! The directory tree Changes and History draw changed files in (ADR 0017, ADR 0046), and
//! how a changed file looks wherever it is listed. Rows here only draw: what a click does
//! is the caller's.

use super::sidebar::CondrIconName;
use super::*;
use condr_core::{GitChangeEntry, GitChangeStatus, GitDiffStat};

/// The glyph and colour a status shows, as Zed draws them: a square with a plus, a dot or
/// a minus in the theme's success, warning and danger colours; a conflict warns.
pub(super) fn status_glyph(status: GitChangeStatus, cx: &App) -> Icon {
    let name = match status {
        GitChangeStatus::Added | GitChangeStatus::Untracked => Icon::new(CondrIconName::SquarePlus),
        GitChangeStatus::Modified | GitChangeStatus::Renamed => Icon::new(CondrIconName::SquareDot),
        GitChangeStatus::Deleted => Icon::new(CondrIconName::SquareMinus),
        GitChangeStatus::Conflicted => Icon::new(IconName::TriangleAlert),
    };
    name.size_4().text_color(status_color(status, cx))
}

/// The theme's version-control colour for a status: success, warning or danger.
pub(super) fn status_color(status: GitChangeStatus, cx: &App) -> Hsla {
    match status {
        GitChangeStatus::Added | GitChangeStatus::Untracked => cx.theme().success,
        GitChangeStatus::Modified | GitChangeStatus::Renamed => cx.theme().warning,
        GitChangeStatus::Deleted | GitChangeStatus::Conflicted => cx.theme().danger,
    }
}

/// `+N −N`, each half only when it is not zero.
pub(super) fn diff_stat(stat: GitDiffStat, cx: &App) -> AnyElement {
    h_flex()
        .flex_none()
        .gap_1()
        .text_xs()
        .when(stat.added > 0, |this| {
            this.child(
                div()
                    .text_color(cx.theme().success)
                    .child(format!("+{}", compact_count(stat.added))),
            )
        })
        .when(stat.deleted > 0, |this| {
            this.child(
                div()
                    .text_color(cx.theme().danger)
                    .child(format!("−{}", compact_count(stat.deleted))),
            )
        })
        .into_any_element()
}

/// A line count as the stat shows it: `999`, `1.2k`, `12k`, `3.4M`, rounded down so it never
/// claims more lines than there are.
fn compact_count(n: u32) -> String {
    let (unit, suffix) = match n {
        0..1_000 => return n.to_string(),
        1_000..1_000_000 => (1_000, "k"),
        _ => (1_000_000, "M"),
    };
    let (whole, tenth) = (n / unit, n % unit / (unit / 10));
    if whole < 10 && tenth > 0 {
        format!("{whole}.{tenth}{suffix}")
    } else {
        format!("{whole}{suffix}")
    }
}

/// Changed files as a directory tree, the way Zed's git panel shows them: a
/// directory that holds nothing but one directory folds into it (`src/app`), files sort after
/// directories, and both sort by name.
#[derive(Debug, PartialEq)]
enum ChangeNode<'a> {
    Directory {
        /// The folded segments joined with `/`.
        label: String,
        /// The directory's repository-relative path, the key of its collapsed state.
        path: RelativePathBuf,
        children: Vec<ChangeNode<'a>>,
    },
    File {
        name: String,
        entry: &'a GitChangeEntry,
    },
}

fn change_tree<'a>(entries: &[&'a GitChangeEntry]) -> Vec<ChangeNode<'a>> {
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

    fn finish<'a>(builder: Builder<'a>, parent: &RelativePath) -> Vec<ChangeNode<'a>> {
        let mut nodes = Vec::new();
        for (name, mut child) in builder.directories {
            let mut label = name.clone();
            let mut path = parent.join(&name);
            // Fold a chain of lone directories into one row.
            while child.files.is_empty() && child.directories.len() == 1 {
                let (next_name, next) = child.directories.pop_first().expect("one entry");
                label.push('/');
                label.push_str(&next_name);
                path.push(&next_name);
                child = next;
            }
            nodes.push(ChangeNode::Directory {
                label,
                children: finish(child, &path),
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
    finish(root, RelativePath::new(""))
}

/// A tree row's left padding: the section gutter plus one indent per level. A file row
/// also skips the chevron slot a directory row starts with, so names line up; both scale
/// with the font since they frame text.
pub(super) fn tree_indent(depth: usize, past_chevron: bool) -> Rems {
    let chevron = if past_chevron { 1.125 } else { 0. };
    rems(0.5 + 0.875 * depth as f32 + chevron)
}

/// One visible row of a change tree, in display order.
pub(super) enum TreeRow<'a> {
    Directory {
        label: String,
        /// The directory's repository-relative path, the key of its folded state.
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

/// The rows `entries` show as a tree, leaving out what the directories `folded` names hold.
pub(super) fn tree_rows<'a>(
    entries: &[&'a GitChangeEntry],
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
    walk(change_tree(entries), 0, &folded, &mut rows);
    rows
}

/// A directory row: a chevron, a folder and the label.
pub(super) fn directory_row(
    id: String,
    label: String,
    depth: usize,
    folded: bool,
    cx: &App,
) -> Stateful<Div> {
    let theme = cx.theme();
    let selector = SharedString::from(id);
    h_flex()
        .id(selector.clone())
        .debug_selector(move || selector.to_string())
        .h_7()
        .w_full()
        .min_w_0()
        .pl(tree_indent(depth, false))
        .pr_2()
        .gap_1()
        .items_center()
        .cursor_pointer()
        .rounded(theme.radius)
        .text_sm()
        .hover(|this| this.bg(theme.sidebar_accent.opacity(0.8)))
        .child(
            Icon::new(IconName::ChevronRight)
                .size_3p5()
                .text_color(theme.muted_foreground)
                .when(!folded, |icon| icon.rotate(percentage(90. / 360.))),
        )
        .child(
            Icon::new(IconName::Folder)
                .size_4()
                .text_color(theme.muted_foreground),
        )
        .child(div().min_w_0().flex_1().truncate().child(label))
}

/// A file row: its status glyph, its name, cut short to fit, and `+N −N`.
pub(super) fn file_row(
    id: String,
    name: String,
    entry: &GitChangeEntry,
    depth: usize,
    selected: bool,
    cx: &App,
) -> Stateful<Div> {
    let theme = cx.theme();
    let selector = SharedString::from(id);
    h_flex()
        .id(selector.clone())
        .debug_selector(move || selector.to_string())
        .h_7()
        .w_full()
        .min_w_0()
        .pl(tree_indent(depth, true))
        .pr_2()
        .gap_2()
        .items_center()
        .cursor_pointer()
        .rounded(theme.radius)
        .text_sm()
        .when(!selected, |this| {
            this.hover(|this| {
                this.bg(theme.sidebar_accent.opacity(0.8))
                    .text_color(theme.sidebar_accent_foreground)
            })
        })
        .when(selected, |this| {
            this.bg(theme.tokens.sidebar_accent)
                .text_color(theme.sidebar_accent_foreground)
        })
        .child(status_glyph(entry.status, cx))
        .child(
            div()
                .min_w_0()
                .flex_1()
                .truncate()
                .when(entry.status == GitChangeStatus::Deleted, |this| {
                    this.line_through().text_color(theme.muted_foreground)
                })
                .child(name),
        )
        .when_some(entry.stat, |this, stat| this.child(diff_stat(stat, cx)))
}

#[cfg(test)]
mod tests {
    use super::{ChangeNode, change_tree, compact_count};
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
    fn large_line_counts_shorten_to_k_and_m() {
        let shown = [
            0, 999, 1_000, 1_234, 9_999, 10_500, 999_999, 1_000_000, 3_456_789,
        ]
        .map(compact_count);
        assert_eq!(
            shown,
            [
                "0", "999", "1k", "1.2k", "9.9k", "10k", "999k", "1M", "3.4M"
            ]
        );
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
        let tree = change_tree(&refs);
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
}
