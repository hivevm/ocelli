//! History exploration on the laid-out graph — search and lineage — ported from
//! the desktop's `search.ts` and `ancestry.ts` so both frontends answer the same
//! questions the same way.

use ocelli_core::model::NodeJson;
use std::collections::HashSet;

/// Rows of nodes matching `query`, newest first.
///
/// A case-insensitive substring test over what a row shows: message, author,
/// committer, SHA, and ref names. A blank query matches nothing — an empty
/// result means "no search active".
pub fn search(nodes: &[NodeJson], query: &str) -> Vec<usize> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    let mut rows: Vec<usize> = nodes
        .iter()
        .filter(|n| {
            n.message.to_lowercase().contains(&q)
                || n.author.to_lowercase().contains(&q)
                || n.committer.to_lowercase().contains(&q)
                || n.sha.to_lowercase().contains(&q)
                || n.refs.iter().any(|r| r.to_lowercase().contains(&q))
        })
        .map(|n| n.row as usize)
        .collect();
    rows.sort_unstable();
    rows
}

/// Rows of the commit's full lineage: itself, every loaded ancestor and every
/// descendant. Parents beyond the commit limit have no node and are skipped.
/// Iterative, so deep histories cannot exhaust the call stack.
pub fn lineage<'a>(
    start: &'a NodeJson,
    lookup: impl Fn(&str) -> Option<&'a NodeJson>,
) -> HashSet<usize> {
    let mut rows = walk(start, &lookup, |n| &n.parents);
    rows.extend(walk(start, &lookup, |n| &n.children));
    rows
}

fn walk<'a>(
    start: &'a NodeJson,
    lookup: &impl Fn(&str) -> Option<&'a NodeJson>,
    next: impl Fn(&'a NodeJson) -> &'a [String],
) -> HashSet<usize> {
    let mut rows = HashSet::new();
    let mut seen = HashSet::new();
    let mut stack = vec![start];
    while let Some(node) = stack.pop() {
        if !seen.insert(node.sha.as_str()) {
            continue;
        }
        rows.insert(node.row as usize);
        for sha in next(node) {
            if let Some(n) = lookup(sha) {
                stack.push(n);
            }
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocelli_core::graph::build_graph;
    use ocelli_core::model::{CommitData, CommitKind, Graph};
    use std::collections::HashMap;

    fn commit(sha: &str, time: i64, parents: &[&str], refs: &[&str]) -> CommitData {
        CommitData {
            sha: sha.to_string(),
            message: format!("Message of {sha}"),
            author_name: if sha == "c" { "Carol" } else { "Ada" }.to_string(),
            author_email: String::new(),
            author_time: time,
            committer_name: "Ada".to_string(),
            committer_email: String::new(),
            commit_time: time,
            refs: refs.iter().map(|r| r.to_string()).collect(),
            parent_shas: parents.iter().map(|p| p.to_string()).collect(),
            kind: CommitKind::Commit,
        }
    }

    /// a ← b ← d (merge of b and c), a ← c, and e on top of c (unmerged).
    fn graph() -> Graph {
        build_graph(vec![
            commit("a", 1, &[], &[]),
            commit("b", 2, &["a"], &[]),
            commit("c", 3, &["a"], &["feature"]),
            commit("d", 4, &["b", "c"], &["main"]),
            commit("e", 5, &["c"], &[]),
        ])
    }

    fn shas(g: &Graph, rows: impl IntoIterator<Item = usize>) -> Vec<String> {
        let mut out: Vec<String> = rows
            .into_iter()
            .map(|r| {
                g.nodes
                    .iter()
                    .find(|n| n.row as usize == r)
                    .unwrap()
                    .sha
                    .clone()
            })
            .collect();
        out.sort();
        out
    }

    #[test]
    // Verifies: G-16
    fn search_matches_author_and_refs_case_insensitively() {
        let g = graph();
        assert_eq!(shas(&g, search(&g.nodes, "CAROL")), ["c"]);
        assert_eq!(shas(&g, search(&g.nodes, "main")), ["d"]);
        assert_eq!(
            shas(&g, search(&g.nodes, " FEAT ")),
            ["c"],
            "trimmed, on refs"
        );
        assert!(search(&g.nodes, "   ").is_empty());
    }

    #[test]
    // Verifies: G-16
    fn search_results_run_newest_first() {
        let g = graph();
        let rows = search(&g.nodes, "message");
        assert_eq!(rows.len(), 5);
        assert!(rows.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    // Verifies: G-17
    fn lineage_covers_ancestors_and_descendants_but_not_siblings() {
        let g = graph();
        let by_sha: HashMap<&str, &NodeJson> =
            g.nodes.iter().map(|n| (n.sha.as_str(), n)).collect();
        let b = by_sha["b"];
        let lookup = |sha: &str| by_sha.get(sha).copied();
        assert_eq!(shas(&g, lineage(b, lookup)), ["a", "b", "d"]);
        let c = by_sha["c"];
        assert_eq!(shas(&g, lineage(c, lookup)), ["a", "c", "d", "e"]);
    }
}
