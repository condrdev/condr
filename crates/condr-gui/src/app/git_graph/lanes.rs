//! The commit graph's lanes (ADR 0079): which column each commit's dot sits in and the edges
//! each row draws, from the commits in the order the Server walked them.

use std::collections::HashMap;

/// An edge across half a row: from a lane at one end to a lane at the other.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Edge {
    pub(super) from: usize,
    pub(super) to: usize,
    pub(super) color: u32,
}

/// What one row of the graph draws.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct GraphRow {
    /// The lane the commit's dot sits in.
    pub(super) lane: usize,
    /// The dot's colour, its lane's.
    pub(super) color: u32,
    /// From the row's top edge, at `from`, to its middle, at `to`.
    pub(super) top: Vec<Edge>,
    /// From the row's middle, at `from`, to its bottom edge, at `to`.
    pub(super) bottom: Vec<Edge>,
    /// How many lanes wide the row is.
    pub(super) width: usize,
}

/// A commit as the layout needs it.
pub(super) struct GraphCommit<'a> {
    pub(super) id: &'a str,
    pub(super) parents: &'a [String],
    /// The ref a lane starting here takes its colour from.
    pub(super) head_ref: Option<&'a str>,
}

struct Lane<'a> {
    waits_for: &'a str,
    color: u32,
}

