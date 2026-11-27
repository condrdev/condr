//! Git Graph's branch tree (ADR 0079), as IntelliJ's log draws its Branches pane: `HEAD`,
//! then Local, Remote with a node per remote, and Tags, each name grouped by `/`.

use condr_core::GitRef;
use std::collections::{BTreeMap, HashSet};

/// What a ref is, by its name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RefKind {
    Head,
    Local,
    Remote,
    Tag,
}

impl RefKind {
    pub(super) fn of(name: &str) -> Option<Self> {
        if name == "HEAD" {
            Some(Self::Head)
        } else if name.starts_with("refs/heads/") {
            Some(Self::Local)
        } else if name.starts_with("refs/remotes/") {
            Some(Self::Remote)
        } else if name.starts_with("refs/tags/") {
            Some(Self::Tag)
        } else {
            None
        }
    }
}

/// A ref's name as a person reads it: `main`, `origin/main`, `v1.0`.
pub(super) fn short_name(name: &str) -> &str {
    ["refs/heads/", "refs/remotes/", "refs/tags/"]
        .iter()
        .find_map(|prefix| name.strip_prefix(prefix))
        .unwrap_or(name)
}

/// One visible row of the tree, in display order.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum BranchRow {
    /// A group or a `/` folder; `key` names its folded state.
    Folder {
        key: String,
        label: String,
        depth: usize,
        folded: bool,
    },
    /// A ref, by its full name, or `HEAD`.
    Ref {
        name: String,
        label: String,
        depth: usize,
        kind: RefKind,
        current: bool,
    },
}

#[derive(Default)]
struct Folder<'a> {
    folders: BTreeMap<&'a str, Folder<'a>>,
    refs: Vec<(&'a str, &'a str)>,
}

impl<'a> Folder<'a> {
    fn insert(&mut self, path: &'a str, name: &'a str) {
        match path.split_once('/') {
            Some((folder, rest)) => self.folders.entry(folder).or_default().insert(rest, name),
            None => self.refs.push((path, name)),
        }
    }
}

/// A submenu of the Branch filter: its `/` folders, then its refs by label and full name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct RefMenu {
    pub(super) folders: BTreeMap<String, RefMenu>,
    pub(super) refs: Vec<(String, String)>,
}

impl RefMenu {
    fn insert(&mut self, path: &str, name: &str) {
        match path.split_once('/') {
            Some((folder, rest)) => self
                .folders
                .entry(folder.to_owned())
                .or_default()
                .insert(rest, name),
            None => self.refs.push((path.to_owned(), name.to_owned())),
        }
    }
}

/// The Branch filter's submenus as IntelliJ's has them: Local, each remote by its name,
/// then Tags; empty ones left out.
pub(super) fn ref_menus(refs: &[GitRef]) -> Vec<(RefKind, String, RefMenu)> {
    let (mut local, mut remotes, mut tags) = (
        RefMenu::default(),
        BTreeMap::<&str, RefMenu>::new(),
        RefMenu::default(),
    );
    for reference in refs {
        let name = reference.name.as_str();
        if let Some(path) = name.strip_prefix("refs/heads/") {
            local.insert(path, name);
        } else if let Some((remote, path)) = name
            .strip_prefix("refs/remotes/")
            .and_then(|rest| rest.split_once('/'))
        {
            remotes.entry(remote).or_default().insert(path, name);
        } else if let Some(path) = name.strip_prefix("refs/tags/") {
            tags.insert(path, name);
        }
    }
    let mut menus = vec![(RefKind::Local, String::new(), local)];
    menus.extend(
        remotes
            .into_iter()
            .map(|(remote, menu)| (RefKind::Remote, remote.to_owned(), menu)),
    );
    menus.push((RefKind::Tag, String::new(), tags));
    menus.retain(|(_, _, menu)| !menu.folders.is_empty() || !menu.refs.is_empty());
    menus
}

