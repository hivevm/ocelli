use crate::model::{CommitData, CommitKind, Edge, Graph, Node, NodeJson};
use std::collections::{HashMap, HashSet};

/// Builds the complete graph with layout from raw commits.
/// Runs both layout passes — rows via temporal topological sort, columns via
/// straight branches — and then derives the edge list.
pub fn build_graph(commits: Vec<CommitData>) -> Graph {
    // 1. Build Node arena with SHA -> index lookup
    let mut sha_to_idx: HashMap<String, usize> = HashMap::new();
    let mut nodes: Vec<Node> = commits
        .into_iter()
        .enumerate()
        .map(|(i, c)| {
            sha_to_idx.insert(c.sha.clone(), i);
            Node {
                commit: c,
                row: -1,
                col: -1,
                source_indices: vec![],
                target_indices: vec![],
            }
        })
        .collect();

    // 2. Wire adjacency (parent->child relationships)
    //    source = parent (older), target = child (newer)
    //    node.source_indices = parent indices
    //    parent.target_indices includes this node
    for i in 0..nodes.len() {
        let parent_shas = nodes[i].commit.parent_shas.clone();
        for parent_sha in parent_shas {
            if let Some(&p_idx) = sha_to_idx.get(&parent_sha) {
                nodes[i].source_indices.push(p_idx);
                nodes[p_idx].target_indices.push(i);
            }
        }
    }

    // 3. Layout Step 1: temporal topological sort
    temporal_topological_sort(&mut nodes);

    // 4. Layout Step 2: straight branches (column assignment)
    straight_branches(&mut nodes);

    // 5. Build edges
    let edges = build_edges(&nodes);

    // 6. Convert to JSON-serializable form
    let node_jsons = nodes
        .iter()
        .map(|n| {
            let sha = n.commit.sha.clone();
            // The working-directory node has no real OID — don't show a short sha.
            let short_sha = if n.commit.kind == CommitKind::Working {
                String::new()
            } else {
                sha[..sha.len().min(7)].to_string()
            };
            let message = n.commit.message.lines().next().unwrap_or("").to_string();
            let author_date = format_commit_time(n.commit.author_time);
            let date = format_commit_time(n.commit.commit_time);

            // Children sorted by row so the newest child comes first, keeping
            // the order deterministic regardless of adjacency insertion order.
            let mut children: Vec<(i32, String)> = n
                .target_indices
                .iter()
                .map(|&c| (nodes[c].row, nodes[c].commit.sha.clone()))
                .collect();
            children.sort();
            let children = children.into_iter().map(|(_, sha)| sha).collect();

            NodeJson {
                row: n.row,
                col: n.col,
                sha,
                short_sha,
                message,
                author: n.commit.author_name.clone(),
                author_date,
                author_avatar: gravatar_url(&n.commit.author_email),
                committer: n.commit.committer_name.clone(),
                date,
                committer_avatar: gravatar_url(&n.commit.committer_email),
                refs: n.commit.refs.clone(),
                kind: n.commit.kind,
                parents: n.commit.parent_shas.clone(),
                children,
            }
        })
        .collect();

    Graph {
        nodes: node_jsons,
        edges,
    }
}

/// Builds a Gravatar avatar URL from an email address.
/// Returns an empty string for empty emails. Uses `d=404` so the image request
/// fails when no Gravatar exists, letting the frontend fall back to initials.
/// The URL is only a local computation; whether it is ever dereferenced is the
/// frontend's opt-in avatar setting (ADR-0023).
fn gravatar_url(email: &str) -> String {
    use sha2::{Digest, Sha256};
    let normalized = email.trim().to_lowercase();
    if normalized.is_empty() {
        return String::new();
    }
    let hash: String = Sha256::digest(normalized.as_bytes())
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect();
    format!("https://www.gravatar.com/avatar/{}?d=404&s=80", hash)
}