/// The rows `commits` draw, children before parents. A lane starting at a ref takes its
/// colour from the ref's name, so a branch keeps its colour; any other takes the next of a
/// counter, as IntelliJ colours a fragment without a ref by its index. Lanes waiting for a
/// parent past the last commit run off the bottom.
pub(super) fn layout(commits: &[GraphCommit<'_>]) -> Vec<GraphRow> {
    let mut lanes: Vec<Option<Lane<'_>>> = Vec::new();
    let mut fragments = 0u32;
    let mut next_fragment = || {
        fragments += 1;
        fragments
    };
    let mut rows = Vec::with_capacity(commits.len());
    for commit in commits {
        let hits: Vec<usize> = lanes
            .iter()
            .enumerate()
            .filter(|(_, lane)| {
                lane.as_ref()
                    .is_some_and(|lane| lane.waits_for == commit.id)
            })
            .map(|(index, _)| index)
            .collect();
        let (lane, color) = match hits.first() {
            Some(&first) => (first, lanes[first].as_ref().map_or(0, |lane| lane.color)),
            None => (
                free_slot(&mut lanes, None),
                commit.head_ref.map_or_else(&mut next_fragment, name_color),
            ),
        };
        let top = lanes
            .iter()
            .enumerate()
            .filter_map(|(index, slot)| {
                let slot = slot.as_ref()?;
                Some(Edge {
                    from: index,
                    to: if slot.waits_for == commit.id {
                        lane
                    } else {
                        index
                    },
                    color: slot.color,
                })
            })
            .collect();
        for &hit in &hits {
            lanes[hit] = None;
        }
        let mut bottom = Vec::new();
        let mut opened = Vec::new();
        if let Some(first) = commit.parents.first() {
            lanes[lane] = Some(Lane {
                waits_for: first,
                color,
            });
        }
        for parent in commit.parents.iter().skip(1) {
            let waiting = lanes
                .iter()
                .position(|slot| slot.as_ref().is_some_and(|slot| slot.waits_for == parent));
            match waiting {
                Some(index) => bottom.push(Edge {
                    from: lane,
                    to: index,
                    color: lanes[index].as_ref().map_or(color, |slot| slot.color),
                }),
                None => {
                    let index = free_slot(&mut lanes, Some(lane));
                    let color = next_fragment();
                    lanes[index] = Some(Lane {
                        waits_for: parent,
                        color,
                    });
                    opened.push(index);
                    bottom.push(Edge {
                        from: lane,
                        to: index,
                        color,
                    });
                }
            }
        }
        for (index, slot) in lanes.iter().enumerate() {
            if let Some(slot) = slot
                && !opened.contains(&index)
            {
                bottom.push(Edge {
                    from: index,
                    to: index,
                    color: slot.color,
                });
            }
        }
        while lanes.last().is_some_and(Option::is_none) {
            lanes.pop();
        }
        let row = GraphRow {
            lane,
            color,
            top,
            bottom,
            width: 0,
        };
        let width = row
            .top
            .iter()
            .chain(&row.bottom)
            .flat_map(|edge| [edge.from, edge.to])
            .chain([lane])
            .max()
            .map_or(1, |widest| widest + 1);
        rows.push(GraphRow { width, ..row });
    }
    rows
}

/// The leftmost free lane other than `taken`, or a new one at the right.
fn free_slot(lanes: &mut Vec<Option<Lane<'_>>>, taken: Option<usize>) -> usize {
    match lanes
        .iter()
        .enumerate()
        .position(|(index, slot)| slot.is_none() && Some(index) != taken)
    {
        Some(index) => index,
        None => {
            lanes.push(None);
            lanes.len() - 1
        }
    }
}

/// A ref name's colour seed, stable across runs and machines.
fn name_color(name: &str) -> u32 {
    // FNV-1a.
    name.bytes().fold(0x811c_9dc5, |hash, byte| {
        (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193)
    })
}

/// The first ref at each commit a lane colours itself by: a local branch before a remote
/// one before a tag, as their labels list them.
pub(super) fn head_refs<'a>(
    refs: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> HashMap<&'a str, &'a str> {
    let rank = |name: &str| {
        ["refs/heads/", "refs/remotes/", "refs/tags/"]
            .iter()
            .position(|prefix| name.starts_with(prefix))
            .unwrap_or(usize::MAX)
    };
    let mut heads: HashMap<&str, &str> = HashMap::new();
    for (name, commit) in refs {
        if rank(name) == usize::MAX {
            continue;
        }
        let better = heads
            .get(commit)
            .is_none_or(|kept| (rank(name), name) < (rank(kept), *kept));
        if better {
            heads.insert(commit, name);
        }
    }
    heads
}

#[cfg(test)]
mod tests {
    use super::{Edge, GraphCommit, layout};

    fn commits<'a>(spec: &'a [(&'a str, Vec<String>)]) -> Vec<GraphCommit<'a>> {
        spec.iter()
            .map(|(id, parents)| GraphCommit {
                id,
                parents,
                head_ref: None,
            })
            .collect()
    }

    fn parents(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|id| (*id).to_owned()).collect()
    }

    #[test]
    fn a_line_stays_in_one_lane() {
        let spec = [
            ("c", parents(&["b"])),
            ("b", parents(&["a"])),
            ("a", parents(&[])),
        ];
        let rows = layout(&commits(&spec));
        assert!(rows.iter().all(|row| row.lane == 0 && row.width == 1));
        assert!(rows[0].top.is_empty(), "nothing above the tip");
        assert_eq!(
            rows[1].top,
            [Edge {
                from: 0,
                to: 0,
                color: rows[0].color
            }]
        );
        assert!(rows[2].bottom.is_empty(), "nothing below the root");
    }

    #[test]
    fn a_merge_opens_a_lane_for_its_second_parent_and_the_fork_closes_it() {
        // m merges f into the line; f and b both come from a.
        let spec = [
            ("m", parents(&["b", "f"])),
            ("f", parents(&["a"])),
            ("b", parents(&["a"])),
            ("a", parents(&[])),
        ];
        let rows = layout(&commits(&spec));
        let lanes: Vec<usize> = rows.iter().map(|row| row.lane).collect();
        assert_eq!(lanes, [0, 1, 0, 0]);
        // The merge goes down to its second parent's new lane.
        assert!(rows[0].bottom.contains(&Edge {
            from: 0,
            to: 1,
            color: rows[1].color
        }));
        assert_ne!(rows[0].color, rows[1].color);
        // Both lanes wait for `a` and meet at it.
        let into_a: Vec<(usize, usize)> = rows[3]
            .top
            .iter()
            .map(|edge| (edge.from, edge.to))
            .collect();
        assert_eq!(into_a, [(0, 0), (1, 0)]);
        assert_eq!(rows[3].width, 2);
    }

    #[test]
    fn an_octopus_opens_a_lane_per_parent_and_lanes_are_reused() {
        let spec = [
            ("o", parents(&["a", "b", "c"])),
            ("c", parents(&[])),
            ("b", parents(&[])),
            ("a", parents(&[])),
            ("t", parents(&[])),
        ];
        let rows = layout(&commits(&spec));
        assert_eq!(rows[0].bottom.len(), 3);
        assert_eq!(rows[0].width, 3);
        let lanes: Vec<usize> = rows.iter().map(|row| row.lane).collect();
        assert_eq!(lanes, [0, 2, 1, 0, 0], "a freed lane is taken again");
    }

    #[test]
    fn a_lane_at_a_ref_keeps_its_colour_and_a_cut_lane_runs_off_the_bottom() {
        let spec = [("x", parents(&["w"])), ("y", parents(&["v"]))];
        let mut graph = commits(&spec);
        graph[0].head_ref = Some("refs/heads/main");
        graph[1].head_ref = Some("refs/heads/main");
        let rows = layout(&graph);
        assert_eq!(
            rows[0].color, rows[1].color,
            "the same name, the same colour"
        );
        assert_eq!(rows[1].lane, 1);
        assert_eq!(rows[1].bottom.len(), 2, "both lanes wait past the page");
    }
}
