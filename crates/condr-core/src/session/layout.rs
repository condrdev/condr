use super::*;

/// Attaches `new_pane` beside `target` on the given side of a fresh split.
pub(super) fn split_layout(
    layout: &mut PaneLayout,
    target: PaneId,
    new_pane: PaneId,
    side: PaneDirection,
    ratio: f32,
) -> bool {
    match layout {
        PaneLayout::Pane(id) if *id == target => {
            let direction = match side {
                PaneDirection::Left | PaneDirection::Right => SplitDirection::Horizontal,
                PaneDirection::Up | PaneDirection::Down => SplitDirection::Vertical,
            };
            let (first, second) = match side {
                PaneDirection::Left | PaneDirection::Up => (new_pane, target),
                PaneDirection::Right | PaneDirection::Down => (target, new_pane),
            };
            *layout = PaneLayout::Split {
                direction,
                ratio,
                first: Box::new(PaneLayout::Pane(first)),
                second: Box::new(PaneLayout::Pane(second)),
            };
            true
        }
        PaneLayout::Pane(_) => false,
        PaneLayout::Split { first, second, .. } => {
            split_layout(first, target, new_pane, side, ratio)
                || split_layout(second, target, new_pane, side, ratio)
        }
    }
}

pub(super) fn pane_layout_depth(
    layout: &PaneLayout,
    target: PaneId,
    depth: usize,
) -> Option<usize> {
    match layout {
        PaneLayout::Pane(id) => (*id == target).then_some(depth),
        PaneLayout::Split { first, second, .. } => pane_layout_depth(first, target, depth + 1)
            .or_else(|| pane_layout_depth(second, target, depth + 1)),
    }
}

pub(super) fn valid_split_ratio(ratio: f32) -> f32 {
    if ratio.is_finite() {
        ratio.clamp(0.1, 0.9)
    } else {
        0.5
    }
}

pub(super) fn remove_from_layout(layout: &PaneLayout, target: PaneId) -> Option<PaneLayout> {
    match layout {
        PaneLayout::Pane(id) if *id == target => None,
        PaneLayout::Pane(_) => Some(layout.clone()),
        PaneLayout::Split {
            direction,
            ratio,
            first,
            second,
        } => match (
            remove_from_layout(first, target),
            remove_from_layout(second, target),
        ) {
            (None, Some(second)) => Some(second),
            (Some(first), None) => Some(first),
            (Some(first), Some(second)) => Some(PaneLayout::Split {
                direction: *direction,
                ratio: *ratio,
                first: Box::new(first),
                second: Box::new(second),
            }),
            (None, None) => None,
        },
    }
}

pub(super) fn first_pane_id(layout: &PaneLayout) -> PaneId {
    match layout {
        PaneLayout::Pane(id) => *id,
        PaneLayout::Split { first, .. } => first_pane_id(first),
    }
}

pub(super) fn neighbor_pane_id(
    layout: &PaneLayout,
    target: PaneId,
    direction: PaneDirection,
) -> Option<PaneId> {
    let mut rects = Vec::new();
    collect_pane_rects(layout, 0.0, 0.0, 1.0, 1.0, &mut rects);
    let target = *rects.iter().find(|rect| rect.id == target)?;
    rects
        .into_iter()
        .filter(|candidate| candidate.id != target.id)
        .filter_map(|candidate| {
            directional_score(target, candidate, direction).map(|score| (candidate.id, score))
        })
        .min_by(|(_, left), (_, right)| left.total_cmp(right))
        .map(|(id, _)| id)
}

pub(super) fn collect_pane_rects(
    layout: &PaneLayout,
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
    rects: &mut Vec<PaneRect>,
) {
    match layout {
        PaneLayout::Pane(id) => rects.push(PaneRect {
            id: *id,
            left,
            top,
            right,
            bottom,
        }),
        PaneLayout::Split {
            direction: SplitDirection::Horizontal,
            ratio,
            first,
            second,
        } => {
            let split = left + (right - left) * ratio;
            collect_pane_rects(first, left, top, split, bottom, rects);
            collect_pane_rects(second, split, top, right, bottom, rects);
        }
        PaneLayout::Split {
            direction: SplitDirection::Vertical,
            ratio,
            first,
            second,
        } => {
            let split = top + (bottom - top) * ratio;
            collect_pane_rects(first, left, top, right, split, rects);
            collect_pane_rects(second, left, split, right, bottom, rects);
        }
    }
}