/// Formats a Unix timestamp as a human-readable date string.
fn format_commit_time(epoch_secs: i64) -> String {
    use chrono::{DateTime, Utc};
    let dt = DateTime::<Utc>::from_timestamp(epoch_secs, 0).unwrap_or_default();
    dt.format("%Y-%m-%d %H:%M").to_string()
}

// ============================================================================
// Layout Step 1: Temporal Topological Sort
// ============================================================================

/// Assigns node.row to every node.
/// Row 0 = newest commit; larger row = older. A parent always receives a higher
/// row number than each of its children, while commits stay ordered by committer date.
fn temporal_topological_sort(nodes: &mut [Node]) {
    // Reset all rows
    for n in nodes.iter_mut() {
        n.row = -1;
    }

    // Sort indices by commit_time descending (newest first)
    let mut by_date_desc: Vec<usize> = (0..nodes.len()).collect();
    by_date_desc.sort_by(|&a, &b| {
        nodes[b]
            .commit
            .commit_time
            .cmp(&nodes[a].commit.commit_time)
    });

    let mut counter: i32 = 0;
    let mut explored = vec![false; nodes.len()];
    // Reused across every start commit: allocating it per start would make the
    // pass O(n · starts) instead of the near-linear cost ADR-0015 states. Safe to
    // share, because a completed traversal leaves a row assigned on every node it
    // marked, and only nodes still lacking a row are ever pushed again.
    let mut expanded = vec![false; nodes.len()];

    for &start_idx in &by_date_desc {
        if !explored[start_idx] {
            dfs_mark_explored(nodes, start_idx, &mut explored);
            assign_counter(nodes, start_idx, &mut counter, &mut expanded);
        }
    }
}

/// Iterative depth-first traversal that marks every reachable node as explored.
/// Uses an explicit work stack, so deep histories cannot exhaust the call stack.
fn dfs_mark_explored(nodes: &[Node], start: usize, explored: &mut [bool]) {
    let mut stack: Vec<usize> = vec![start];
    while let Some(&top) = stack.last() {
        if !explored[top] {
            explored[top] = true;
            // Push target (child) indices sorted by commit_time ascending
            // so newest is processed first when popped
            let mut sorted_targets = nodes[top].target_indices.clone();
            sorted_targets.sort_by(|&a, &b| {
                nodes[a]
                    .commit
                    .commit_time
                    .cmp(&nodes[b].commit.commit_time)
            });
            for child_idx in sorted_targets {
                if !explored[child_idx] {
                    stack.push(child_idx);
                }
            }
        } else {
            stack.pop();
        }
    }
}

/// Iterative depth-first traversal that assigns rows in post-order, so children
/// receive lower row numbers than their parents.
fn assign_counter(nodes: &mut [Node], start: usize, counter: &mut i32, expanded: &mut [bool]) {
    let mut work: Vec<usize> = vec![start];

    while let Some(&top) = work.last() {
        if !expanded[top] {
            expanded[top] = true;
            let mut unvisited_targets: Vec<usize> = nodes[top]
                .target_indices
                .iter()
                .copied()
                .filter(|&i| nodes[i].row == -1)
                .collect();
            // Sort newest-first (descending commit_time)
            unvisited_targets.sort_by(|&a, &b| {
                nodes[b]
                    .commit
                    .commit_time
                    .cmp(&nodes[a].commit.commit_time)
            });
            for child in unvisited_targets {
                work.push(child);
            }
        } else {
            work.pop();
            if nodes[top].row == -1 {
                nodes[top].row = *counter;
                *counter += 1;
            }
        }
    }
}

// ============================================================================
// Layout Step 2: Straight Branches
// ============================================================================

