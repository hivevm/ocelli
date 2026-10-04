//! Turns the laid-out commit graph into terminal cells (ADR-0028).
//!
//! The rows, columns and edges come unchanged from `ocelli_core::graph`; this
//! module only decides which box-drawing character each cell shows. A graph
//! column occupies two cells — the glyph cell at `x = 2 * col` and a connector
//! cell to its right — and every row is one line.
//!
//! Edge shapes follow the *Edge rendering* table of the specification, keyed on
//! the edge type (ADR-0030): a BRANCH edge runs horizontally along the parent's
//! row and bends up into the child's column; a MERGE edge runs up the parent's
//! column and bends at the child's row — whichever side the child is on. The
//! rounded box corners `╭ ╮ ╰ ╯` are the terminal form of the softened bend.

use ocelli_core::model::{CommitKind, Edge, Graph, NodeJson};
use std::collections::HashSet;

const UP: u8 = 1;
const DOWN: u8 = 2;
const LEFT: u8 = 4;
const RIGHT: u8 = 8;

/// Edges spanning more rows than this are not bucketed by their first row but
/// kept in a separate list, so a window query scans a bounded number of
/// buckets plus that short list — the same scheme the desktop canvas uses.
const LONG_EDGE_ROWS: usize = 64;

/// The character set the graph is drawn with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Glyphs {
    Unicode,
    /// For terminals or fonts without box-drawing characters.
    Ascii,
}

/// One rasterised cell. `lane` is the graph column whose colour the cell takes;
/// `None` for an empty cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub ch: char,
    pub lane: Option<usize>,
    /// Outside the highlighted lineage — drawn faint.
    pub dim: bool,
}

const EMPTY: Cell = Cell {
    ch: ' ',
    lane: None,
    dim: false,
};

/// Lookups built once per load, so that rasterising a window costs
/// `O(visible rows)` rather than `O(history)` (ADR-0017).
pub struct GraphIndex {
    /// `by_row[row]` is the index into `graph.nodes` of the node in that row.
    by_row: Vec<usize>,
    /// Short edges, bucketed by their first (lowest) row.
    short_by_min_row: Vec<Vec<usize>>,
    /// Edges spanning more than [`LONG_EDGE_ROWS`] rows.
    long: Vec<usize>,
    /// Width of the graph in cells.
    pub width: usize,
}

impl GraphIndex {
    pub fn new(graph: &Graph) -> Self {
        let rows = graph.nodes.len();
        let mut by_row = vec![0; rows];
        for (i, n) in graph.nodes.iter().enumerate() {
            by_row[n.row as usize] = i;
        }

        let mut short_by_min_row = vec![Vec::new(); rows];
        let mut long = Vec::new();
        for (i, e) in graph.edges.iter().enumerate() {
            if (e.max_row - e.min_row) as usize > LONG_EDGE_ROWS {
                long.push(i);
            } else {
                short_by_min_row[e.min_row as usize].push(i);
            }
        }

        let columns = graph.nodes.iter().map(|n| n.col + 1).max().unwrap_or(0) as usize;
        Self {
            by_row,
            short_by_min_row,
            long,
            width: (2 * columns).saturating_sub(1),
        }
    }

    pub fn rows(&self) -> usize {
        self.by_row.len()
    }

    /// Index into `graph.nodes` of the node in `row`.
    pub fn node_at(&self, row: usize) -> usize {
        self.by_row[row]
    }

    /// Every edge whose row span intersects `top..=bottom` — including edges
    /// that only pass through the window with both ends off-screen.
    fn edges_in<'g>(
        &self,
        graph: &'g Graph,
        top: usize,
        bottom: usize,
    ) -> impl Iterator<Item = &'g Edge> {
        let first_bucket = top.saturating_sub(LONG_EDGE_ROWS);
        let last_bucket = bottom.min(self.short_by_min_row.len().saturating_sub(1));
        let short = self
            .short_by_min_row
            .get(first_bucket..=last_bucket)
            .unwrap_or_default()
            .iter()
            .flatten();
        short
            .chain(self.long.iter())
            .map(|&i| &graph.edges[i])
            .filter(move |e| e.max_row as usize >= top && e.min_row as usize <= bottom)
    }
}