/// The rows `refs` show. `current` is the checked-out branch's short name: only the folders
/// on the way to it start open, and `toggled` names the folders opened or shut against
/// that. A non-empty `filter` keeps the refs whose short name holds it, ignoring case, and
/// opens every folder on the way to them.
pub(super) fn branch_rows(
    refs: &[GitRef],
    current: Option<&str>,
    filter: &str,
    toggled: &HashSet<String>,
) -> Vec<BranchRow> {
    let filter = filter.to_lowercase();
    let shown = |name: &str| filter.is_empty() || short_name(name).to_lowercase().contains(&filter);
    let mut rows = Vec::new();
    if refs.iter().any(|reference| reference.name == "HEAD") && shown("HEAD") {
        rows.push(BranchRow::Ref {
            name: "HEAD".into(),
            label: String::new(),
            depth: 0,
            kind: RefKind::Head,
            current: false,
        });
    }
    for (group, kind, strip) in [
        ("local", RefKind::Local, "refs/heads/"),
        ("remote", RefKind::Remote, "refs/remotes/"),
        ("tags", RefKind::Tag, "refs/tags/"),
    ] {
        let mut root = Folder::default();
        for reference in refs {
            if let Some(path) = reference.name.strip_prefix(strip)
                && shown(&reference.name)
            {
                root.insert(path, &reference.name);
            }
        }
        if root.folders.is_empty() && root.refs.is_empty() {
            continue;
        }
        let open = is_open(group, current, &filter, toggled);
        rows.push(BranchRow::Folder {
            key: group.into(),
            label: group.into(),
            depth: 0,
            folded: !open,
        });
        if open {
            let current = current.filter(|_| kind == RefKind::Local);
            walk(&root, group, 1, kind, current, &filter, toggled, &mut rows);
        }
    }
    rows
}

/// Whether the folder `key` (`local`, `local/feat`, `remote/origin`…) is open.
fn is_open(key: &str, current: Option<&str>, filter: &str, toggled: &HashSet<String>) -> bool {
    // The folders a Local branch `feat/x` sits in are `local` and `local/feat`.
    let on_the_way = key.strip_prefix("local").is_some_and(|path| {
        current.is_some_and(|current| format!("/{current}").starts_with(&format!("{path}/")))
    });
    !filter.is_empty() || on_the_way != toggled.contains(key)
}

