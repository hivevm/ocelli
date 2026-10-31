//! The laid-out commit graph as the core keeps it (ADR-0019). The history grows chunk by chunk
//! (ADR-0013), and the front ends read from it only what they draw, a window of rows with the
//! edges that cross it, and ask it to search and to compute lineage, so that none of them needs
//! the whole graph.

use crate::graph::{LaidChunk, LaidCommit, Layout, node_json};
use crate::model::{CommitData, CommitKind, Edge, NodeJson};
use std::collections::{HashMap, HashSet};

/// The rows of a bucket of the first level of edges, which holds the edges spanning at most as
/// many rows; every level up doubles both.
const LEVEL_ROWS: usize = 64;

/// The rows `top..top + nodes.len()` of the history and every edge whose span crosses them,
/// including edges with both ends outside the window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Window {
    pub top: usize,
    /// The nodes of the window's rows, in row order.
    pub nodes: Vec<NodeJson>,
    pub edges: Vec<Edge>,
}

impl Window {
    /// The lanes the window's nodes and edges reach. A front end that sizes its graph column to
    /// the most lanes it has shown keeps the column steady while older history arrives, widening
    /// it only when the developer scrolls to wider rows.
    pub fn lanes(&self) -> usize {
        let nodes = self.nodes.iter().map(|n| n.col as usize + 1);
        nodes
            .chain(self.edges.iter().map(Edge::lanes))
            .max()
            .unwrap_or(0)
    }
}

/// A commit's lineage (G-10): the rows of the commit and its ancestors, and of the commit and its
/// descendants, kept apart so the lineage can grow with the chunks laid out after it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Lineage {
    ancestors: RowSet,
    descendants: RowSet,
}

impl Lineage {
    pub fn contains(&self, row: usize) -> bool {
        self.ancestors.contains(row) || self.descendants.contains(row)
    }

    /// Every row of the lineage.
    pub fn rows(&self) -> HashSet<usize> {
        self.ancestors
            .iter()
            .chain(self.descendants.iter())
            .collect()
    }
}

/// A set of rows, one bit per row of the history, so a lineage of a million rows takes a few
/// hundred kilobytes and a lookup no hashing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct RowSet(Vec<u64>);

impl RowSet {
    /// An empty set sized for `rows` rows.
    fn new(rows: usize) -> Self {
        RowSet(vec![0; rows.div_ceil(64)])
    }

    /// Sizes the set for `rows` rows.
    fn grow(&mut self, rows: usize) {
        let words = rows.div_ceil(64);
        if words > self.0.len() {
            self.0.resize(words, 0);
        }
    }

    fn contains(&self, row: usize) -> bool {
        self.0
            .get(row / 64)
            .is_some_and(|w| w >> (row % 64) & 1 == 1)
    }

    /// Adds `row`; whether it was not in the set before.
    fn insert(&mut self, row: usize) -> bool {
        self.grow(row + 1);
        let (word, bit) = (&mut self.0[row / 64], 1u64 << (row % 64));
        let added = *word & bit == 0;
        *word |= bit;
        added
    }

    fn iter(&self) -> impl Iterator<Item = usize> + '_ {
        self.0.iter().enumerate().flat_map(|(i, &w)| {
            (0..64)
                .filter(move |b| w >> b & 1 == 1)
                .map(move |b| i * 64 + b)
        })
    }
}

/// The end of a row's list of parents.
const NO_LINK: u32 = u32::MAX;