/// Assigns node.col to every node, so all commits on one branch line share a
/// column and the branch renders as a straight vertical line.
fn straight_branches(nodes: &mut [Node]) {
    // Process in row order (row 0 = newest first)
    let mut sorted_indices: Vec<usize> = (0..nodes.len()).collect();
    sorted_indices.sort_by_key(|&i| nodes[i].row);

    // active_branches[j] = Some(node_index) if column j is occupied
    // col_start_row[j] = row at which the occupant was placed
    let mut active_branches: Vec<Option<usize>> = Vec::new();
    let mut col_start_row: Vec<i32> = Vec::new();
    // last_used_row[j] = the lowest row so far at which column j held a node
    // or a branch line ended. A column freed again is still "used" in the rows
    // its line ran through, and a merge edge must not pass through them.
    let mut last_used_row: Vec<i32> = Vec::new();

    for &c_idx in &sorted_indices {
        let branch_children = branch_children_of(nodes, c_idx);
        let merge_children = merge_children_of(nodes, c_idx);

        // Compute forbidden columns J(c)
        let forbidden = compute_forbidden(
            nodes,
            &merge_children,
            nodes[c_idx].row,
            &active_branches,
            &col_start_row,
            &last_used_row,
        );

        // Find valid branch target to replace (leftmost non-forbidden)
        let replaced = branch_children
            .iter()
            .filter(|&&d| nodes[d].col >= 0 && !forbidden.contains(&(nodes[d].col as usize)))
            .min_by_key(|&&d| nodes[d].col)
            .copied();

        if let Some(r_idx) = replaced {
            let col = nodes[r_idx].col as usize;
            active_branches[col] = Some(c_idx);
            col_start_row[col] = nodes[c_idx].row;
            nodes[c_idx].col = col as i32;
        } else {
            // Find first free non-forbidden slot, or append
            let insert_col = (0..active_branches.len())
                .find(|&j| active_branches[j].is_none() && !forbidden.contains(&j))
                .unwrap_or_else(|| {
                    active_branches.push(None);
                    col_start_row.push(0);
                    last_used_row.push(-1);
                    active_branches.len() - 1
                });
            active_branches[insert_col] = Some(c_idx);
            col_start_row[insert_col] = nodes[c_idx].row;
            nodes[c_idx].col = insert_col as i32;
        }

        let c_row = nodes[c_idx].row;
        last_used_row[nodes[c_idx].col as usize] = c_row;

        // Free all other branch-target slots. Their lines run down to this
        // row (the fork edge bends into them here), so they count as used.
        for &d_idx in &branch_children {
            if Some(d_idx) != replaced {
                let d_col = nodes[d_idx].col;
                if d_col >= 0 && (d_col as usize) < active_branches.len() {
                    active_branches[d_col as usize] = None;
                    last_used_row[d_col as usize] = c_row;
                }
            }
        }
    }
}

/// Computes the forbidden column set J(c) for node c — every column in which
/// the incoming MERGE edges could not be drawn without overlap:
/// - an active branch line that spans the edges' row range, and
/// - any column used (a node, or a branch line ending) strictly between the
///   highest merge child and c, even if it has been freed since — otherwise
///   the merge edge's vertical run would pass through that commit. This is
///   Pvigier's "column j is non-empty between rows min{d.i} and c.i", as his
///   reference implementation (gitamine) computes it.
fn compute_forbidden(
    nodes: &[Node],
    merge_children: &[usize],
    c_row: i32,
    active_branches: &[Option<usize>],
    col_start_row: &[i32],
    last_used_row: &[i32],
) -> HashSet<usize> {
    if merge_children.is_empty() {
        return HashSet::new();
    }

    let i_min = merge_children
        .iter()
        .map(|&i| nodes[i].row)
        .min()
        .unwrap_or(c_row);

    (0..active_branches.len())
        .filter(|&j| {
            (active_branches[j].is_some() && col_start_row[j] <= i_min) || last_used_row[j] > i_min
        })
        .collect()
}

/// Branch children of node c_idx: targets where c_idx == target.source_indices[0]
fn branch_children_of(nodes: &[Node], c_idx: usize) -> Vec<usize> {
    nodes[c_idx]
        .target_indices
        .iter()
        .copied()
        .filter(|&t| nodes[t].source_indices.first() == Some(&c_idx))
        .collect()
}