#[allow(clippy::too_many_arguments)]
fn walk(
    folder: &Folder<'_>,
    key: &str,
    depth: usize,
    kind: RefKind,
    current: Option<&str>,
    filter: &str,
    toggled: &HashSet<String>,
    rows: &mut Vec<BranchRow>,
) {
    let is_current = |name: &str| current == Some(short_name(name));
    // The current branch first, then folders, then names, as IntelliJ sorts them.
    let mut refs = folder.refs.clone();
    refs.sort_by_key(|(label, name)| (!is_current(name), *label));
    let (first, rest): (Vec<_>, Vec<_>) = refs.into_iter().partition(|(_, name)| is_current(name));
    let mut push = |label: &str, name: &str| {
        rows.push(BranchRow::Ref {
            name: name.into(),
            label: label.into(),
            depth,
            kind,
            current: is_current(name),
        })
    };
    for (label, name) in &first {
        push(label, name);
    }
    for (label, child) in &folder.folders {
        let child_key = format!("{key}/{label}");
        let open = is_open(&child_key, current, filter, toggled);
        rows.push(BranchRow::Folder {
            key: child_key.clone(),
            label: (*label).into(),
            depth,
            folded: !open,
        });
        if open {
            walk(
                child,
                &child_key,
                depth + 1,
                kind,
                current,
                filter,
                toggled,
                rows,
            );
        }
    }
    for (label, name) in &rest {
        rows.push(BranchRow::Ref {
            name: (*name).into(),
            label: (*label).into(),
            depth,
            kind,
            current: false,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{BranchRow, RefKind, RefMenu, branch_rows, ref_menus};
    use condr_core::GitRef;
    use std::collections::HashSet;

    fn refs(names: &[&str]) -> Vec<GitRef> {
        names
            .iter()
            .map(|name| GitRef {
                name: (*name).into(),
                commit: "c".into(),
            })
            .collect()
    }

    fn labels(rows: &[BranchRow]) -> Vec<String> {
        rows.iter()
            .map(|row| match row {
                BranchRow::Folder { label, depth, .. } => format!("{depth}{label}/"),
                BranchRow::Ref { label, depth, .. } if label.is_empty() => format!("{depth}HEAD"),
                BranchRow::Ref { label, depth, .. } => format!("{depth}{label}"),
            })
            .collect()
    }

    #[test]
    fn groups_come_in_order_and_names_fold_into_folders() {
        let refs = refs(&[
            "refs/tags/v1",
            "refs/remotes/origin/main",
            "refs/remotes/origin/feat/a",
            "refs/heads/zeta",
            "refs/heads/feat/x",
            "refs/heads/main",
            "HEAD",
        ]);
        let opened: HashSet<String> = [
            "local/feat",
            "remote",
            "remote/origin",
            "remote/origin/feat",
            "tags",
        ]
        .map(str::to_owned)
        .into();
        let rows = branch_rows(&refs, Some("main"), "", &opened);
        assert_eq!(
            labels(&rows),
            [
                "0HEAD", "0local/", "1main", "1feat/", "2x", "1zeta", "0remote/", "1origin/",
                "2feat/", "3a", "2main", "0tags/", "1v1",
            ]
        );
        assert!(matches!(
            &rows[2],
            BranchRow::Ref {
                current: true,
                kind: RefKind::Local,
                ..
            }
        ));
    }

    #[test]
    fn only_the_way_to_the_current_branch_starts_open_and_a_filter_opens_the_way() {
        let refs = refs(&[
            "refs/heads/feat/x",
            "refs/heads/main",
            "refs/remotes/origin/main",
            "refs/tags/v1",
        ]);
        let none = HashSet::new();
        assert_eq!(
            labels(&branch_rows(&refs, Some("feat/x"), "", &none)),
            ["0local/", "1feat/", "2x", "1main", "0remote/", "0tags/"]
        );
        // Detached, every folder starts shut.
        assert_eq!(
            labels(&branch_rows(&refs, None, "", &none)),
            ["0local/", "0remote/", "0tags/"]
        );
        // A toggle shuts a folder on the way and opens one off it.
        let toggled: HashSet<String> = ["local/feat".to_owned(), "tags".to_owned()].into();
        assert_eq!(
            labels(&branch_rows(&refs, Some("feat/x"), "", &toggled)),
            ["0local/", "1feat/", "1main", "0remote/", "0tags/", "1v1"]
        );
        assert_eq!(
            labels(&branch_rows(&refs, Some("feat/x"), "X", &toggled)),
            ["0local/", "1feat/", "2x"]
        );
    }

    #[test]
    fn the_branch_menus_are_local_each_remote_and_tags_folded_by_slash() {
        let refs = refs(&[
            "HEAD",
            "refs/heads/main",
            "refs/heads/feat/x",
            "refs/remotes/origin/main",
            "refs/remotes/upstream/main",
        ]);
        let menus = ref_menus(&refs);
        let kinds: Vec<_> = menus
            .iter()
            .map(|(kind, name, _)| (*kind, name.as_str()))
            .collect();
        assert_eq!(
            kinds,
            [
                (RefKind::Local, ""),
                (RefKind::Remote, "origin"),
                (RefKind::Remote, "upstream")
            ],
            "no tags, no menu"
        );
        let local = &menus[0].2;
        assert_eq!(
            local.refs,
            [("main".to_owned(), "refs/heads/main".to_owned())]
        );
        assert_eq!(
            local.folders["feat"],
            RefMenu {
                folders: Default::default(),
                refs: vec![("x".into(), "refs/heads/feat/x".into())],
            }
        );
    }
}
