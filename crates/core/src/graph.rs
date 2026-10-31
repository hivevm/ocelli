use crate::model::{CommitData, CommitKind, Edge, Graph, NodeJson};
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};

/// Builds the complete graph with layout from raw commits, laid out as one chunk.
pub fn build_graph(commits: Vec<CommitData>) -> Graph {
    let mut layout = Layout::new();
    let chunk = layout.place(commits);
    chunk.graph(&layout)
}

/// A commit graph laid out chunk by chunk (ADR-0015). Each chunk is laid out with both of
/// Vigier's phases, rows via the temporal topological sort over the chunk alone and lanes via
/// straight branches carried on from the chunks before, and placed below them. A chunk holds
/// commits the walk yields after every commit of the chunks before it, so each of its commits is
/// an ancestor of commits laid out before, but for a late child, whose date runs backwards and
/// whose parent stands above it: what is laid out never moves when a chunk is added, and the first
/// chunk is laid out exactly as it is alone.
///
/// The layout keeps its nodes in an index arena for the whole load, every reference between them
/// an index, and finds a commit by its SHA through one index keyed by the SHA's first eight bytes;
/// a node keeps only its place, its first parent, and its SHA as twenty bytes, so a load costs no
/// allocation per commit once its chunk is handed over (ADR-0015).
#[derive(Debug, Default)]
pub struct Layout {
    nodes: Vec<Node>,
    /// Node indices by the first eight bytes of their SHA, the rest checked against the node.
    index: HashMap<u64, u32>,
    /// Nodes whose SHA shares its first eight bytes with one in `index`.
    collided: HashMap<[u8; 20], u32>,
    /// Commits laid out whose parent, by SHA, has not arrived yet: the parent's SHA and the
    /// children's indices.
    waiting: HashMap<String, Vec<usize>>,
    /// Commits laid out with no parent laid out yet, and their parents' SHAs in commit order: the
    /// first of them to arrive becomes the first parent.
    orphans: HashMap<usize, Vec<String>>,
    /// The next row free below the chunks laid out.
    next_row: i32,
    lanes: Lanes,
    /// Whether lane 0 is the main line's alone (ADR-0032).
    main_line: bool,
    /// The SHA of the next commit on the main line, the first parent of the last one laid out.
    main_next: Option<String>,
}

/// No node: a commit without a parent laid out.
const NO_NODE: u32 = u32::MAX;

/// What the layout keeps of a commit once its chunk is laid out.
#[derive(Debug, Clone, Copy)]
struct Node {
    /// -1 until assigned; 0 is the newest.
    row: i32,
    /// -1 until assigned; 0 is the leftmost.
    col: i32,
    /// The first parent laid out, or [`NO_NODE`]: the branch the commit continues.
    first_parent: u32,
    sha: [u8; 20],
}

/// The chunk being laid out: its commits and, by arena index, each one's parents laid out and
/// children, in the order they were wired.
struct Chunk {
    first: usize,
    commits: Vec<CommitData>,
    parents: Vec<Vec<usize>>,
    children: Vec<Vec<usize>>,
    /// The track of each merge out of the main line, by its parent and child (ADR-0030).
    tracks: HashMap<(usize, usize), i32>,
}

impl Chunk {
    fn indices(&self) -> std::ops::Range<usize> {
        self.first..self.first + self.commits.len()
    }

    fn children_of(&self, i: usize) -> &[usize] {
        &self.children[i - self.first]
    }

    fn commit_time(&self, i: usize) -> i64 {
        self.commits[i - self.first].commit_time
    }

    /// The chunk as one whole graph, the nodes and their edges in arena order; a graph laid out
    /// as a single chunk has every child of its commits among them.
    fn graph(&self, layout: &Layout) -> Graph {
        let nodes = self
            .indices()
            .map(|i| {
                let n = &layout.nodes[i];
                let children = layout
                    .newest_first(self.children_of(i))
                    .into_iter()
                    .map(|c| self.commits[c - self.first].sha.clone())
                    .collect();
                node_json(&self.commits[i - self.first], n.row, n.col, children)
            })
            .collect();
        let edges = self
            .indices()
            .flat_map(|i| {
                self.children_of(i)
                    .iter()
                    .map(move |&c| layout.edge(i, c, self.tracks.get(&(i, c)).copied()))
            })
            .collect();
        Graph { nodes, edges }
    }
}

/// A SHA as twenty bytes: decoded when it is forty lowercase hexadecimal digits, as git writes a
/// commit's, and otherwise the first twenty bytes of its SHA-256, for the working-directory node's
/// sentinel; two SHAs give the same bytes only when they are the same string.
fn sha_bytes(sha: &str) -> [u8; 20] {
    let digit = |b: u8| match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        _ => None,
    };
    let mut out = [0u8; 20];
    let decoded = sha.len() == 40
        && sha
            .as_bytes()
            .chunks(2)
            .zip(out.iter_mut())
            .all(|(pair, byte)| match (digit(pair[0]), digit(pair[1])) {
                (Some(high), Some(low)) => {
                    *byte = high << 4 | low;
                    true
                }
                _ => false,
            });
    if !decoded {
        use sha2::{Digest, Sha256};
        out.copy_from_slice(&Sha256::digest(sha.as_bytes())[..20]);
    }
    out
}

fn index_key(sha: &[u8; 20]) -> u64 {
    u64::from_le_bytes(sha[..8].try_into().expect("eight bytes"))
}

