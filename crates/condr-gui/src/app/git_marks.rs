//! How Git's facts look wherever a row shows them: a change's status glyph and colour,
//! a `+N −N` line count, and a commit's short id.

use super::icons::CondrIconName;
use super::*;
use condr_core::{GitChangeStatus, GitDiffStat};

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

pub(super) fn short_id(id: &str) -> &str {
    &id[..id.len().min(7)]
}

#[cfg(test)]
mod tests {
    use super::compact_count;
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
}