/// Cell state while edges are being collected: the directions that meet in the
/// cell, and which lanes supplied the vertical and the horizontal part.
#[derive(Clone, Copy, Default)]
struct Work {
    mask: u8,
    v_lane: Option<usize>,
    h_lane: Option<usize>,
    v_dim: bool,
    h_dim: bool,
}

/// The window being rasterised: `rows` lines starting at row `top`.
struct Window {
    top: usize,
    cells: Vec<Vec<Work>>,
}

impl Window {
    fn bottom(&self) -> usize {
        self.top + self.cells.len() - 1
    }

    /// A vertical run in cell column `x` from `top_row` down to `bottom_row`,
    /// clipped to the window.
    fn vertical(&mut self, x: usize, top_row: usize, bottom_row: usize, lane: usize, dim: bool) {
        let from = top_row.max(self.top);
        let to = bottom_row.min(self.bottom());
        for row in from..=to {
            let mut bits = 0;
            if row > top_row {
                bits |= UP;
            }
            if row < bottom_row {
                bits |= DOWN;
            }
            let w = &mut self.cells[row - self.top][x];
            w.mask |= bits;
            w.v_lane = Some(lane);
            w.v_dim = dim;
        }
    }

    /// A horizontal run along `row` from cell `left` to cell `right`.
    fn horizontal(&mut self, row: usize, left: usize, right: usize, lane: usize, dim: bool) {
        if row < self.top || row > self.bottom() {
            return;
        }
        for x in left..=right {
            let mut bits = 0;
            if x > left {
                bits |= LEFT;
            }
            if x < right {
                bits |= RIGHT;
            }
            let w = &mut self.cells[row - self.top][x];
            w.mask |= bits;
            if w.h_lane.is_none() {
                w.h_lane = Some(lane);
                w.h_dim = dim;
            }
        }
    }

    /// Draws one edge; `dim` when it is not on the highlighted lineage.
    fn edge(&mut self, e: &Edge, dim: bool) {
        // source = parent (older, lower on screen), target = child (newer).
        let (sr, tr) = (e.source_row as usize, e.target_row as usize);
        let (sc, tc) = (e.source_col as usize, e.target_col as usize);
        let (xs, xt) = (2 * sc, 2 * tc);
        // Same rule as the desktop canvas: a BRANCH edge takes the child's lane
        // colour, a MERGE edge the parent's.
        let lane = if e.is_branch { tc } else { sc };

        let (left, right) = (xs.min(xt), xs.max(xt));
        if sc == tc {
            self.vertical(xs, tr, sr, lane, dim);
        } else if e.is_branch {
            // BRANCH: along the parent's row, then up the child's lane.
            self.horizontal(sr, left, right, lane, dim);
            self.vertical(xt, tr, sr, lane, dim);
        } else {
            // MERGE: up the parent's column, then along the child's row.
            self.vertical(xs, tr, sr, lane, dim);
            self.horizontal(tr, left, right, lane, dim);
        }
    }
}

fn line_char(mask: u8, glyphs: Glyphs) -> char {
    let unicode = match mask {
        0 => ' ',
        m if m == UP | DOWN || m == UP || m == DOWN => '│',
        m if m == LEFT | RIGHT || m == LEFT || m == RIGHT => '─',
        m if m == DOWN | RIGHT => '╭',
        m if m == DOWN | LEFT => '╮',
        m if m == UP | RIGHT => '╰',
        m if m == UP | LEFT => '╯',
        m if m == UP | DOWN | RIGHT => '├',
        m if m == UP | DOWN | LEFT => '┤',
        m if m == LEFT | RIGHT | DOWN => '┬',
        m if m == LEFT | RIGHT | UP => '┴',
        _ => '┼',
    };
    match (glyphs, unicode) {
        (Glyphs::Unicode, c) => c,
        (Glyphs::Ascii, ' ') => ' ',
        (Glyphs::Ascii, '│') => '|',
        (Glyphs::Ascii, '─') => '-',
        (Glyphs::Ascii, _) => '+',
    }
}