/// A chunk as laid out: its commits in row order and the edges it brings, every edge from one
/// of its commits to a child and every edge down to one of its late children. None of them
/// changes when a later chunk is laid out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaidChunk {
    pub commits: Vec<LaidCommit>,
    pub edges: Vec<Edge>,
    /// The chunk's late children (ADR-0015) as rows of a parent laid out before and of its child
    /// in this chunk: the parent gains a child, and the edge between them is among `edges`.
    pub late_children: Vec<(i32, i32)>,
}

/// A commit as laid out, with what the front ends show of it: its row and lane, the rows of its
/// children, and the commit with its message cut to the first line, the only one shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaidCommit {
    pub commit: CommitData,
    pub row: i32,
    pub col: i32,
    /// The rows of its children, newest first.
    pub children: Vec<i32>,
}

impl LaidCommit {
    /// The commit as the front ends show it; `sha_at` names the commit in a row.
    pub fn node_json<'a>(&self, sha_at: impl Fn(i32) -> &'a str) -> NodeJson {
        let children = self
            .children
            .iter()
            .map(|&r| sha_at(r).to_string())
            .collect();
        node_json(&self.commit, self.row, self.col, children)
    }
}

/// A commit as the front ends show it, at `row` and `col`, with its children's SHAs newest first.
pub(crate) fn node_json(
    commit: &CommitData,
    row: i32,
    col: i32,
    children: Vec<String>,
) -> NodeJson {
    let sha = commit.sha.clone();
    // The working-directory node has no real OID — don't show a short sha.
    let short_sha = if commit.kind == CommitKind::Working {
        String::new()
    } else {
        sha[..sha.len().min(7)].to_string()
    };
    NodeJson {
        row,
        col,
        sha,
        short_sha,
        message: first_line(&commit.message).to_string(),
        author: commit.author_name.clone(),
        author_date: format_commit_time(commit.author_time),
        author_avatar: gravatar_url(&commit.author_email),
        committer: commit.committer_name.clone(),
        date: format_commit_time(commit.commit_time),
        committer_avatar: gravatar_url(&commit.committer_email),
        refs: commit.refs.clone(),
        kind: commit.kind,
        stash: commit.stash,
        parents: commit.parent_shas.clone(),
        children,
    }
}

fn first_line(message: &str) -> &str {
    message.lines().next().unwrap_or("")
}

/// The straight-branch state carried from one chunk to the next.
#[derive(Debug, Default)]
struct Lanes {
    /// `active[j]` holds the commit whose branch line occupies lane j.
    active: Vec<Option<usize>>,
    /// The row at which the occupant of lane j was placed.
    start_row: Vec<i32>,
    /// The lowest row so far at which lane j held a commit or a branch line ended. A lane freed
    /// again is still used in the rows its line ran through, and a merge edge must not pass there.
    last_used_row: Vec<i32>,
}

impl Layout {
    pub fn new() -> Self {
        Self::default()
    }

    /// A layout that keeps lane 0 for the main line from `tip` down (ADR-0032).
    pub fn with_main_line(tip: String) -> Self {
        Self {
            main_line: true,
            main_next: Some(tip),
            ..Self::default()
        }
    }

    /// The node of the commit `sha`, when it is laid out.
    fn find(&self, sha: &str) -> Option<usize> {
        let bytes = sha_bytes(sha);
        if let Some(&i) = self.collided.get(&bytes) {
            return Some(i as usize);
        }
        let i = *self.index.get(&index_key(&bytes))? as usize;
        (self.nodes[i].sha == bytes).then_some(i)
    }

    /// Wires the next chunk and places it below the ones before.
    fn place(&mut self, commits: Vec<CommitData>) -> Chunk {
        let first = self.nodes.len();
        for c in &commits {
            let sha = sha_bytes(&c.sha);
            let i = self.nodes.len() as u32;
            match self.index.entry(index_key(&sha)) {
                Entry::Occupied(_) => {
                    self.collided.insert(sha, i);
                }
                Entry::Vacant(slot) => {
                    slot.insert(i);
                }
            }
            self.nodes.push(Node {
                row: -1,
                col: -1,
                first_parent: NO_NODE,
                sha,
            });
        }
        let n = commits.len();
        let mut chunk = Chunk {
            first,
            commits,
            parents: vec![Vec::new(); n],
            children: vec![Vec::new(); n],
            tracks: HashMap::new(),
        };
        self.wire(&mut chunk);
        let on_main = self.follow_main_line(&chunk);
        temporal_topological_sort(&mut self.nodes, &chunk, &mut self.next_row);
        let main = self.main_line.then_some(on_main.as_slice());
        chunk.tracks = straight_branches(&mut self.nodes, &chunk, &mut self.lanes, main);
        chunk
    }