/// Merge children: targets where c_idx is NOT the first source
fn merge_children_of(nodes: &[Node], c_idx: usize) -> Vec<usize> {
    nodes[c_idx]
        .target_indices
        .iter()
        .copied()
        .filter(|&t| nodes[t].source_indices.first() != Some(&c_idx))
        .collect()
}

// ============================================================================
// Build edges
// ============================================================================

/// Derives the full edge list from the laid-out commit graph.
/// Call this AFTER both layout steps have been executed.
fn build_edges(nodes: &[Node]) -> Vec<Edge> {
    let mut edges = Vec::new();
    for c_idx in 0..nodes.len() {
        for &t_idx in &nodes[c_idx].target_indices {
            let is_branch = nodes[t_idx].source_indices.first() == Some(&c_idx);
            let src_row = nodes[c_idx].row;
            let tgt_row = nodes[t_idx].row;
            edges.push(Edge {
                source_row: src_row,
                source_col: nodes[c_idx].col,
                target_row: tgt_row,
                target_col: nodes[t_idx].col,
                is_branch,
                min_row: src_row.min(tgt_row),
                max_row: src_row.max(tgt_row),
            });
        }
    }
    edges
}

// ============================================================================
// Tests
// ============================================================================
//
// The specification requires row assignment, column assignment and edge building
// to be covered by tests that need no repository window and no running UI. These
// operate on plain `CommitData` fixtures and assert on the laid-out graph.

#[cfg(test)]
mod tests {
    use super::*;

    /// Fixture commit. `time` is the committer date; parents are listed
    /// first-parent first, exactly as git orders them.
    fn commit(sha: &str, time: i64, parents: &[&str]) -> CommitData {
        CommitData {
            sha: sha.to_string(),
            message: format!("commit {sha}"),
            author_name: "Test".to_string(),
            author_email: "test@example.com".to_string(),
            author_time: time,
            committer_name: "Test".to_string(),
            committer_email: "test@example.com".to_string(),
            commit_time: time,
            refs: vec![],
            parent_shas: parents.iter().map(|s| s.to_string()).collect(),
            kind: CommitKind::Commit,
        }
    }

    fn row_of(g: &Graph, sha: &str) -> i32 {
        g.nodes.iter().find(|n| n.sha == sha).expect(sha).row
    }

    fn col_of(g: &Graph, sha: &str) -> i32 {
        g.nodes.iter().find(|n| n.sha == sha).expect(sha).col
    }

    /// A ─ B ─ C, newest first. Rows must run 0, 1, 2 and all share one column.
    #[test]
    // Verifies: ADR-0015, G-1, G-2
    fn linear_history_is_one_straight_column() {
        let g = build_graph(vec![
            commit("A", 300, &["B"]),
            commit("B", 200, &["C"]),
            commit("C", 100, &[]),
        ]);

        assert_eq!(row_of(&g, "A"), 0, "newest commit must be row 0");
        assert_eq!(row_of(&g, "B"), 1);
        assert_eq!(row_of(&g, "C"), 2);
        for sha in ["A", "B", "C"] {
            assert_eq!(col_of(&g, sha), 0, "{sha} must stay in the first column");
        }
    }

    /// Every parent must sit below each of its children, for every edge.
    #[test]
    // Verifies: ADR-0015, G-10
    fn parents_are_always_below_their_children() {
        // M merges F into main; main is B ─ C, feature is F ─ C.
        let g = build_graph(vec![
            commit("M", 500, &["B", "F"]),
            commit("B", 400, &["C"]),
            commit("F", 300, &["C"]),
            commit("C", 100, &[]),
        ]);

        for (child, parents) in [("M", vec!["B", "F"]), ("B", vec!["C"]), ("F", vec!["C"])] {
            for parent in parents {
                assert!(
                    row_of(&g, parent) > row_of(&g, child),
                    "{parent} (row {}) must be below {child} (row {})",
                    row_of(&g, parent),
                    row_of(&g, child),
                );
            }
        }
    }