/// The node glyph. A merge commit carries a double circle, so merges stand
/// out without following their edges — the desktop's merge marker.
fn node_char(node: &NodeJson, glyphs: Glyphs) -> char {
    let merge = node.kind == CommitKind::Commit && node.parents.len() > 1;
    match (glyphs, node.kind) {
        (Glyphs::Unicode, CommitKind::Commit) if merge => '◉',
        (Glyphs::Ascii, CommitKind::Commit) if merge => '@',
        (Glyphs::Unicode, CommitKind::Commit) => '●',
        (Glyphs::Unicode, CommitKind::Stash) => '◆',
        (Glyphs::Unicode, CommitKind::Working) => '◌',
        (Glyphs::Ascii, CommitKind::Commit) => '*',
        (Glyphs::Ascii, CommitKind::Stash) => '#',
        (Glyphs::Ascii, CommitKind::Working) => 'o',
    }
}

/// Rasterises rows `top..top + rows` of the graph, one `Vec<Cell>` of
/// `index.width` cells per row. Rows past the end of the history are omitted.
///
/// With a `highlight` set of rows (a selected commit's lineage), nodes outside
/// it are dimmed, and so is every edge that does not join two of its rows —
/// every loaded parent of a lineage node is itself in the lineage, so the
/// edges inside the set are exactly the lineage's own.
pub fn rasterize(
    graph: &Graph,
    index: &GraphIndex,
    top: usize,
    rows: usize,
    glyphs: Glyphs,
    highlight: Option<&HashSet<usize>>,
) -> Vec<Vec<Cell>> {
    let off = |row: i32| highlight.is_some_and(|h| !h.contains(&(row as usize)));
    let rows = rows.min(index.rows().saturating_sub(top));
    if rows == 0 {
        return Vec::new();
    }
    let mut window = Window {
        top,
        cells: vec![vec![Work::default(); index.width]; rows],
    };
    for e in index.edges_in(graph, top, window.bottom()) {
        window.edge(e, off(e.source_row) || off(e.target_row));
    }

    window
        .cells
        .iter()
        .enumerate()
        .map(|(i, line)| {
            let node = &graph.nodes[index.node_at(top + i)];
            line.iter()
                .enumerate()
                .map(|(x, w)| {
                    if x == 2 * node.col as usize {
                        Cell {
                            ch: node_char(node, glyphs),
                            lane: Some(node.col as usize),
                            dim: off(node.row),
                        }
                    } else if w.mask == 0 {
                        EMPTY
                    } else {
                        // The vertical lane wins the colour, so a lane keeps its
                        // colour where another edge crosses it.
                        Cell {
                            ch: line_char(w.mask, glyphs),
                            lane: w.v_lane.or(w.h_lane),
                            dim: if w.v_lane.is_some() { w.v_dim } else { w.h_dim },
                        }
                    }
                })
                .collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocelli_core::graph::build_graph;
    use ocelli_core::model::CommitData;

    fn commit(sha: &str, time: i64, parents: &[&str]) -> CommitData {
        CommitData {
            sha: sha.to_string(),
            message: format!("commit {sha}"),
            author_name: "Test".to_string(),
            author_email: String::new(),
            author_time: time,
            committer_name: "Test".to_string(),
            committer_email: String::new(),
            commit_time: time,
            refs: vec![],
            parent_shas: parents.iter().map(|p| p.to_string()).collect(),
            kind: CommitKind::Commit,
        }
    }

    fn render(graph: &Graph, top: usize, rows: usize, glyphs: Glyphs) -> Vec<String> {
        let index = GraphIndex::new(graph);
        rasterize(graph, &index, top, rows, glyphs, None)
            .iter()
            .map(|line| line.iter().map(|c| c.ch).collect::<String>())
            .collect()
    }

    /// `a` ← `b` on main, `c` branches off `a` and is merged back by `d`.
    fn fork_and_merge() -> Graph {
        build_graph(vec![
            commit("a", 1, &[]),
            commit("b", 2, &["a"]),
            commit("c", 3, &["a"]),
            commit("d", 4, &["b", "c"]),
        ])
    }

    #[test]
    fn linear_history_is_one_straight_lane() {
        let g = build_graph(vec![
            commit("a", 1, &[]),
            commit("b", 2, &["a"]),
            commit("c", 3, &["b"]),
        ]);
        assert_eq!(render(&g, 0, 10, Glyphs::Unicode), ["●", "●", "●"]);
    }

    #[test]
    // Verifies: ADR-0030
    fn fork_bends_at_the_target_column_and_merge_at_the_source_column() {
        let g = fork_and_merge();
        // Rows: d (merge), c (side branch), b, a. The merge edge d←c runs up
        // c's column and bends left at d's row; the fork a→c runs along a's row
        // and bends up at c's column.
        assert_eq!(
            render(&g, 0, 10, Glyphs::Unicode),
            ["◉─╮", "│ ●", "● │", "●─╯"]
        );
    }

    #[test]
    // Verifies: ADR-0028
    fn ascii_glyphs_replace_every_box_drawing_character() {
        let g = fork_and_merge();
        assert_eq!(
            render(&g, 0, 10, Glyphs::Ascii),
            ["@-+", "| *", "* |", "*-+"]
        );
    }

    #[test]
    // Verifies: ADR-0017
    fn edges_passing_through_the_window_are_drawn_from_off_screen_ends() {
        let g = fork_and_merge();
        // Only the middle two rows: both lanes still show, although every edge
        // crossing them has at least one end outside the window.
        assert_eq!(render(&g, 1, 2, Glyphs::Unicode), ["│ ●", "● │"]);
    }

    /// A hand-built graph, for shapes the layout itself avoids producing.
    fn graph(nodes: &[(i32, i32)], edges: &[(i32, i32, i32, i32, bool)]) -> Graph {
        use ocelli_core::model::NodeJson;
        Graph {
            nodes: nodes
                .iter()
                .map(|&(row, col)| NodeJson {
                    row,
                    col,
                    sha: format!("{row}"),
                    short_sha: format!("{row}"),
                    message: String::new(),
                    author: String::new(),
                    author_date: String::new(),
                    author_avatar: String::new(),
                    committer: String::new(),
                    date: String::new(),
                    committer_avatar: String::new(),
                    refs: vec![],
                    kind: CommitKind::Commit,
                    parents: vec![],
                    children: vec![],
                })
                .collect(),
            edges: edges
                .iter()
                .map(
                    |&(source_row, source_col, target_row, target_col, is_branch)| Edge {
                        source_row,
                        source_col,
                        target_row,
                        target_col,
                        is_branch,
                        min_row: target_row,
                        max_row: source_row,
                    },
                )
                .collect(),
        }
    }

    #[test]
    // Verifies: G-2
    fn a_lane_crossed_by_a_fork_stays_unbroken_and_keeps_its_colour() {
        // Lane 1 runs from row 0 to row 3; a fork from row 2 (column 0) to
        // row 1 (column 2) crosses it on row 2.
        let g = graph(
            &[(0, 1), (1, 2), (2, 0), (3, 1)],
            &[(3, 1, 0, 1, true), (2, 0, 1, 2, true)],
        );
        assert_eq!(
            render(&g, 0, 10, Glyphs::Unicode),
            ["  ●  ", "  │ ●", "●─┼─╯", "  ●  "]
        );
        let index = GraphIndex::new(&g);
        let cells = rasterize(&g, &index, 2, 1, Glyphs::Unicode, None);
        assert_eq!(
            cells[0][2].lane,
            Some(1),
            "the crossed lane keeps its colour"
        );
        assert_eq!(
            cells[0][3].lane,
            Some(2),
            "the fork takes the child's colour"
        );
    }

    #[test]
    // Verifies: ADR-0017, G-14
    fn long_edges_are_found_by_a_window_far_from_both_ends() {
        // A side branch whose fork is more than LONG_EDGE_ROWS below its tip.
        let mut commits = vec![commit("root", 0, &[]), commit("side", 1000, &["root"])];
        let mut prev = "root".to_string();
        for i in 1..=(LONG_EDGE_ROWS as i64 + 10) {
            let sha = format!("m{i}");
            commits.push(commit(&sha, i, &[prev.as_str()]));
            prev = sha;
        }
        let g = build_graph(commits);
        let lines = render(&g, 30, 1, Glyphs::Unicode);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].chars().filter(|&c| c == '│').count(), 1);
    }

    #[test]
    // Verifies: ADR-0024
    fn synthetic_nodes_get_their_own_glyphs() {
        let mut stash = commit("s", 3, &["a"]);
        stash.kind = CommitKind::Stash;
        let mut work = commit("w", 4, &["a"]);
        work.kind = CommitKind::Working;
        let g = build_graph(vec![commit("a", 1, &[]), stash, work]);
        let lines = render(&g, 0, 10, Glyphs::Unicode).concat();
        assert!(lines.contains('◆') && lines.contains('◌'), "{lines}");
    }

    #[test]
    // Verifies: ADR-0030
    fn a_merge_to_the_right_bends_at_the_top_and_clears_the_child_lane() {
        // `m` (column 1) merges `c` (column 0) into the feature line `f`
        // (column 1). The merge edge runs up c's column and bends at m's row,
        // instead of up m's column through `f`.
        let g = graph(
            &[(0, 1), (1, 1), (2, 0)],
            &[(1, 1, 0, 1, true), (2, 0, 0, 1, false)],
        );
        assert_eq!(render(&g, 0, 10, Glyphs::Unicode), ["╭─●", "│ ●", "●  "]);
    }

    #[test]
    // Verifies: ADR-0030
    fn a_fork_to_the_left_bends_at_the_bottom() {
        // `p` (column 1) is the first parent of `d` (column 0).
        let g = graph(&[(0, 0), (1, 1)], &[(1, 1, 0, 0, true)]);
        assert_eq!(render(&g, 0, 10, Glyphs::Unicode), ["●  ", "╰─●"]);
    }

    #[test]
    fn outside_a_highlighted_lineage_nodes_and_edges_are_dimmed() {
        let g = fork_and_merge();
        let index = GraphIndex::new(&g);
        // Lineage of b: rows 0 (d), 2 (b), 3 (a) — c's row 1 is outside.
        let lineage: HashSet<usize> = [0, 2, 3].into();
        let cells = rasterize(&g, &index, 0, 4, Glyphs::Unicode, Some(&lineage));
        let dim = |row: usize, x: usize| cells[row][x].dim;
        assert!(!dim(0, 0) && !dim(1, 0) && !dim(2, 0), "d—b—a stays bright");
        assert!(dim(1, 2), "c is outside the lineage");
        assert!(dim(0, 1) && dim(0, 2), "the merge edge d←c is dimmed");
        assert!(dim(3, 1) && dim(3, 2), "the fork a→c is dimmed");
        let none = rasterize(&g, &index, 0, 4, Glyphs::Unicode, None);
        assert!(none.iter().flatten().all(|c| !c.dim));
    }

    #[test]
    // Verifies: ADR-0017, G-14
    fn a_window_past_the_end_is_empty() {
        let g = fork_and_merge();
        assert!(render(&g, 4, 5, Glyphs::Unicode).is_empty());
        assert_eq!(render(&g, 3, 5, Glyphs::Unicode).len(), 1);
    }
}