fn directional_score(
    target: PaneRect,
    candidate: PaneRect,
    direction: PaneDirection,
) -> Option<f32> {
    let (primary, secondary, center) = match direction {
        PaneDirection::Left if candidate.right <= target.left + f32::EPSILON => (
            target.left - candidate.right,
            interval_gap(target.top, target.bottom, candidate.top, candidate.bottom),
            ((target.top + target.bottom) - (candidate.top + candidate.bottom)).abs(),
        ),
        PaneDirection::Right if candidate.left >= target.right - f32::EPSILON => (
            candidate.left - target.right,
            interval_gap(target.top, target.bottom, candidate.top, candidate.bottom),
            ((target.top + target.bottom) - (candidate.top + candidate.bottom)).abs(),
        ),
        PaneDirection::Up if candidate.bottom <= target.top + f32::EPSILON => (
            target.top - candidate.bottom,
            interval_gap(target.left, target.right, candidate.left, candidate.right),
            ((target.left + target.right) - (candidate.left + candidate.right)).abs(),
        ),
        PaneDirection::Down if candidate.top >= target.bottom - f32::EPSILON => (
            candidate.top - target.bottom,
            interval_gap(target.left, target.right, candidate.left, candidate.right),
            ((target.left + target.right) - (candidate.left + candidate.right)).abs(),
        ),
        _ => return None,
    };
    Some(primary * 100.0 + secondary * 10.0 + center)
}

fn interval_gap(first_start: f32, first_end: f32, second_start: f32, second_end: f32) -> f32 {
    if first_end < second_start {
        second_start - first_end
    } else if second_end < first_start {
        first_start - second_end
    } else {
        0.0
    }
}

pub(super) fn contains_pane(layout: &PaneLayout, pane_id: PaneId) -> bool {
    match layout {
        PaneLayout::Pane(id) => *id == pane_id,
        PaneLayout::Split { first, second, .. } => {
            contains_pane(first, pane_id) || contains_pane(second, pane_id)
        }
    }
}

pub(super) fn resize_between(
    layout: &mut PaneLayout,
    target: PaneId,
    neighbor: PaneId,
    amount: f32,
) -> bool {
    let PaneLayout::Split {
        ratio,
        first,
        second,
        ..
    } = layout
    else {
        return false;
    };
    let target_first = contains_pane(first, target);
    let neighbor_first = contains_pane(first, neighbor);
    if target_first != neighbor_first {
        let next = valid_split_ratio(*ratio + if target_first { amount } else { -amount });
        if next == *ratio {
            return false;
        }
        *ratio = next;
        return true;
    }
    if target_first {
        resize_between(first, target, neighbor, amount)
    } else {
        resize_between(second, target, neighbor, amount)
    }
}

pub(super) fn swap_layout_panes(layout: &mut PaneLayout, first_id: PaneId, second_id: PaneId) {
    match layout {
        PaneLayout::Pane(id) if *id == first_id => *id = second_id,
        PaneLayout::Pane(id) if *id == second_id => *id = first_id,
        PaneLayout::Pane(_) => {}
        PaneLayout::Split { first, second, .. } => {
            swap_layout_panes(first, first_id, second_id);
            swap_layout_panes(second, first_id, second_id);
        }
    }
}

pub(super) fn split_count(layout: &PaneLayout) -> usize {
    match layout {
        PaneLayout::Pane(_) => 0,
        PaneLayout::Split { first, second, .. } => 1 + split_count(first) + split_count(second),
    }
}

pub(super) fn apply_split_ratios(layout: &mut PaneLayout, ratios: &mut impl Iterator<Item = f32>) {
    if let PaneLayout::Split {
        ratio,
        first,
        second,
        ..
    } = layout
    {
        *ratio = valid_split_ratio(ratios.next().expect("split ratio count was checked"));
        apply_split_ratios(first, ratios);
        apply_split_ratios(second, ratios);
    }
}