    /// Independent commits are ordered by committer date, newest at the top.
    #[test]
    // Verifies: ADR-0015, G-10
    fn independent_tips_are_ordered_by_committer_date() {
        // Two disconnected chains. The newer tip must come first.
        let g = build_graph(vec![
            commit("old_tip", 200, &["old_root"]),
            commit("old_root", 100, &[]),
            commit("new_tip", 400, &["new_root"]),
            commit("new_root", 300, &[]),
        ]);

        assert!(
            row_of(&g, "new_tip") < row_of(&g, "old_tip"),
            "the newer chain must be laid out above the older one",
        );
        assert_eq!(row_of(&g, "new_tip"), 0);
    }

    /// Two disconnected components exercise more than one traversal start, which
    /// is what the shared scratch buffer in `temporal_topological_sort` touches.
    /// Every node must still receive a distinct row.
    #[test]
    fn disconnected_components_all_get_distinct_rows() {
        let commits = vec![
            commit("a1", 600, &["a2"]),
            commit("a2", 500, &[]),
            commit("b1", 400, &["b2"]),
            commit("b2", 300, &[]),
            commit("c1", 200, &[]),
        ];
        let n = commits.len() as i32;
        let g = build_graph(commits);

        let mut rows: Vec<i32> = g.nodes.iter().map(|node| node.row).collect();
        rows.sort_unstable();
        assert_eq!(
            rows,
            (0..n).collect::<Vec<_>>(),
            "rows must be 0..n with no gaps or repeats"
        );
    }

    /// Specification Goal 9: the same input always yields the same layout.
    #[test]
    // Verifies: ADR-0015, G-15
    fn layout_is_deterministic() {
        let fixture = || {
            vec![
                commit("M", 500, &["B", "F"]),
                commit("B", 400, &["C"]),
                commit("F", 300, &["C"]),
                commit("C", 200, &["R"]),
                commit("R", 100, &[]),
            ]
        };

        let first = build_graph(fixture());
        let second = build_graph(fixture());

        let layout = |g: &Graph| -> Vec<(String, i32, i32)> {
            g.nodes
                .iter()
                .map(|n| (n.sha.clone(), n.row, n.col))
                .collect()
        };
        assert_eq!(layout(&first), layout(&second));

        let edges = |g: &Graph| -> Vec<(i32, i32, i32, i32, bool)> {
            g.edges
                .iter()
                .map(|e| {
                    (
                        e.source_row,
                        e.source_col,
                        e.target_row,
                        e.target_col,
                        e.is_branch,
                    )
                })
                .collect()
        };
        assert_eq!(edges(&first), edges(&second));
    }

    /// A merge parent must not land in the column its own branch line occupies,
    /// so the two branch lines stay separate columns.
    #[test]
    // Verifies: ADR-0015
    fn merge_puts_the_second_parent_in_its_own_column() {
        let g = build_graph(vec![
            commit("M", 500, &["B", "F"]),
            commit("B", 400, &["C"]),
            commit("F", 300, &["C"]),
            commit("C", 100, &[]),
        ]);

        assert_ne!(
            col_of(&g, "B"),
            col_of(&g, "F"),
            "the merged-in branch needs a column of its own",
        );
        assert_eq!(
            col_of(&g, "M"),
            col_of(&g, "B"),
            "a merge continues its first parent's line"
        );
    }

