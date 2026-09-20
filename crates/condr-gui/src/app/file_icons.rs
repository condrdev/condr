//! File-type icons for the Files sidebar and the Preview Tab (ADR 0018): the
//! JetBrains set (assets/icons/NOTICE-FILE-ICONS). Filename associations come from
//! native file-type registrations and bundled language definitions, generated alongside
//! the unmodified light/dark SVGs by `script/generate-file-icons.mjs`.
//!
//! The icons are multi-colour, so they are drawn with `img()`, which rasterizes an SVG
//! as-is, not with `Icon`, which tints a silhouette in the text colour.

mod generated;

use std::path::Path;

const PREFIX: &str = "icons/jetbrains/";

/// The asset path of the icon for a file called `name`.
pub(super) fn file_icon(name: &str, dark: bool) -> String {
    let lower = name.to_ascii_lowercase();
    let icon = lookup(generated::FILE_NAMES, name)
        .or_else(|| lookup(generated::FILE_NAMES_INSENSITIVE, &lower))
        .or_else(|| {
            generated::FILE_PATTERNS
                .iter()
                .find(|(pattern, _)| matches_pattern(name, pattern))
                .map(|(_, icon)| *icon)
        })
        .or_else(|| {
            // `a.test.ts` tries `test.ts`, then `ts`: the longest known suffix wins.
            lower
                .match_indices('.')
                .find_map(|(index, _)| lookup(generated::FILE_EXTENSIONS, &lower[index + 1..]))
        })
        .unwrap_or("unknown");
    icon_path(icon, dark)
}

/// JetBrains' unmarked directory. Source/test/excluded-root icons require IDE project
/// metadata; directory names and Git ignore status do not establish those roles.
pub(super) fn folder_icon(dark: bool) -> String {
    icon_path("folder", dark)
}

/// The icon for the last component of `path`, a file.
pub(super) fn file_icon_for_path(path: &Path, dark: bool) -> String {
    file_icon(
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default(),
        dark,
    )
}

fn icon_path(icon: &str, dark: bool) -> String {
    format!("{PREFIX}{icon}{}.svg", if dark { "_dark" } else { "" })
}

fn matches_pattern(name: &str, pattern: &str) -> bool {
    // ponytail: the imported patterns contain one '*'; generation rejects more complex
    // globs. Use a glob matcher if an upstream registration eventually needs one.
    let Some((prefix, suffix)) = pattern.split_once('*') else {
        return false;
    };
    name.len() >= prefix.len() + suffix.len() && name.starts_with(prefix) && name.ends_with(suffix)
}

/// The bytes behind one of this module's asset paths, for the asset source.
pub(crate) fn icon_bytes(path: &str) -> Option<&'static [u8]> {
    let icon = path.strip_prefix(PREFIX)?.strip_suffix(".svg")?;
    lookup(generated::ICONS, icon)
}

/// Every asset path this module serves, for the asset source's listing.
pub(crate) fn icon_paths() -> impl Iterator<Item = String> {
    generated::ICONS
        .iter()
        .map(|(icon, _)| format!("{PREFIX}{icon}.svg"))
}

fn lookup<T: Copy>(table: &'static [(&'static str, T)], key: &str) -> Option<T> {
    table
        .binary_search_by(|(candidate, _)| candidate.cmp(&key))
        .ok()
        .map(|index| table[index].1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tables_are_sorted_for_binary_search() {
        for table in [
            generated::FILE_NAMES,
            generated::FILE_NAMES_INSENSITIVE,
            generated::FILE_EXTENSIONS,
        ] {
            assert!(table.windows(2).all(|pair| pair[0].0 < pair[1].0));
        }
        assert!(
            generated::ICONS
                .windows(2)
                .all(|pair| pair[0].0 < pair[1].0)
        );
    }

    #[test]
    fn every_mapped_icon_exists_and_resolves_through_the_asset_path() {
        for (_, icon) in generated::FILE_NAMES
            .iter()
            .chain(generated::FILE_NAMES_INSENSITIVE)
            .chain(generated::FILE_EXTENSIONS)
            .chain(generated::FILE_PATTERNS)
        {
            for dark in [false, true] {
                assert!(
                    icon_bytes(&icon_path(icon, dark)).is_some(),
                    "{icon} has no SVG"
                );
            }
        }
        for dark in [false, true] {
            assert!(icon_bytes(&file_icon("weird.zzzz", dark)).is_some());
            assert!(icon_bytes(&folder_icon(dark)).is_some());
        }
        assert!(icon_bytes("icons/circle.svg").is_none());
    }

    #[test]
    fn jetbrains_associations_preserve_names_patterns_and_theme() {
        for (name, icon) in [
            ("main.rs", "rust"),
            ("Main.RS", "rust"),
            ("README.md", "markdown"),
            ("Dockerfile", "dockerfile"),
            ("dockerfile", "dockerfile"),
            ("Dockerfile.release", "dockerfile"),
            ("Gemfile", "ruby"),
            ("app.test.ts", "typescript"),
            ("types.d.ts", "typescript"),
            ("Cargo.toml", "toml"),
            ("Cargo.lock", "toml"),
            ("Cargo.toml.orig", "toml"),
            ("uv.lock", "toml"),
            ("CARGO.LOCK", "unknown"),
            (".editorconfig", "editorconfig"),
            (".gitignore", "gitignore"),
            ("no-extension", "unknown"),
            ("weird.zzzz", "unknown"),
        ] {
            for dark in [false, true] {
                assert_eq!(file_icon(name, dark), icon_path(icon, dark), "{name}");
            }
        }
        assert_eq!(
            file_icon_for_path(Path::new("crates/gui/Cargo.toml"), true),
            file_icon("Cargo.toml", true)
        );
        assert_ne!(file_icon("main.rs", false), file_icon("main.rs", true));
    }

    #[test]
    fn patterns_do_not_overlap_their_prefix_and_suffix() {
        assert!(matches_pattern("tsconfig.app.json", "tsconfig.*.json"));
        assert!(!matches_pattern("TSConfig.app.json", "tsconfig.*.json"));
        assert!(!matches_pattern("aba", "ab*ba"));
        assert!(matches_pattern("abba", "ab*ba"));
        for (pattern, _) in generated::FILE_PATTERNS {
            assert_eq!(pattern.matches('*').count(), 1);
            assert!(!pattern.contains('?'));
        }
    }

    #[test]
    fn bundled_svgs_render_in_both_themes() {
        for (icon, bytes) in generated::ICONS {
            let tree = resvg::usvg::Tree::from_data(bytes, &Default::default())
                .unwrap_or_else(|error| panic!("{icon}: {error}"));
            let mut pixmap = resvg::tiny_skia::Pixmap::new(16, 16).unwrap();
            resvg::render(&tree, Default::default(), &mut pixmap.as_mut());
            assert!(
                pixmap.pixels().iter().any(|pixel| pixel.alpha() > 0),
                "{icon}"
            );
        }
    }
}