    /// Lays out the next chunk below the ones before, and returns it as the front ends show it.
    ///
    /// The chunk takes the commits' details along, and the layout keeps only what laying out
    /// the chunks after needs, so the history is held once, by whoever takes the chunk in.
    pub fn lay_out(&mut self, commits: Vec<CommitData>) -> LaidChunk {
        let chunk = self.place(commits);
        let first = chunk.first;
        let mut edges: Vec<Edge> = chunk
            .indices()
            .flat_map(|i| chunk.children_of(i).iter().map(move |&c| (i, c)))
            .map(|(i, c)| self.edge(i, c, chunk.tracks.get(&(i, c)).copied()))
            .collect();
        let mut late_children: Vec<(i32, i32)> = Vec::new();
        for i in chunk.indices() {
            for &p in chunk.parents[i - first].iter().filter(|&&p| p < first) {
                late_children.push((self.nodes[p].row, self.nodes[i].row));
                edges.push(self.edge(p, i, None));
            }
        }
        // Newest child first, as a parent lists its children.
        late_children.sort_unstable_by_key(|&(parent, child)| (child, parent));
        let Chunk {
            commits, children, ..
        } = chunk;
        let mut laid: Vec<LaidCommit> = commits
            .into_iter()
            .zip(children)
            .enumerate()
            .map(|(k, (mut commit, children))| {
                let n = &self.nodes[first + k];
                commit.message = first_line(&commit.message).to_string();
                LaidCommit {
                    commit,
                    row: n.row,
                    col: n.col,
                    children: self
                        .newest_first(&children)
                        .into_iter()
                        .map(|c| self.nodes[c].row)
                        .collect(),
                }
            })
            .collect();
        laid.sort_by_key(|c| c.row);
        LaidChunk {
            commits: laid,
            edges,
            late_children,
        }
    }

    /// Wires the chunk to its parents and its children, as a single chunk has them: each
    /// commit's parents in commit order, the first parent first, and its children in arena order,
    /// children from earlier chunks first and in order, the chunk's own after them.
    fn wire(&mut self, chunk: &mut Chunk) {
        let first = chunk.first;
        // Children laid out in an earlier chunk that wait for a commit of this one as a parent.
        let mut arrived: HashMap<usize, Vec<usize>> = HashMap::new();
        for k in 0..chunk.commits.len() {
            if let Some(children) = self.waiting.remove(&chunk.commits[k].sha) {
                for child in children {
                    chunk.children[k].push(child);
                    arrived.entry(child).or_default().push(first + k);
                }
            }
        }
        // A parent arriving in a later chunk joins its child after the parents laid out with it:
        // the child's lines are drawn already, and its first line stays the branch it was laid
        // out on (ADR-0015). A child with no parent laid out takes the first to arrive in commit
        // order as the branch it continues.
        for (child, parents) in arrived {
            // A main-line commit waits for its first parent alone, so another parent arriving
            // first leaves it waiting (ADR-0032).
            let first_parent = self.orphans.get(&child).and_then(|shas| {
                shas.iter().find_map(|sha| {
                    parents
                        .iter()
                        .copied()
                        .find(|&p| chunk.commits[p - first].sha == *sha)
                })
            });
            if let Some(p) = first_parent {
                self.orphans.remove(&child);
                self.nodes[child].first_parent = p as u32;
            }
        }
        for k in 0..chunk.commits.len() {
            let i = first + k;
            for parent_sha in &chunk.commits[k].parent_shas {
                match self.find(parent_sha) {
                    // A parent laid out before makes this commit a late child (ADR-0015): it is
                    // laid out in its chunk, below its parent.
                    Some(p) => {
                        chunk.parents[k].push(p);
                        if p >= first {
                            chunk.children[p - first].push(i);
                        }
                    }
                    None => self.waiting.entry(parent_sha.clone()).or_default().push(i),
                }
            }
            match chunk.parents[k].first() {
                Some(&p) => self.nodes[i].first_parent = p as u32,
                None if !chunk.commits[k].parent_shas.is_empty() => {
                    self.orphans.insert(i, chunk.commits[k].parent_shas.clone());
                }
                None => {}
            }
        }
    }

    /// Which commits of the chunk are on the main line: the next one expected, and from it each
    /// first parent in commit order found in the chunk. A first parent laid out before its child,
    /// a late child's, ends the main line there (ADR-0032).
    fn follow_main_line(&mut self, chunk: &Chunk) -> Vec<bool> {
        let mut on_main = vec![false; chunk.commits.len()];
        if self.main_next.is_none() {
            return on_main;
        }
        let in_chunk: HashMap<&str, usize> = chunk
            .commits
            .iter()
            .enumerate()
            .map(|(k, c)| (c.sha.as_str(), k))
            .collect();
        while let Some(k) = self
            .main_next
            .as_deref()
            .and_then(|sha| in_chunk.get(sha).copied())
        {
            on_main[k] = true;
            self.main_next = chunk.commits[k].parent_shas.first().cloned();
            // The main line continues through its first parent alone: until that parent is laid
            // out, no other parent is the branch the commit continues, so its edge stays a merge
            // and the main line's edge, once it arrives, a branch edge down lane 0 (ADR-0032).
            let i = chunk.first + k;
            if let Some(parent) = &self.main_next
                && self.find(parent).is_none()
            {
                self.nodes[i].first_parent = NO_NODE;
                self.orphans.insert(i, vec![parent.clone()]);
            }
            let laid_out_before = |sha: &String| self.find(sha).is_some_and(|i| i < chunk.first);
            if self.main_next.as_ref().is_some_and(laid_out_before) {
                self.main_next = None;
            }
        }
        on_main
    }

    /// The given nodes, newest first.
    fn newest_first(&self, nodes: &[usize]) -> Vec<usize> {
        let mut sorted = nodes.to_vec();
        sorted.sort_unstable_by_key(|&c| self.nodes[c].row);
        sorted
    }