    /// `is_branch` marks exactly the first-parent edges; everything else is a
    /// MERGE edge. `min_row`/`max_row` must span the two endpoints.
    #[test]
    // Verifies: ADR-0030
    fn edges_distinguish_branch_from_merge_and_span_their_rows() {
        let g = build_graph(vec![
            commit("M", 500, &["B", "F"]),
            commit("B", 400, &["C"]),
            commit("F", 300, &["C"]),
            commit("C", 100, &[]),
        ]);

        // M has two incoming edges: B -> M is a BRANCH edge, F -> M a MERGE edge.
        let m_row = row_of(&g, "M");
        let into_m: Vec<&Edge> = g.edges.iter().filter(|e| e.target_row == m_row).collect();
        assert_eq!(into_m.len(), 2, "the merge commit has two parents");
        assert_eq!(
            into_m.iter().filter(|e| e.is_branch).count(),
            1,
            "exactly one of them is the first parent",
        );

        for e in &g.edges {
            assert_eq!(e.min_row, e.source_row.min(e.target_row));
            assert_eq!(e.max_row, e.source_row.max(e.target_row));
            assert!(
                e.min_row < e.max_row,
                "an edge always spans at least one row"
            );
        }
    }

    /// Parents outside the loaded set are ignored rather than panicking — that is
    /// what makes a truncated load (the commit limit) safe.
    #[test]
    // Verifies: ADR-0015
    fn unknown_parents_are_dropped() {
        let g = build_graph(vec![
            commit("A", 200, &["B"]),
            commit("B", 100, &["missing"]),
        ]);

        assert_eq!(g.nodes.len(), 2);
        assert_eq!(
            g.edges.len(),
            1,
            "only the edge between loaded nodes survives"
        );
    }

    /// The detail contract: every node carries its parent SHAs in commit order —
    /// including parents outside the loaded set — and the SHAs of its loaded
    /// children, newest (lowest row) first.
    #[test]
    // Verifies: ADR-0015, G-12
    fn nodes_carry_parents_and_children() {
        // M merges F into main; main is B ─ C, feature is F ─ C.
        // C additionally records a parent that was not loaded.
        let g = build_graph(vec![
            commit("M", 500, &["B", "F"]),
            commit("B", 400, &["C"]),
            commit("F", 300, &["C"]),
            commit("C", 100, &["missing"]),
        ]);

        let node = |sha: &str| g.nodes.iter().find(|n| n.sha == sha).expect(sha);

        assert_eq!(
            node("M").parents,
            vec!["B", "F"],
            "parents keep commit order, first parent first"
        );
        assert_eq!(
            node("C").parents,
            vec!["missing"],
            "unloaded parents stay listed even though they get no edge"
        );
        assert!(node("M").children.is_empty());

        let c_children = &node("C").children;
        assert_eq!(c_children.len(), 2);
        let rows: Vec<i32> = c_children.iter().map(|sha| row_of(&g, sha)).collect();
        assert!(rows[0] < rows[1], "children are ordered newest first");
    }

    /// The working-directory node has no object id, so it must not show a short sha.
    #[test]
    // Verifies: ADR-0024
    fn working_directory_node_has_no_short_sha() {
        let mut wip = commit("WORKING_DIRECTORY", 900, &["A"]);
        wip.kind = CommitKind::Working;

        let g = build_graph(vec![wip, commit("A", 100, &[])]);
        let node = g
            .nodes
            .iter()
            .find(|n| n.sha == "WORKING_DIRECTORY")
            .unwrap();

        assert_eq!(node.short_sha, "");
        assert_eq!(node.row, 0, "uncommitted changes sort to the top");
    }

    /// The avatar URL uses Gravatar's SHA-256 identifier of the trimmed,
    /// lowercased address; an empty address yields no URL at all.
    #[test]
    // Verifies: ADR-0023
    fn gravatar_url_hashes_normalized_email_with_sha256() {
        // SHA-256 of "gravatar@example.com".
        assert_eq!(
            gravatar_url("  Gravatar@Example.com "),
            "https://www.gravatar.com/avatar/\
             0b75aca8db05083a7e419cc3b10f44a91c0a88a23bdcfb5cc72f0d41dec4ebab?d=404&s=80"
        );
        assert_eq!(gravatar_url("   "), "");
    }

