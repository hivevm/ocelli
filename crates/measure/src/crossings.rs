//! Counting the *edge crossings* (glossary) in Ocelli's layout (Q-1).
//!
//! An edge is drawn as an orthogonal L-shape whose bend follows its type (ADR-0030): a branch edge
//! runs along the parent's row and then up the child's lane, a merge edge up the parent's lane and
//! then along the child's row, and an edge within one lane is a single vertical line. No commit can
//! sit where two segments meet strictly inside both: every row holds one commit, at an end of each
//! horizontal run on it. Edges to one parent overlap where git draws a single line, which is why a
//! point counts once.

use ocelli_core::model::{Edge, Graph};
use std::collections::HashSet;

/// A horizontal segment on `row`, from lane `from` to lane `to` (`from < to`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Horizontal {
    row: i32,
    from: i32,
    to: i32,
}

/// A vertical segment in `lane`, from row `from` to row `to` (`from < to`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Vertical {
    lane: i32,
    from: i32,
    to: i32,
}

/// The straight segments an edge is drawn with (ADR-0030).
fn segments(e: &Edge) -> (Option<Horizontal>, Option<Vertical>) {
    let (low, high) = (
        e.target_row.min(e.source_row),
        e.target_row.max(e.source_row),
    );
    if e.source_col == e.target_col {
        return (None, vertical(e.source_col, low, high));
    }
    let (from, to) = (
        e.source_col.min(e.target_col),
        e.source_col.max(e.target_col),
    );
    if e.is_branch {
        let along = Horizontal {
            row: e.source_row,
            from,
            to,
        };
        (Some(along), vertical(e.target_col, low, high))
    } else {
        let along = Horizontal {
            row: e.target_row,
            from,
            to,
        };
        (Some(along), vertical(e.source_col, low, high))
    }
}

fn vertical(lane: i32, from: i32, to: i32) -> Option<Vertical> {
    (from < to).then_some(Vertical { lane, from, to })
}

/// The number of edge crossings in a laid-out commit graph.
pub fn count(graph: &Graph) -> usize {
    count_edges(&graph.edges)
}

/// The number of edge crossings among `edges`.
pub fn count_edges(edges: &[Edge]) -> usize {
    let parts: Vec<_> = edges.iter().map(segments).collect();
    let mut crossings = HashSet::new();
    for (i, (horizontal, _)) in parts.iter().enumerate() {
        let Some(h) = horizontal else { continue };
        for (j, (_, vertical)) in parts.iter().enumerate() {
            let Some(v) = vertical else { continue };
            if i != j && h.from < v.lane && v.lane < h.to && v.from < h.row && h.row < v.to {
                crossings.insert((h.row, v.lane));
            }
        }
    }
    crossings.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edge(source: (i32, i32), target: (i32, i32), is_branch: bool) -> Edge {
        Edge {
            source_row: source.0,
            source_col: source.1,
            target_row: target.0,
            target_col: target.1,
            is_branch,
            min_row: source.0.min(target.0),
            max_row: source.0.max(target.0),
        }
    }

    #[test]
    fn two_branches_that_fork_and_merge_do_not_cross() {
        // main: rows 0, 3 in lane 0; feature: rows 1, 2 in lane 1, forked at row 3, merged at 0.
        let edges = [
            edge((3, 0), (0, 0), true),
            edge((3, 0), (2, 1), true),
            edge((2, 1), (1, 1), true),
            edge((1, 1), (0, 0), false),
        ];
        assert_eq!(count_edges(&edges), 0);
    }

    #[test]
    fn a_merge_edge_across_an_occupied_lane_crosses_it_once() {
        // Lane 1 runs from row 4 up to row 0; a merge edge from lane 2 at row 3 runs up lane 2
        // and along row 1 to lane 0, crossing lane 1 at row 1.
        let edges = [edge((4, 1), (0, 1), true), edge((3, 2), (1, 0), false)];
        assert_eq!(count_edges(&edges), 1);
    }

    #[test]
    fn two_merge_edges_across_an_occupied_lane_cross_it_twice() {
        let edges = [
            edge((5, 1), (0, 1), true),
            edge((4, 2), (1, 0), false),
            edge((3, 2), (2, 0), false),
        ];
        assert_eq!(count_edges(&edges), 2);
    }

    #[test]
    fn a_branch_edge_crosses_along_the_parents_row() {
        // A branch edge from lane 0 at row 3 runs along row 3 to lane 2, across lane 1, which
        // runs from row 4 up to row 0.
        let edges = [edge((4, 1), (0, 1), true), edge((3, 0), (1, 2), true)];
        assert_eq!(count_edges(&edges), 1);
    }

    #[test]
    fn edges_that_overlap_where_they_cross_count_once() {
        // Two branch edges leave the commit at row 3, lane 0, along the same row to lanes 2 and
        // 3, and both pass lane 1, which runs from row 4 up to row 0: one point, one crossing.
        let edges = [
            edge((4, 1), (0, 1), true),
            edge((3, 0), (1, 2), true),
            edge((3, 0), (2, 3), true),
        ];
        assert_eq!(count_edges(&edges), 1);
    }

    #[test]
    fn edges_that_meet_only_at_a_commit_do_not_cross() {
        // Both edges leave the commit at row 2, lane 1: one up its lane, one along its row.
        let edges = [
            edge((2, 1), (0, 1), true),
            edge((2, 1), (1, 0), false),
            edge((2, 1), (0, 2), false),
        ];
        assert_eq!(count_edges(&edges), 0);
    }

    fn commit(sha: &str, time: i64, parents: &[&str]) -> ocelli_core::model::CommitData {
        ocelli_core::model::CommitData {
            sha: sha.into(),
            message: sha.into(),
            author_name: String::new(),
            author_email: String::new(),
            author_time: time,
            committer_name: String::new(),
            committer_email: String::new(),
            commit_time: time,
            refs: Vec::new(),
            parent_shas: parents.iter().map(|p| p.to_string()).collect(),
            kind: ocelli_core::model::CommitKind::Commit,
        }
    }

    #[test]
    fn a_feature_merged_back_into_main_lays_out_without_a_crossing() {
        let graph = ocelli_core::graph::build_graph(vec![
            commit("a", 1, &[]),
            commit("b", 2, &["a"]),
            commit("f", 3, &["b"]),
            commit("c", 4, &["b"]),
            commit("m", 5, &["c", "f"]),
        ]);
        assert_eq!(graph.nodes.len(), 5);
        assert_eq!(count(&graph), 0);
    }

    #[test]
    fn the_count_of_a_layout_is_the_same_every_time() {
        let history = || {
            vec![
                commit("a", 1, &[]),
                commit("x", 2, &["a"]),
                commit("y", 3, &["a"]),
                commit("z", 4, &["a"]),
                commit("m", 5, &["x", "y"]),
                commit("n", 6, &["m", "z"]),
            ]
        };
        let first = count(&ocelli_core::graph::build_graph(history()));
        assert_eq!(first, count(&ocelli_core::graph::build_graph(history())));
    }
}