/// The commit graph laid out so far, with the lookups its reads need. A chunk added never moves
/// what is laid out before it (ADR-0013), so each lookup only grows.
///
/// The history is kept compact, for a history of a million commits: the text of a chunk's
/// commits lies in one buffer, each row holds spans into it, and a node is built only when it is
/// read. A chunk then costs a few allocations instead of several per commit, which keeps both
/// the memory a history holds and what the allocator cannot hand back afterwards small (Q-3).
#[derive(Debug, Default)]
pub struct History {
    layout: Layout,
    /// One buffer per chunk with the text of its commits.
    texts: Vec<String>,
    /// `rows[row]` is the commit in that row.
    rows: Vec<Row>,
    /// The spans of every commit's refs and parent SHAs, which `Row::refs` and `Row::parents`
    /// index.
    spans: Vec<Span>,
    /// The rows of every commit's children, which `Row::children` indexes.
    children: Vec<u32>,
    /// Children laid out after their parent, late children (ADR-0013), by their parent's row.
    late: HashMap<u32, Vec<u32>>,
    edges: Vec<Edge>,
    /// Each row's first link to a parent laid out, or [`NO_LINK`]; a link holds a parent's row and
    /// the row's next link, so finding parents takes no hashing and no allocation per row.
    parent_head: Vec<u32>,
    parent_links: Vec<(u32, u32)>,
    /// The edges by length and place: level `k` holds the edges spanning at most
    /// `LEVEL_ROWS << k` rows, in buckets of as many rows by their first (lowest) row. A window
    /// looks at the buckets of each level near it, and a bucket holds few edges that miss it,
    /// since every edge of a level runs at least half its length, a lane each.
    edge_levels: Vec<Vec<Vec<u32>>>,
    /// Rows by a 64-bit hash of their SHA; a SHA whose hash an earlier one holds already goes
    /// into `collided` instead.
    row_of: HashMap<u64, u32>,
    collided: HashMap<String, u32>,
    /// The place in the stash of each stash node, by its row; few rows have one, so they stay
    /// out of `Row`.
    stashes: HashMap<u32, usize>,
    lanes: usize,
}

/// A run of `len` items from `start`: bytes of a chunk's text, or entries of a shared list.
#[derive(Debug, Clone, Copy, Default)]
struct Span {
    start: u32,
    len: u32,
}

impl Span {
    fn range(self) -> std::ops::Range<usize> {
        self.start as usize..(self.start + self.len) as usize
    }
}

/// One commit, its text as spans of its chunk's buffer.
#[derive(Debug)]
struct Row {
    chunk: u32,
    sha: Span,
    message: Span,
    author: Span,
    author_email: Span,
    committer: Span,
    committer_email: Span,
    author_time: i64,
    commit_time: i64,
    kind: CommitKind,
    col: i32,
    /// Spans in `History::spans`, each a ref name in the chunk's buffer.
    refs: Span,
    /// Spans in `History::spans`, each a parent SHA in the chunk's buffer.
    parents: Span,
    /// Rows in `History::children`, newest first.
    children: Span,
}

fn hash(sha: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::hash::DefaultHasher::new();
    sha.hash(&mut h);
    h.finish()
}

impl History {
    pub fn new() -> Self {
        Self::default()
    }

    /// Lays out the next chunk below the rows laid out before.
    pub fn add_chunk(&mut self, commits: Vec<CommitData>) {
        let chunk = self.layout.lay_out(commits);
        self.append(chunk);
    }

    /// Takes in the next chunk laid out by a [`Layout`] of its own, as a front end does that lays
    /// out on another thread than the one reading. Its commits fill the rows below.
    pub fn append(&mut self, chunk: LaidChunk) {
        let chunk_index = self.texts.len() as u32;
        let size: usize = chunk.commits.iter().map(text_len).sum();
        // Spans are 32 bits wide: a chunk of `CHUNK_COMMITS` commits stays far below 4 GB of text.
        assert!(u32::try_from(size).is_ok(), "a chunk's text exceeds 4 GB");
        let mut text = String::with_capacity(size);
        let mut put = |s: &str| {
            let span = Span {
                start: text.len() as u32,
                len: s.len() as u32,
            };
            text.push_str(s);
            span
        };
        let first_row = self.rows.len();
        self.rows.reserve(chunk.commits.len());
        for (k, laid) in chunk.commits.into_iter().enumerate() {
            let row = first_row + k;
            debug_assert_eq!(laid.row as usize, row, "a chunk fills the rows below");
            let c = laid.commit;
            let refs = Span {
                start: self.spans.len() as u32,
                len: c.refs.len() as u32,
            };
            self.spans.extend(c.refs.iter().map(|r| put(r)));
            let parents = Span {
                start: self.spans.len() as u32,
                len: c.parent_shas.len() as u32,
            };
            self.spans.extend(c.parent_shas.iter().map(|p| put(p)));
            let children = Span {
                start: self.children.len() as u32,
                len: laid.children.len() as u32,
            };
            self.children
                .extend(laid.children.iter().map(|&r| r as u32));
            if let Some(place) = c.stash {
                self.stashes.insert(row as u32, place);
            }
            self.rows.push(Row {
                chunk: chunk_index,
                sha: put(&c.sha),
                message: put(&c.message),
                author: put(&c.author_name),
                author_email: put(&c.author_email),
                committer: put(&c.committer_name),
                committer_email: put(&c.committer_email),
                author_time: c.author_time,
                commit_time: c.commit_time,
                kind: c.kind,
                col: laid.col,
                refs,
                parents,
                children,
            });
            self.lanes = self.lanes.max(laid.col as usize + 1);
        }
        self.texts.push(text);
        for (parent, child) in chunk.late_children {
            self.late
                .entry(parent as u32)
                .or_default()
                .push(child as u32);
        }
        for row in first_row..self.rows.len() {
            let sha = self.sha(row);
            let h = hash(sha);
            match self.row_of.get(&h) {
                Some(_) => {
                    self.collided.insert(sha.to_string(), row as u32);
                }
                None => {
                    self.row_of.insert(h, row as u32);
                }
            }
        }
        self.parent_head.resize(self.rows.len(), NO_LINK);
        for e in chunk.edges {
            self.lanes = self.lanes.max(e.lanes());
            let (parent, child) = (e.source_row as usize, e.target_row as usize);
            self.parent_links
                .push((parent as u32, self.parent_head[child]));
            self.parent_head[child] = (self.parent_links.len() - 1) as u32;
            let span = (e.max_row - e.min_row) as usize;
            let level = span
                .div_ceil(LEVEL_ROWS)
                .max(1)
                .next_power_of_two()
                .trailing_zeros();
            let level = level as usize;
            if self.edge_levels.len() <= level {
                self.edge_levels.resize(level + 1, Vec::new());
            }
            let buckets = &mut self.edge_levels[level];
            let bucket = e.min_row as usize / (LEVEL_ROWS << level);
            if buckets.len() <= bucket {
                buckets.resize(bucket + 1, Vec::new());
            }
            buckets[bucket].push(self.edges.len() as u32);
            self.edges.push(e);
        }
    }