    /// The edge from the commit at index `p` to its child at index `c`: a branch edge where `p`
    /// is the child's first parent as laid out, a merge edge otherwise, up `track` where it has one.
    fn edge(&self, p: usize, c: usize, track: Option<i32>) -> Edge {
        let nodes = &self.nodes;
        let (src_row, tgt_row) = (nodes[p].row, nodes[c].row);
        Edge {
            source_row: src_row,
            source_col: nodes[p].col,
            target_row: tgt_row,
            target_col: nodes[c].col,
            is_branch: nodes[c].first_parent == p as u32,
            min_row: src_row.min(tgt_row),
            max_row: src_row.max(tgt_row),
            track,
        }
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

/// Assigns a row to every commit of the chunk, from `next_row` down. A parent always receives a
/// higher row number than each of its children, while commits stay ordered by committer date.
/// The nodes before the chunk are laid out already, so the traversal stops at them; a late
/// child's parent among them stays above it.
fn temporal_topological_sort(nodes: &mut [Node], chunk: &Chunk, next_row: &mut i32) {
    // Sort the chunk's indices by commit_time descending (newest first)
    let mut by_date_desc: Vec<usize> = chunk.indices().collect();
    by_date_desc.sort_by_key(|&i| std::cmp::Reverse(chunk.commit_time(i)));

    // Only the chunk's commits are explored: the chunks before have their rows, which stay.
    let mut explored = vec![false; chunk.commits.len()];
    // Reused across every start commit: allocating it per start would make the
    // pass O(n · starts) instead of the near-linear cost ADR-0015 states. Safe to
    // share, because a completed traversal leaves a row assigned on every node it
    // marked, and only nodes still lacking a row are ever pushed again.
    let mut expanded = vec![false; chunk.commits.len()];

    for &start_idx in &by_date_desc {
        if !explored[start_idx - chunk.first] {
            dfs_mark_explored(chunk, start_idx, &mut explored);
            assign_counter(nodes, chunk, start_idx, next_row, &mut expanded);
        }
    }
}

/// The chunk's own children of a commit of the chunk.
fn children_in_chunk(chunk: &Chunk, i: usize) -> impl Iterator<Item = usize> + '_ {
    chunk
        .children_of(i)
        .iter()
        .copied()
        .filter(|&c| c >= chunk.first)
}

/// Iterative depth-first traversal that marks every reachable commit of the chunk as explored.
/// Uses an explicit work stack, so deep histories cannot exhaust the call stack.
fn dfs_mark_explored(chunk: &Chunk, start: usize, explored: &mut [bool]) {
    let first = chunk.first;
    let mut stack: Vec<usize> = vec![start];
    while let Some(&top) = stack.last() {
        if !explored[top - first] {
            explored[top - first] = true;
            // Push the children sorted by commit_time ascending, so the newest is processed
            // first when popped
            let mut sorted_children: Vec<usize> = children_in_chunk(chunk, top).collect();
            sorted_children.sort_by_key(|&c| chunk.commit_time(c));
            for child in sorted_children {
                if !explored[child - first] {
                    stack.push(child);
                }
            }
        } else {
            stack.pop();
        }
    }
}

/// Iterative depth-first traversal that assigns rows in post-order, so children
/// receive lower row numbers than their parents.
fn assign_counter(
    nodes: &mut [Node],
    chunk: &Chunk,
    start: usize,
    counter: &mut i32,
    expanded: &mut [bool],
) {
    let first = chunk.first;
    let mut work: Vec<usize> = vec![start];

    while let Some(&top) = work.last() {
        if !expanded[top - first] {
            expanded[top - first] = true;
            let mut unvisited: Vec<usize> = children_in_chunk(chunk, top)
                .filter(|&c| nodes[c].row == -1)
                .collect();
            // Sort newest-first (descending commit_time)
            unvisited.sort_by_key(|&i| std::cmp::Reverse(chunk.commit_time(i)));
            for child in unvisited {
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

/// Assigns a column to every commit of the chunk, so all commits on one branch line share a
/// column and the branch renders as a straight vertical line. The lane state carries on from
/// the chunks before.
///
/// With `main`, which commits of the chunk are on the main line, lane 0 is the main line's alone:
/// its commits take it, and it is forbidden to every other commit (ADR-0032).
///
/// Returns the track each merge out of the main line runs up (ADR-0030).
fn straight_branches(
    nodes: &mut [Node],
    chunk: &Chunk,
    lanes: &mut Lanes,
    main: Option<&[bool]>,
) -> HashMap<(usize, usize), i32> {
    let mut tracks = HashMap::new();
    // Process in row order (row 0 = newest first)
    let mut sorted_indices: Vec<usize> = chunk.indices().collect();
    sorted_indices.sort_by_key(|&i| nodes[i].row);

    let Lanes {
        active: active_branches,
        start_row: col_start_row,
        last_used_row,
    } = lanes;
    // The main line's lane is the first one opened (ADR-0032).
    if main.is_some() && active_branches.is_empty() {
        free_lane(
            active_branches,
            col_start_row,
            last_used_row,
            &HashSet::new(),
        );
    }

    for &c_idx in &sorted_indices {
        let (branch_children, merge_children): (Vec<usize>, Vec<usize>) = chunk
            .children_of(c_idx)
            .iter()
            .partition(|&&t| nodes[t].first_parent == c_idx as u32);

        // Compute forbidden columns J(c)
        let mut forbidden = compute_forbidden(
            nodes,
            &merge_children,
            nodes[c_idx].row,
            active_branches,
            col_start_row,
            last_used_row,
        );
        let on_main = main.map(|m| m[c_idx - chunk.first]);
        if on_main == Some(false) {
            forbidden.insert(0);
        }

        // Find valid branch target to replace (leftmost non-forbidden)
        let replaced = if on_main == Some(true) {
            branch_children.iter().copied().find(|&d| nodes[d].col == 0)
        } else {
            branch_children
                .iter()
                .filter(|&&d| nodes[d].col >= 0 && !forbidden.contains(&(nodes[d].col as usize)))
                .min_by_key(|&&d| nodes[d].col)
                .copied()
        };

        if on_main == Some(true) {
            active_branches[0] = Some(c_idx);
            col_start_row[0] = nodes[c_idx].row;
            nodes[c_idx].col = 0;
            // Each merge out of the main line runs up a track of its own: the first lane from 1
            // on that J(c) leaves free over its rows, used over them from now on (ADR-0030).
            for &t in &merge_children {
                let mut taken = compute_forbidden(
                    nodes,
                    &[t],
                    nodes[c_idx].row,
                    active_branches,
                    col_start_row,
                    last_used_row,
                );
                taken.insert(0);
                let track = free_lane(active_branches, col_start_row, last_used_row, &taken);
                last_used_row[track] = nodes[c_idx].row;
                tracks.insert((c_idx, t), track as i32);
            }
        } else if let Some(r_idx) = replaced {
            let col = nodes[r_idx].col as usize;
            active_branches[col] = Some(c_idx);
            col_start_row[col] = nodes[c_idx].row;
            nodes[c_idx].col = col as i32;
        } else {
            // Find first free non-forbidden slot, or append
            let insert_col = free_lane(active_branches, col_start_row, last_used_row, &forbidden);
            active_branches[insert_col] = Some(c_idx);
            col_start_row[insert_col] = nodes[c_idx].row;
            nodes[c_idx].col = insert_col as i32;
        }

        let c_row = nodes[c_idx].row;
        last_used_row[nodes[c_idx].col as usize] = c_row;
        // A late child's first parent stands above it (ADR-0015): its line runs up to that parent,
        // not down, so its lane is free below it.
        let first_parent = nodes[c_idx].first_parent;
        if first_parent != NO_NODE && (first_parent as usize) < chunk.first {
            active_branches[nodes[c_idx].col as usize] = None;
        }

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
    tracks
}

/// The first lane no branch line occupies and outside `taken`, a new one opened where none is.
fn free_lane(
    active: &mut Vec<Option<usize>>,
    start_row: &mut Vec<i32>,
    last_used_row: &mut Vec<i32>,
    taken: &HashSet<usize>,
) -> usize {
    (0..active.len())
        .find(|&j| active[j].is_none() && !taken.contains(&j))
        .unwrap_or_else(|| {
            active.push(None);
            start_row.push(0);
            last_used_row.push(-1);
            active.len() - 1
        })
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

// ============================================================================
// Build edges
// ============================================================================

// ============================================================================
// Tests
// ============================================================================
//
// The specification requires row assignment, column assignment and edge building
// to be covered by tests that need no repository window and no running UI. These
// operate on plain `CommitData` fixtures and assert on the laid-out graph.

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::model::Run;

    /// Fixture commit. `time` is the committer date; parents are listed
    /// first-parent first, exactly as git orders them.
    pub(crate) fn commit(sha: &str, time: i64, parents: &[&str]) -> CommitData {
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
            stash: None,
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
    // Verifies: ADR-0015, G-5
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
    // Verifies: ADR-0015, G-5
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

    /// G-8: the same input always yields the same layout.
    #[test]
    // Verifies: ADR-0015, G-8
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
    /// what makes a chunk whose parents are still to come safe.
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
    // Verifies: ADR-0015, G-9
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
        e.route()
            .into_iter()
            .filter_map(|run| match run {
                Run::Down { lane, from, to } => Some((from + 1..to).map(move |row| (row, lane))),
                Run::Along { .. } => None,
            })
            .flatten()
            .collect()
    }

    /// A merge edge must not run vertically through an unrelated commit.
    ///
    /// `typo` forks `api` off main, and `merge` (on main) takes `login` as
    /// its second parent. `api` sits between `merge` and `login` and briefly
    /// occupies column 1; once its line ended there, column 1 counted as free
    /// for `login` although the merge edge from `login` runs up column 1 right
    /// through `api`. J(c) must include every column used between the highest
    /// merge child and `c` — as in Pvigier's reference implementation — not
    /// only the columns still active.
    #[test]
    // Verifies: ADR-0015, ADR-0030, G-4
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
    // Verifies: ADR-0015, ADR-0030, G-4
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

    // ── Chunk by chunk (ADR-0015) ────────────────────────────────────────────

    /// A generated history, newest first: every commit before its parents, as the walk yields
    /// them, so that consecutive slices are chunks.
    pub(crate) fn generated_history(seed: u64, len: usize) -> Vec<CommitData> {
        let mut state = seed.max(1);
        let mut next = move |n: usize| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state % n as u64) as usize
        };
        let mut commits = vec![commit("c0", 0, &[])];
        let mut tips = vec!["c0".to_string()];
        for i in 1..len {
            let t = next(tips.len());
            let mut parents = vec![tips[t].clone()];
            if tips.len() > 1 && next(4) == 0 {
                let o = (t + 1 + next(tips.len() - 1)) % tips.len();
                parents.push(tips[o].clone());
            }
            let sha = format!("c{i}");
            let refs: Vec<&str> = parents.iter().map(String::as_str).collect();
            commits.push(commit(&sha, i as i64, &refs));
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
        commits.reverse();
        commits
    }

    /// A generated history in which dates run backwards: five commits the walk yields only some
    /// commits later than it would, after a parent of theirs where the move passes one (ADR-0016).
    pub(crate) fn skewed_history(seed: u64, len: usize) -> Vec<CommitData> {
        let mut history = generated_history(seed, len);
        let mut state = seed;
        for _ in 0..5 {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            let from = (state >> 33) as usize % (len - 10);
            let to = (from + 1 + (state >> 40) as usize % 40).min(len - 1);
            let moved = history.remove(from);
            history.insert(to, moved);
        }
        history
    }

    /// A layout fed chunk by chunk and read back as one graph, as the tests compare it: every
    /// commit with its children, late children last, and every edge.
    #[derive(Default)]
    pub(crate) struct Chunked {
        layout: Layout,
        chunks: Vec<LaidChunk>,
    }

    impl Chunked {
        pub(crate) fn new() -> Self {
            Self::default()
        }

        pub(crate) fn add_chunk(&mut self, commits: Vec<CommitData>) -> &LaidChunk {
            let laid = self.layout.lay_out(commits);
            self.chunks.push(laid);
            self.chunks.last().expect("just pushed")
        }

        pub(crate) fn graph(&self) -> Graph {
            let laid = || self.chunks.iter().flat_map(|c| &c.commits);
            let sha_at: HashMap<i32, &str> =
                laid().map(|c| (c.row, c.commit.sha.as_str())).collect();
            let mut late: HashMap<i32, Vec<i32>> = HashMap::new();
            for &(parent, child) in self.chunks.iter().flat_map(|c| &c.late_children) {
                late.entry(parent).or_default().push(child);
            }
            let mut nodes: Vec<NodeJson> = laid()
                .map(|c| {
                    let mut node = c.node_json(|r| sha_at[&r]);
                    for child in late.get(&c.row).into_iter().flatten() {
                        node.children.push(sha_at[child].to_string());
                    }
                    node
                })
                .collect();
            nodes.sort_by_key(|n| n.row);
            let edges = self.chunks.iter().flat_map(|c| c.edges.clone()).collect();
            Graph { nodes, edges }
        }
    }

    /// Each commit's row and lane, by SHA.
    fn places(g: &Graph) -> HashMap<String, (i32, i32)> {
        g.nodes
            .iter()
            .map(|n| (n.sha.clone(), (n.row, n.col)))
            .collect()
    }

    /// An edge's two ends, as (row, lane) each.
    type Ends = ((i32, i32), (i32, i32));

    /// Each edge between two commits of `among`, by its ends, with its type.
    fn edge_types(g: &Graph, among: &HashSet<(i32, i32)>) -> HashMap<Ends, bool> {
        g.edges
            .iter()
            .filter(|e| {
                among.contains(&(e.source_row, e.source_col))
                    && among.contains(&(e.target_row, e.target_col))
            })
            .map(|e| {
                (
                    ((e.source_row, e.source_col), (e.target_row, e.target_col)),
                    e.is_branch,
                )
            })
            .collect()
    }

    #[test]
    // Verifies: ADR-0015
    fn the_first_chunk_lays_out_exactly_as_it_does_alone() {
        for seed in 1..60u64 {
            let history = generated_history(seed, 150);
            let first: Vec<CommitData> = history[..40].to_vec();
            let alone = build_graph(first.clone());

            let mut layout = Chunked::new();
            for chunk in history.chunks(40) {
                layout.add_chunk(chunk.to_vec());
            }
            let all = layout.graph();

            let alone_places = places(&alone);
            let all_places = places(&all);
            for (sha, place) in &alone_places {
                assert_eq!(all_places[sha], *place, "seed {seed}: {sha} moved");
            }
            let cells: HashSet<(i32, i32)> = alone_places.values().copied().collect();
            assert_eq!(
                edge_types(&all, &cells),
                edge_types(&alone, &cells),
                "seed {seed}"
            );
        }
    }

    #[test]
    // Verifies: ADR-0015
    fn adding_a_chunk_moves_nothing_already_laid_out() {
        for seed in 1..60u64 {
            let history = generated_history(seed, 150);
            let mut layout = Chunked::new();
            let mut before: Option<Graph> = None;
            for chunk in history.chunks(25) {
                layout.add_chunk(chunk.to_vec());
                let now = layout.graph();
                if let Some(before) = &before {
                    let then = places(before);
                    let now_places = places(&now);
                    for (sha, place) in &then {
                        assert_eq!(now_places[sha], *place, "seed {seed}: {sha} moved");
                    }
                    let cells: HashSet<(i32, i32)> = then.values().copied().collect();
                    assert_eq!(
                        edge_types(&now, &cells),
                        edge_types(before, &cells),
                        "seed {seed}"
                    );
                }
                before = Some(now);
            }
        }
    }

    #[test]
    // Verifies: ADR-0015
    fn a_first_parent_arriving_later_joins_as_a_further_parent() {
        // `m` merges `topic` into a main line whose parent `main` lies in the next chunk.
        let mut layout = Chunked::new();
        layout.add_chunk(vec![
            commit("m", 3, &["main", "topic"]),
            commit("topic", 2, &[]),
        ]);
        let before = layout.graph();
        layout.add_chunk(vec![commit("main", 1, &[])]);
        let after = layout.graph();

        let topic_edge = |g: &Graph| {
            let (m, t) = (row_of(g, "m"), row_of(g, "topic"));
            g.edges
                .iter()
                .find(|e| e.target_row == m && e.source_row == t)
                .unwrap()
                .is_branch
        };
        assert!(
            topic_edge(&before),
            "laid out alone, topic continues m's line"
        );
        assert!(topic_edge(&after), "it stays the branch m was drawn on");
        let main_edge = after
            .edges
            .iter()
            .find(|e| e.target_row == row_of(&after, "m") && e.source_row == row_of(&after, "main"))
            .unwrap();
        assert!(
            !main_edge.is_branch,
            "the late first parent joins as a merge"
        );
        assert!(row_of(&after, "main") > row_of(&after, "topic"));
    }

    #[test]
    // Verifies: ADR-0015
    fn a_late_child_is_laid_out_below_its_parent_without_moving_anything() {
        // `late` sits on `p` but is dated before it, so the walk yields it in the next chunk.
        let mut layout = Layout::new();
        let first = layout.lay_out(vec![commit("tip", 4, &["p"]), commit("p", 3, &["base"])]);
        let second = layout.lay_out(vec![commit("late", 1, &["p"]), commit("base", 0, &[])]);

        let rows: Vec<(String, i32)> = first
            .commits
            .iter()
            .map(|c| (c.commit.sha.clone(), c.row))
            .collect();
        assert_eq!(
            rows,
            [("tip".to_string(), 0), ("p".to_string(), 1)],
            "the first chunk as laid out alone"
        );
        let late = second
            .commits
            .iter()
            .find(|c| c.commit.sha == "late")
            .unwrap();
        assert!(late.row > 1, "below its parent");
        assert_eq!(second.late_children, [(1, late.row)]);
        let edge = second
            .edges
            .iter()
            .find(|e| e.source_row == 1 && e.target_row == late.row)
            .expect("an edge from the parent down to its late child");
        assert!(edge.is_branch, "p is its first parent");
        assert_eq!((edge.min_row, edge.max_row), (1, late.row));
    }

    #[test]
    // Verifies: ADR-0015, G-2
    fn a_late_child_frees_its_lane_below_it() {
        // `late` on `p` is laid out in the next chunk; `side`, a tip below it, takes the lane
        // the late child leaves behind.
        let mut layout = Layout::new();
        layout.lay_out(vec![commit("p", 5, &["q"])]);
        let laid = layout.lay_out(vec![
            commit("late", 4, &["p"]),
            commit("q", 3, &[]),
            commit("side", 2, &[]),
        ]);
        let col = |sha: &str| {
            laid.commits
                .iter()
                .find(|c| c.commit.sha == sha)
                .unwrap()
                .col
        };
        assert_eq!(
            col("side"),
            col("late"),
            "no lane is lost to the late child"
        );
    }

    #[test]
    // Verifies: ADR-0015
    fn late_children_of_one_parent_come_newest_first() {
        let mut layout = Layout::new();
        layout.lay_out(vec![commit("p", 5, &[])]);
        let laid = layout.lay_out(vec![commit("b", 4, &["p"]), commit("a", 3, &["p"])]);
        let row = |sha: &str| {
            laid.commits
                .iter()
                .find(|c| c.commit.sha == sha)
                .unwrap()
                .row
        };
        assert_eq!(laid.late_children, [(0, row("b")), (0, row("a"))]);
        assert!(row("b") < row("a"));
    }

    #[test]
    // Verifies: ADR-0015
    fn a_chunk_laid_out_reads_back_as_the_graph_built_whole() {
        // The chunks the front ends take in carry what build_graph returns: every node with its
        // children and every edge, whichever way a single chunk is read.
        for seed in 1..40u64 {
            let commits = generated_history(seed, 120);
            let whole = build_graph(commits.clone());
            let mut chunked = Chunked::new();
            chunked.add_chunk(commits);
            let read = chunked.graph();
            let mut nodes = whole.nodes.clone();
            nodes.sort_by_key(|n| n.row);
            assert_eq!(read.nodes, nodes, "seed {seed}: nodes");
            let ends = |g: &Graph| {
                let mut e: Vec<_> = g
                    .edges
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
                    .collect();
                e.sort_unstable();
                e
            };
            assert_eq!(ends(&read), ends(&whole), "seed {seed}: edges");
        }
    }

    #[test]
    // Verifies: ADR-0015
    fn a_sha_is_found_only_as_the_same_string() {
        let lower = "0123456789abcdef0123456789abcdef01234567";
        let upper = lower.to_uppercase();
        let mut layout = Layout::new();
        layout.lay_out(vec![commit(lower, 1, &[])]);
        assert_eq!(layout.find(lower), Some(0));
        assert_eq!(layout.find(&upper), None, "git writes SHAs in lowercase");
        assert_eq!(layout.find("WORKING_DIRECTORY"), None);
    }

    /// Lays `history` out in chunks of `size` with the main line from its first commit, the
    /// newest, and returns each commit's place and the main line as followed by hand: from the
    /// tip, each first parent, until one is laid out in a chunk before its child's.
    fn with_main_line(history: &[CommitData], size: usize) -> (Chunked, HashSet<String>) {
        let tip = history[0].sha.clone();
        let mut chunked = Chunked {
            layout: Layout::with_main_line(tip.clone()),
            chunks: Vec::new(),
        };
        for chunk in history.chunks(size) {
            chunked.add_chunk(chunk.to_vec());
        }
        let chunk_of: HashMap<&str, usize> = history
            .iter()
            .enumerate()
            .map(|(i, c)| (c.sha.as_str(), i / size))
            .collect();
        let by_sha: HashMap<&str, &CommitData> =
            history.iter().map(|c| (c.sha.as_str(), c)).collect();
        let mut main = HashSet::new();
        let mut at = Some(tip);
        while let Some(sha) = at {
            main.insert(sha.clone());
            at = by_sha[sha.as_str()]
                .parent_shas
                .first()
                .filter(|p| {
                    chunk_of
                        .get(p.as_str())
                        .is_some_and(|&c| c >= chunk_of[sha.as_str()])
                })
                .cloned();
        }
        (chunked, main)
    }

    #[test]
    // Verifies: ADR-0032, ADR-0015
    fn a_main_line_whose_tip_arrives_later_keeps_the_lane_free_above_it() {
        // Newer work on a side branch comes first; the main line's tip only in the next chunk.
        let mut chunked = Chunked {
            layout: Layout::with_main_line("m2".into()),
            chunks: Vec::new(),
        };
        chunked.add_chunk(vec![commit("f2", 5, &["f1"]), commit("f1", 4, &["m1"])]);
        chunked.add_chunk(vec![commit("m2", 3, &["m1"]), commit("m1", 2, &[])]);
        let g = chunked.graph();
        let col = |sha: &str| g.nodes.iter().find(|n| n.sha == sha).unwrap().col;
        assert_eq!((col("m2"), col("m1")), (0, 0));
        assert!(
            col("f2") > 0 && col("f1") > 0,
            "the lane is free above the tip"
        );
    }

    #[test]
    // Verifies: ADR-0015, ADR-0032, ADR-0030, G-3, G-4, G-5
    fn every_layout_keeps_its_rows_parents_edges_and_main_line() {
        // Over both generators, in one chunk and in several, with and without a main line: rows
        // run 0..n; a parent stands below its children and no edge runs through a commit, but
        // for the edge down to a late child, which G-4 and G-5 except; with a main line, lane 0
        // holds its commits alone and each of its edges runs straight down lane 0.
        let (mut late_children, mut main_edges) = (0, 0);
        for seed in 1..60u64 {
            for (history, size) in [
                (generated_history(seed, 150), 150),
                (generated_history(seed, 150), 20),
                (skewed_history(seed, 150), 25),
            ] {
                for main_line in [false, true] {
                    let (chunked, main) = if main_line {
                        let (chunked, main) = with_main_line(&history, size);
                        (chunked, Some(main))
                    } else {
                        let mut chunked = Chunked::new();
                        for chunk in history.chunks(size) {
                            chunked.add_chunk(chunk.to_vec());
                        }
                        (chunked, None)
                    };
                    let case = format!("seed {seed}, chunks of {size}, main line {main_line}");
                    let late: HashSet<i32> = chunked
                        .chunks
                        .iter()
                        .flat_map(|c| c.late_children.iter().map(|&(_, child)| child))
                        .collect();
                    late_children += late.len();
                    let g = chunked.graph();
                    let mut rows: Vec<i32> = g.nodes.iter().map(|n| n.row).collect();
                    rows.sort_unstable();
                    assert_eq!(rows, (0..150).collect::<Vec<_>>(), "{case}: rows");
                    let at: HashMap<(i32, i32), &str> = g
                        .nodes
                        .iter()
                        .map(|n| ((n.row, n.col), n.sha.as_str()))
                        .collect();
                    for e in &g.edges {
                        if e.source_row < e.target_row {
                            assert!(late.contains(&e.target_row), "{case}: {e:?}");
                            continue;
                        }
                        for cell in vertical_cells(e) {
                            assert!(
                                !at.contains_key(&cell),
                                "{case}: {e:?} runs through {}",
                                at[&cell]
                            );
                        }
                    }
                    let Some(main) = main else { continue };
                    let node = |sha: &str| g.nodes.iter().find(|n| n.sha == sha).unwrap();
                    for n in &g.nodes {
                        assert_eq!(
                            n.col == 0,
                            main.contains(&n.sha),
                            "{case}: {} in lane {}",
                            n.sha,
                            n.col
                        );
                    }
                    for n in g.nodes.iter().filter(|n| main.contains(&n.sha)) {
                        let Some(parent) = n.parents.first().filter(|p| main.contains(*p)) else {
                            continue;
                        };
                        let p = node(parent);
                        let edge = g
                            .edges
                            .iter()
                            .find(|e| e.source_row == p.row && e.target_row == n.row)
                            .unwrap();
                        assert!(
                            edge.is_branch && edge.track.is_none(),
                            "{case}: the main line's edge {} -> {} is {edge:?}",
                            p.sha,
                            n.sha
                        );
                        main_edges += 1;
                    }
                }
            }
        }
        assert!(late_children > 0, "the histories hold late children");
        assert!(main_edges > 0, "the histories hold main lines");
    }

    #[test]
    // Verifies: ADR-0032, ADR-0015
    fn a_main_line_parent_arriving_after_another_keeps_the_main_line_straight() {
        // `m` on the main line merges `topic`, laid out with it, and its first parent `main`
        // arrives in the next chunk: the main line's edge runs down lane 0, and topic's edge
        // stays a merge into it.
        let mut chunked = Chunked {
            layout: Layout::with_main_line("m".into()),
            chunks: Vec::new(),
        };
        chunked.add_chunk(vec![
            commit("m", 4, &["main", "topic"]),
            commit("topic", 3, &[]),
        ]);
        chunked.add_chunk(vec![commit("main", 1, &[])]);
        let g = chunked.graph();
        let row = |sha: &str| g.nodes.iter().find(|n| n.sha == sha).unwrap().row;
        let edge = |from: &str| {
            g.edges
                .iter()
                .find(|e| e.source_row == row(from) && e.target_row == row("m"))
                .unwrap()
        };
        let main_edge = edge("main");
        assert!(main_edge.is_branch && main_edge.track.is_none());
        assert_eq!((main_edge.source_col, main_edge.target_col), (0, 0));
        assert!(!edge("topic").is_branch, "topic's edge stays a merge");
    }
}