    /// The cells an edge's vertical segment runs through, endpoints excluded,
    /// following the specification's edge shapes (ADR-0030): a BRANCH edge
    /// runs vertically in the child's column, a MERGE edge in the parent's.
    fn vertical_cells(e: &Edge) -> Vec<(i32, i32)> {
        let col = if e.is_branch {
            e.target_col
        } else {
            e.source_col
        };
        (e.min_row + 1..e.max_row).map(|row| (row, col)).collect()
    }

    /// Regression: a merge edge ran vertically through an unrelated commit.
    ///
    /// `typo` forks `api` off main, and `merge` (on main) takes `login` as
    /// its second parent. `api` sits between `merge` and `login` and briefly
    /// occupies column 1; once its line ended there, column 1 counted as free
    /// for `login` although the merge edge from `login` runs up column 1 right
    /// through `api`. J(c) must include every column used between the highest
    /// merge child and `c` — as in Pvigier's reference implementation — not
    /// only the columns still active.
    #[test]
    // Verifies: ADR-0015, ADR-0030, G-9
    fn a_merge_edge_never_runs_through_another_commit() {
        let g = build_graph(vec![
            commit("init", 1, &[]),
            commit("login", 2, &["init"]),
            commit("typo", 3, &["init"]),
            commit("api", 4, &["typo"]),
            commit("merge", 5, &["typo", "login"]),
        ]);
        let at: HashMap<(i32, i32), &str> = g
            .nodes
            .iter()
            .map(|n| ((n.row, n.col), n.sha.as_str()))
            .collect();
        for e in &g.edges {
            for cell in vertical_cells(e) {
                assert!(
                    !at.contains_key(&cell),
                    "edge {e:?} runs through {} at {cell:?}",
                    at[&cell]
                );
            }
        }
    }

    /// The same property over many generated histories, for every edge: with
    /// shapes chosen by edge type (ADR-0030), each vertical run lies in a
    /// column the layout keeps clear — a BRANCH edge in its child's active
    /// line, a MERGE edge in the parent's column guarded by J(c).
    #[test]
    // Verifies: ADR-0015, ADR-0030, G-9
    fn generated_histories_keep_every_edge_clear_of_commits() {
        // xorshift — deterministic, and no dependency for a test.
        let mut seed: u64 = 0x9e37_79b9_7f4a_7c15;
        let mut next = move |n: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n as u64) as usize
        };
        for history in 0..200 {
            let mut commits = vec![commit("c0", 0, &[])];
            let mut tips = vec!["c0".to_string()];
            for i in 1..120 {
                let t = next(tips.len());
                let mut parents = vec![tips[t].clone()];
                // Merge another tip in, one commit in four.
                if tips.len() > 1 && next(4) == 0 {
                    let o = (t + 1 + next(tips.len() - 1)) % tips.len();
                    parents.push(tips[o].clone());
                }
                let sha = format!("c{i}");
                let refs: Vec<&str> = parents.iter().map(String::as_str).collect();
                commits.push(commit(&sha, i, &refs));
                // The merged tip ends; the first parent's tip moves on, or forks.
                if let Some(merged) = parents.get(1) {
                    tips.retain(|x| x != merged);
                }
                let t = tips.iter().position(|x| *x == parents[0]).unwrap();
                if next(3) == 0 {
                    tips.push(sha);
                } else {
                    tips[t] = sha;
                }
            }
            let g = build_graph(commits);
            let at: HashMap<(i32, i32), &str> = g
                .nodes
                .iter()
                .map(|n| ((n.row, n.col), n.sha.as_str()))
                .collect();
            for e in &g.edges {
                for cell in vertical_cells(e) {
                    assert!(
                        !at.contains_key(&cell),
                        "history {history}: edge {e:?} runs through {}",
                        at[&cell]
                    );
                }
            }
        }
    }
}