    /// The number of rows laid out.
    pub fn rows(&self) -> usize {
        self.rows.len()
    }

    /// The number of lanes the rows laid out use.
    pub fn lanes(&self) -> usize {
        self.lanes
    }

    fn text(&self, row: &Row, span: Span) -> &str {
        &self.texts[row.chunk as usize][span.range()]
    }

    fn texts_of(&self, row: &Row, list: Span) -> impl Iterator<Item = &str> {
        self.spans[list.range()]
            .iter()
            .map(move |&span| self.text(row, span))
    }

    /// The rows of the commit's children, newest first, its late children last.
    fn children_of(&self, row: usize) -> impl Iterator<Item = usize> + '_ {
        let late = self.late.get(&(row as u32)).map_or(&[][..], Vec::as_slice);
        self.children[self.rows[row].children.range()]
            .iter()
            .chain(late)
            .map(|&r| r as usize)
    }

    /// The SHA of the commit in `row`.
    pub fn sha(&self, row: usize) -> &str {
        let r = &self.rows[row];
        self.text(r, r.sha)
    }

    /// The node in `row`.
    pub fn node(&self, row: usize) -> NodeJson {
        let r = &self.rows[row];
        let text = |span| self.text(r, span).to_string();
        let commit = CommitData {
            sha: text(r.sha),
            message: text(r.message),
            author_name: text(r.author),
            author_email: text(r.author_email),
            author_time: r.author_time,
            committer_name: text(r.committer),
            committer_email: text(r.committer_email),
            commit_time: r.commit_time,
            refs: self.texts_of(r, r.refs).map(str::to_string).collect(),
            parent_shas: self.texts_of(r, r.parents).map(str::to_string).collect(),
            kind: r.kind,
            stash: self.stashes.get(&(row as u32)).copied(),
        };
        let children = self
            .children_of(row)
            .map(|c| self.sha(c).to_string())
            .collect();
        node_json(&commit, row as i32, r.col, children)
    }

    /// The label of a link to the commit `sha`, a parent or a child: its short SHA, or for the
    /// working-directory node, whose short SHA is empty, its name (ADR-0018). A commit not laid
    /// out is labelled by its SHA.
    pub fn link_label(&self, sha: &str) -> String {
        match self.row_of(sha).map(|row| &self.rows[row]) {
            Some(r) if r.kind == CommitKind::Working => "Uncommitted".to_string(),
            _ => sha[..sha.len().min(7)].to_string(),
        }
    }

    /// The row of the commit `sha`, when it is laid out.
    pub fn row_of(&self, sha: &str) -> Option<usize> {
        if let Some(&row) = self.collided.get(sha) {
            return Some(row as usize);
        }
        let row = *self.row_of.get(&hash(sha))? as usize;
        (self.sha(row) == sha).then_some(row)
    }

    /// Rows `top..top + rows`, cut at the end of the history, with every edge crossing them. Its
    /// cost follows the rows asked for and the lanes, not the length of the history.
    pub fn window(&self, top: usize, rows: usize) -> Window {
        let end = top.saturating_add(rows).min(self.rows.len());
        if top >= end {
            return Window {
                top,
                nodes: Vec::new(),
                edges: Vec::new(),
            };
        }
        let bottom = end - 1;
        let edges = self
            .edges_near(top, bottom)
            .filter(|e| e.max_row as usize >= top && e.min_row as usize <= bottom)
            .cloned()
            .collect();
        Window {
            top,
            nodes: (top..end).map(|row| self.node(row)).collect(),
            edges,
        }
    }

    /// The edges a window from `top` to `bottom` looks at: on each level, those of the buckets
    /// from one bucket above `top`, where an edge reaching `top` starts at the earliest, down to
    /// the bucket of `bottom`.
    fn edges_near(&self, top: usize, bottom: usize) -> impl Iterator<Item = &Edge> + '_ {
        self.edge_levels
            .iter()
            .enumerate()
            .flat_map(move |(level, buckets)| {
                let rows = LEVEL_ROWS << level;
                let from = top.saturating_sub(rows) / rows;
                let to = (bottom / rows + 1).min(buckets.len());
                buckets.get(from..to).unwrap_or_default().iter().flatten()
            })
            .map(|&k| &self.edges[k as usize])
    }

    /// Rows of the commits matching `query`, newest first (G-11).
    ///
    /// A case-insensitive substring test over what a row shows: message, author, committer,
    /// SHA, and ref names. A blank query matches nothing; an empty result means "no search
    /// active".
    pub fn search(&self, query: &str) -> Vec<usize> {
        self.search_from(query, 0)
    }

    /// The rows from `from` on that match `query`: what a search finds in the chunks laid out
    /// after it ran.
    pub fn search_from(&self, query: &str, from: usize) -> Vec<usize> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return Vec::new();
        }
        let matches = |s: &str| s.to_lowercase().contains(&q);
        (from..self.rows.len())
            .filter(|&row| {
                let r = &self.rows[row];
                matches(self.text(r, r.message))
                    || matches(self.text(r, r.author))
                    || matches(self.text(r, r.committer))
                    || matches(self.text(r, r.sha))
                    || self.texts_of(r, r.refs).any(matches)
            })
            .collect()
    }

    /// Rows of the commit in `row` and every ancestor laid out. Parents not laid out (yet) are
    /// skipped.
    pub fn ancestors(&self, row: usize) -> HashSet<usize> {
        let mut rows = RowSet::new(self.rows.len());
        self.walk_up(row, &mut rows);
        rows.iter().collect()
    }

    /// Rows of the commit in `row` and every descendant.
    pub fn descendants(&self, row: usize) -> HashSet<usize> {
        let mut rows = RowSet::new(self.rows.len());
        self.walk_down(row, &mut rows);
        rows.iter().collect()
    }

    /// The commit's lineage: itself, its ancestors, and its descendants (G-10).
    pub fn lineage(&self, row: usize) -> Lineage {
        let mut lineage = Lineage {
            ancestors: RowSet::new(self.rows.len()),
            descendants: RowSet::new(self.rows.len()),
        };
        self.walk_up(row, &mut lineage.ancestors);
        self.walk_down(row, &mut lineage.descendants);
        lineage
    }

    /// Grows a `lineage` by the rows from `from` on, those of the chunks laid out after it was
    /// computed; the cost follows the rows added, not the history.
    ///
    /// A row joins the ancestors when one of its children is an ancestor; its children lie in
    /// rows above it, so one pass in row order finds them, and where a late child (ADR-0013)
    /// joins, its parent laid out before brings its own ancestors along. The rows added hold a
    /// descendant only below a late child of a descendant, which brings its own descendants.
    pub fn extend_lineage(&self, lineage: &mut Lineage, from: usize) {
        lineage.ancestors.grow(self.rows.len());
        lineage.descendants.grow(self.rows.len());
        for row in from..self.rows.len() {
            if self.children_of(row).any(|c| lineage.ancestors.contains(c)) {
                lineage.ancestors.insert(row);
            }
        }
        for (&parent, children) in &self.late {
            for &child in children.iter().filter(|&&c| c as usize >= from) {
                let (parent, child) = (parent as usize, child as usize);
                if lineage.ancestors.contains(child) {
                    self.walk_up(parent, &mut lineage.ancestors);
                }
                if lineage.descendants.contains(parent) {
                    self.walk_down(child, &mut lineage.descendants);
                }
            }
        }
    }

    /// The rows of the commit's parents laid out.
    fn parents_of(&self, row: usize) -> impl Iterator<Item = usize> + '_ {
        let mut link = self.parent_head[row];
        std::iter::from_fn(move || {
            let (parent, next) = *self.parent_links.get(link as usize)?;
            link = next;
            Some(parent as usize)
        })
    }

    /// Adds `start` and every ancestor to `rows`, stopping at rows already in it, whose ancestors
    /// are in it too. Iterative, so a deep history cannot exhaust the call stack.
    fn walk_up(&self, start: usize, rows: &mut RowSet) {
        let mut stack = vec![start];
        while let Some(row) = stack.pop() {
            if rows.insert(row) {
                stack.extend(self.parents_of(row));
            }
        }
    }

    /// Adds `start` and every descendant to `rows`, as [`History::walk_up`] does upwards.
    fn walk_down(&self, start: usize, rows: &mut RowSet) {
        let mut stack = vec![start];
        while let Some(row) = stack.pop() {
            if rows.insert(row) {
                stack.extend(self.children_of(row));
            }
        }
    }
}

/// The bytes of text a laid-out commit puts into its chunk's buffer.
fn text_len(laid: &LaidCommit) -> usize {
    let c = &laid.commit;
    c.sha.len()
        + c.message.len()
        + c.author_name.len()
        + c.author_email.len()
        + c.committer_name.len()
        + c.committer_email.len()
        + c.refs.iter().map(String::len).sum::<usize>()
        + c.parent_shas.iter().map(String::len).sum::<usize>()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::tests::{Chunked, commit, generated_history, skewed_history};
    use crate::model::Graph;

    fn history(chunks: &[Vec<CommitData>]) -> History {
        let mut h = History::new();
        for chunk in chunks {
            h.add_chunk(chunk.clone());
        }
        h
    }

    /// An edge as a comparable key: its ends and its type.
    type Key = (i32, i32, i32, i32, bool);

    fn key(e: &Edge) -> Key {
        (
            e.source_row,
            e.source_col,
            e.target_row,
            e.target_col,
            e.is_branch,
        )
    }

    /// The window as the whole graph answers it, by brute force.
    fn reference(g: &Graph, top: usize, rows: usize) -> (Vec<NodeJson>, Vec<Key>) {
        let end = (top + rows).min(g.nodes.len());
        let mut nodes: Vec<NodeJson> = g
            .nodes
            .iter()
            .filter(|n| (top..end).contains(&(n.row as usize)))
            .cloned()
            .collect();
        nodes.sort_by_key(|n| n.row);
        let mut edges: Vec<_> = g
            .edges
            .iter()
            .filter(|e| top < end && e.max_row as usize >= top && (e.min_row as usize) < end)
            .map(key)
            .collect();
        edges.sort_unstable();
        (nodes, edges)
    }

    #[test]
    // Verifies: ADR-0019, ADR-0020, G-7
    fn a_window_holds_exactly_its_rows_and_every_edge_crossing_them() {
        let mut long = 0;
        for seed in 1..30u64 {
            let commits = generated_history(seed, 400);
            let chunks: Vec<Vec<CommitData>> = commits.chunks(70).map(<[_]>::to_vec).collect();
            let h = history(&chunks);
            let mut layout = Chunked::new();
            for chunk in &chunks {
                layout.add_chunk(chunk.clone());
            }
            let g = layout.graph();
            assert_eq!(h.rows(), g.nodes.len());
            long += h.edge_levels.iter().skip(1).flatten().flatten().count();
            for (top, rows) in [
                (0, 25),
                (37, 1),
                (100, 90),
                (330, 200),
                (399, 5),
                (400, 3),
                (0, 1000),
            ] {
                let w = h.window(top, rows);
                let mut edges: Vec<_> = w.edges.iter().map(key).collect();
                edges.sort_unstable();
                assert_eq!(w.top, top);
                assert_eq!(
                    (w.nodes, edges),
                    reference(&g, top, rows),
                    "seed {seed}, {top}+{rows}"
                );
            }
        }
        assert!(
            long > 0,
            "the histories hold long edges, which windows find apart"
        );
    }

    #[test]
    // Verifies: ADR-0019, ADR-0013
    fn the_history_grows_by_chunks_without_changing_a_row_read_before() {
        let commits = generated_history(7, 300);
        let mut h = History::new();
        let mut before: Option<Window> = None;
        for chunk in commits.chunks(60) {
            h.add_chunk(chunk.to_vec());
            let now = h.window(0, 60);
            if let Some(before) = before {
                assert_eq!(now.nodes, before.nodes, "a row read before changed");
            }
            before = Some(now);
        }
        assert_eq!(h.rows(), 300);
        assert!(h.lanes() >= 1);
        for row in 0..h.rows() {
            assert_eq!(h.row_of(&h.node(row).sha), Some(row));
        }
    }

    // ── Search (G-11) ──────────────────────────────────────────────────────────────────────

    fn found(h: &History, query: &str) -> Vec<String> {
        h.search(query)
            .iter()
            .map(|&r| h.node(r).sha.clone())
            .collect()
    }

    /// deadbee (newest) ← abc1234 ← c0ffee1 (oldest).
    fn searchable() -> History {
        let mut c0ffee1 = commit("c0ffee1", 1, &[]);
        c0ffee1.message = "fix: align the header".into();
        c0ffee1.author_name = "Alice".into();
        let mut abc1234 = commit("abc1234", 2, &["c0ffee1"]);
        abc1234.message = "docs: readme".into();
        abc1234.author_name = "alice".into();
        abc1234.committer_name = "Carol".into();
        let mut deadbee = commit("deadbee", 3, &["abc1234"]);
        deadbee.message = "feat: add search".into();
        deadbee.author_name = "Bob".into();
        deadbee.refs = vec!["feature/search".into()];
        history(&[vec![deadbee, abc1234, c0ffee1]])
    }

    #[test]
    // Verifies: ADR-0019, G-11
    fn a_blank_query_matches_nothing() {
        let h = searchable();
        assert!(h.search("").is_empty());
        assert!(h.search("   ").is_empty());
    }

    #[test]
    // Verifies: ADR-0019, G-11
    fn search_matches_message_author_committer_sha_and_refs_case_insensitively() {
        let h = searchable();
        assert_eq!(found(&h, "HEADER"), ["c0ffee1"]);
        assert_eq!(found(&h, "alice"), ["abc1234", "c0ffee1"]);
        assert_eq!(found(&h, "carol"), ["abc1234"]);
        assert_eq!(found(&h, "DEADBE"), ["deadbee"]);
        assert_eq!(found(&h, "feature/"), ["deadbee"]);
        assert_eq!(found(&h, " FEAT "), ["deadbee"], "trimmed");
    }

    #[test]
    // Verifies: ADR-0019, G-11
    fn search_results_run_newest_first() {
        assert_eq!(found(&searchable(), ":"), ["deadbee", "abc1234", "c0ffee1"]);
    }

    #[test]
    // Verifies: ADR-0019, G-11
    fn search_covers_every_chunk_laid_out() {
        let commits = generated_history(3, 90);
        let h = history(&commits.chunks(30).map(<[_]>::to_vec).collect::<Vec<_>>());
        assert_eq!(
            found(&h, "commit c0"),
            ["c0"],
            "the oldest commit, in the last chunk"
        );
        assert_eq!(h.search("commit").len(), 90);
    }

    // ── Lineage (G-10) ─────────────────────────────────────────────────────────────────────

    fn shas(h: &History, rows: HashSet<usize>) -> Vec<String> {
        let mut out: Vec<String> = rows.into_iter().map(|r| h.node(r).sha.clone()).collect();
        out.sort();
        out
    }

    /// M merges B and F, both on C, which stands on R; D is an unrelated root.
    fn diamond() -> History {
        history(&[vec![
            commit("M", 6, &["B", "F"]),
            commit("F", 5, &["C"]),
            commit("D", 4, &[]),
            commit("B", 3, &["C"]),
            commit("C", 2, &["R"]),
            commit("R", 1, &[]),
        ]])
    }

    fn at(h: &History, sha: &str) -> usize {
        h.row_of(sha).unwrap()
    }

    #[test]
    // Verifies: ADR-0019, G-10
    fn ancestors_follow_every_parent_line_and_include_the_start() {
        let h = diamond();
        assert_eq!(
            shas(&h, h.ancestors(at(&h, "M"))),
            ["B", "C", "F", "M", "R"]
        );
        assert_eq!(
            shas(&h, h.ancestors(at(&h, "F"))),
            ["C", "F", "R"],
            "no children, no siblings"
        );
    }

    #[test]
    // Verifies: ADR-0019, G-10
    fn parents_not_laid_out_are_skipped() {
        let h = history(&[vec![commit("A", 1, &["missing"])]]);
        assert_eq!(shas(&h, h.ancestors(0)), ["A"]);
    }

    #[test]
    // Verifies: ADR-0019, G-10
    fn descendants_follow_child_links_up_to_the_tips() {
        let h = diamond();
        assert_eq!(shas(&h, h.descendants(at(&h, "C"))), ["B", "C", "F", "M"]);
        assert_eq!(shas(&h, h.descendants(at(&h, "B"))), ["B", "M"]);
        assert_eq!(
            shas(&h, h.descendants(at(&h, "M"))),
            ["M"],
            "a tip has none"
        );
    }

    #[test]
    // Verifies: ADR-0019, G-10
    fn lineage_is_ancestors_and_descendants_without_siblings() {
        let h = diamond();
        assert_eq!(
            shas(&h, h.lineage(at(&h, "B")).rows()),
            ["B", "C", "M", "R"]
        );
        assert_eq!(shas(&h, h.lineage(at(&h, "D")).rows()), ["D"]);
    }

    #[test]
    // Verifies: ADR-0019, G-11, G-10
    fn the_terminal_applications_cases_hold_over_the_core() {
        // a ← b ← d (merge of b and c), a ← c, and e on top of c (unmerged).
        let mut c = commit("c", 3, &["a"]);
        c.author_name = "Carol".into();
        c.refs = vec!["feature".into()];
        let mut d = commit("d", 4, &["b", "c"]);
        d.refs = vec!["main".into()];
        let h = history(&[vec![
            commit("e", 5, &["c"]),
            d,
            c,
            commit("b", 2, &["a"]),
            commit("a", 1, &[]),
        ]]);
        assert_eq!(found(&h, "CAROL"), ["c"]);
        assert_eq!(found(&h, "main"), ["d"]);
        assert_eq!(found(&h, " FEAT "), ["c"]);
        assert_eq!(shas(&h, h.lineage(at(&h, "b")).rows()), ["a", "b", "d"]);
        assert_eq!(
            shas(&h, h.lineage(at(&h, "c")).rows()),
            ["a", "c", "d", "e"]
        );
    }

    #[test]
    // Verifies: ADR-0019, G-11, G-10
    fn search_and_lineage_grown_by_a_chunk_equal_them_computed_afresh() {
        for seed in 1..20u64 {
            let commits = generated_history(seed, 120);
            let (first, rest) = commits.split_at(50);
            let mut h = history(&[first.to_vec()]);
            let lineages: Vec<Lineage> = (0..h.rows()).map(|r| h.lineage(r)).collect();
            let found = h.search("1");
            h.add_chunk(rest.to_vec());
            for (start, mut lineage) in lineages.into_iter().enumerate() {
                h.extend_lineage(&mut lineage, 50);
                assert_eq!(lineage, h.lineage(start), "seed {seed}, row {start}");
            }
            let grown: Vec<usize> = found.into_iter().chain(h.search_from("1", 50)).collect();
            assert_eq!(grown, h.search("1"), "seed {seed}");
        }
    }

    #[test]
    // Verifies: ADR-0019, G-10
    fn lineage_reaches_into_chunks_laid_out_later() {
        let h = history(&[
            vec![commit("c", 3, &["b"])],
            vec![commit("b", 2, &["a"])],
            vec![commit("a", 1, &[])],
        ]);
        assert_eq!(shas(&h, h.lineage(at(&h, "c")).rows()), ["a", "b", "c"]);
        assert_eq!(shas(&h, h.lineage(at(&h, "a")).rows()), ["a", "b", "c"]);
    }

    #[test]
    // Verifies: ADR-0013, G-10
    fn a_late_child_joins_its_parents_children_and_lineage() {
        let mut h = History::new();
        h.add_chunk(vec![commit("tip", 4, &["p"]), commit("p", 3, &["base"])]);
        let mut lineage = h.lineage(at(&h, "p"));
        let from = h.rows();
        h.add_chunk(vec![commit("late", 1, &["p"]), commit("base", 0, &[])]);
        assert_eq!(h.node(at(&h, "p")).children, ["tip", "late"]);
        h.extend_lineage(&mut lineage, from);
        assert_eq!(lineage, h.lineage(at(&h, "p")));
        assert_eq!(shas(&h, lineage.rows()), ["base", "late", "p", "tip"]);
        let w = h.window(0, 4);
        assert!(
            w.edges.iter().any(|e| e.source_row < e.target_row),
            "the edge down to the late child is read with the window"
        );
    }

    #[test]
    // Verifies: ADR-0019, ADR-0013, G-10
    fn lineages_grown_over_late_children_equal_them_computed_afresh() {
        let mut late = 0;
        for seed in 1..30u64 {
            let commits = skewed_history(seed, 150);
            let mut h = History::new();
            let mut lineages: Vec<Lineage> = Vec::new();
            for chunk in commits.chunks(25) {
                let from = h.rows();
                h.add_chunk(chunk.to_vec());
                for lineage in &mut lineages {
                    h.extend_lineage(lineage, from);
                }
                lineages.extend((from..h.rows()).map(|r| h.lineage(r)));
            }
            late += h.late.len();
            for (start, lineage) in lineages.iter().enumerate() {
                assert_eq!(*lineage, h.lineage(start), "seed {seed}, row {start}");
            }
        }
        assert!(late > 0, "the histories hold late children");
    }

    #[test]
    // Verifies: ADR-0018
    fn a_link_is_labelled_by_its_short_sha_or_the_working_directorys_name() {
        let mut work = commit("WORKING_DIRECTORY", 9, &["abcdef123456"]);
        work.kind = CommitKind::Working;
        let h = history(&[vec![work, commit("abcdef123456", 1, &[])]]);
        assert_eq!(h.link_label("abcdef123456"), "abcdef1");
        assert_eq!(h.link_label("WORKING_DIRECTORY"), "Uncommitted");
        assert_eq!(h.link_label("0123456789"), "0123456", "not laid out");
    }

    #[test]
    // Verifies: ADR-0019, ADR-0020, G-7
    fn a_window_looks_at_the_edges_near_it_not_at_every_long_one() {
        // A main line of 20 000 commits with a branch forked off every tenth and merged back two
        // hundred commits above: thousands of long edges, few of them near any one window.
        let mut commits = Vec::new();
        for i in 0..20_000i64 {
            let parent = format!("m{}", i - 1);
            let mut parents = if i > 0 { vec![parent.as_str()] } else { vec![] };
            let side = format!("s{}", i - 200);
            if i >= 200 && i % 10 == 0 {
                parents.push(side.as_str());
            }
            commits.push(commit(&format!("m{i}"), 2 * i, &parents));
            if i % 10 == 0 && i + 200 < 20_000 {
                commits.push(commit(&format!("s{i}"), 2 * i + 1, &[&format!("m{i}")]));
            }
        }
        commits.reverse();
        let h = history(&commits.chunks(3_000).map(<[_]>::to_vec).collect::<Vec<_>>());
        let long = h.edge_levels.iter().skip(1).flatten().flatten().count();
        assert!(long > 1_900, "the history holds {long} long edges");
        for top in [0, 7_000, 15_000] {
            let near = h.edges_near(top, top + 59).count();
            let crossing = h.window(top, 60).edges.len();
            assert!(
                near <= 4 * crossing + 100,
                "a window at {top} looks at {near} edges for {crossing} crossing it"
            );
        }
    }

    #[test]
    // Verifies: ADR-0019
    fn a_window_names_the_lanes_its_nodes_and_edges_reach() {
        // d merges c, whose lane is 1; the window of d's row alone reaches it by the edge.
        let h = history(&[vec![
            commit("d", 4, &["b", "c"]),
            commit("c", 3, &["a"]),
            commit("b", 2, &["a"]),
            commit("a", 1, &[]),
        ]]);
        assert_eq!(h.window(0, 1).lanes(), 2);
        assert_eq!(h.window(0, 4).lanes(), h.lanes());
        assert_eq!(h.window(9, 1).lanes(), 0, "past the end");
    }
}
