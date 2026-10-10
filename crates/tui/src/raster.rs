//! Turns the laid-out commit graph into terminal cells (ADR-0028).
//!
//! The rows, columns and edges come unchanged from the window of rows the core
//! answers (ADR-0031); this module only decides which box-drawing character
//! each cell shows. A graph
//! column occupies two cells — the glyph cell at `x = 2 * col` and a connector
//! cell to its right — and every row is one line.
//!
//! Edge shapes follow the table of ADR-0030, keyed on
//! the edge type: a BRANCH edge runs horizontally along the parent's
//! row and bends up into the child's column; a MERGE edge runs up the parent's
//! column and bends at the child's row — whichever side the child is on. The
//! rounded box corners `╭ ╮ ╰ ╯` are the terminal form of the softened bend.

use ocelli_core::history::Lineage;
use ocelli_core::history::Window;
use ocelli_core::model::{CommitKind, Edge, NodeJson, Run};

const UP: u8 = 1;
const DOWN: u8 = 2;
const LEFT: u8 = 4;
const RIGHT: u8 = 8;

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

/// The width in cells of a graph of `lanes` lanes.
pub fn width(lanes: usize) -> usize {
    (2 * lanes).saturating_sub(1)
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

/// The cells being rasterised: `rows` lines starting at row `top`.
struct Canvas {
    top: usize,
    cells: Vec<Vec<Work>>,
}

impl Canvas {
    fn bottom(&self) -> usize {
        self.top + self.cells.len() - 1
    }

    /// A vertical run in cell column `x` from `top_row` down to `bottom_row`,
    /// clipped to the canvas.
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
        // Same rule as the desktop canvas: a BRANCH edge takes the child's lane
        // colour, a MERGE edge the parent's.
        let lane = if e.is_branch {
            e.target_col
        } else {
            e.source_col
        } as usize;
        // The runs the core routes the edge along (ADR-0030); a lane is two cells wide.
        for run in e.route() {
            match run {
                Run::Along { row, from, to } => {
                    self.horizontal(row as usize, 2 * from as usize, 2 * to as usize, lane, dim)
                }
                Run::Down { lane: x, from, to } => {
                    self.vertical(2 * x as usize, from as usize, to as usize, lane, dim)
                }
            }
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

/// Rasterises a window of rows, one `Vec<Cell>` of `width` cells per row.
///
/// With a `highlight` set of rows (a selected commit's lineage), nodes outside
/// it are dimmed, and so is every edge that does not join two of its rows —
/// every loaded parent of a lineage node is itself in the lineage, so the
/// edges inside the set are exactly the lineage's own.
pub fn rasterize(
    window: &Window,
    width: usize,
    glyphs: Glyphs,
    highlight: Option<&Lineage>,
) -> Vec<Vec<Cell>> {
    let off = |row: i32| highlight.is_some_and(|h| !h.contains(row as usize));
    if window.nodes.is_empty() {
        return Vec::new();
    }
    let mut canvas = Canvas {
        top: window.top,
        cells: vec![vec![Work::default(); width]; window.nodes.len()],
    };
    for e in &window.edges {
        canvas.edge(e, off(e.source_row) || off(e.target_row));
    }

    canvas
        .cells
        .iter()
        .zip(&window.nodes)
        .map(|(line, node)| {
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
    use ocelli_core::history::History;
    use ocelli_core::model::{CommitData, Graph};

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
            stash: None,
        }
    }

    fn laid_out(commits: Vec<CommitData>) -> History {
        let mut h = History::new();
        h.add_chunk(commits);
        h
    }

    fn chars(cells: Vec<Vec<Cell>>) -> Vec<String> {
        cells
            .iter()
            .map(|line| line.iter().map(|c| c.ch).collect::<String>())
            .collect()
    }

    /// Rows `top..top + rows` of a laid-out history, through the window the core answers.
    fn render(h: &History, top: usize, rows: usize, glyphs: Glyphs) -> Vec<String> {
        chars(rasterize(
            &h.window(top, rows),
            width(h.lanes()),
            glyphs,
            None,
        ))
    }

    /// `a` ← `b` on main, `c` branches off `a` and is merged back by `d`.
    fn fork_and_merge() -> History {
        laid_out(vec![
            commit("a", 1, &[]),
            commit("b", 2, &["a"]),
            commit("c", 3, &["a"]),
            commit("d", 4, &["b", "c"]),
        ])
    }

    #[test]
    fn linear_history_is_one_straight_lane() {
        let g = laid_out(vec![
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

    /// The whole of a hand-built graph as one window.
    fn whole(g: &Graph) -> Window {
        Window {
            top: 0,
            nodes: g.nodes.clone(),
            edges: g.edges.clone(),
        }
    }

    fn render_whole(g: &Graph) -> Vec<String> {
        let w = whole(g);
        chars(rasterize(&w, width(w.lanes()), Glyphs::Unicode, None))
    }

    /// A hand-built graph, for shapes the layout itself avoids producing. Its nodes are given
    /// in row order.
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
                    stash: None,
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
                        track: None,
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
        assert_eq!(render_whole(&g), ["  ●  ", "  │ ●", "●─┼─╯", "  ●  "]);
        let row_2 = Window {
            top: 2,
            nodes: vec![g.nodes[2].clone()],
            edges: g.edges.clone(),
        };
        let cells = rasterize(&row_2, width(3), Glyphs::Unicode, None);
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
    fn long_edges_are_drawn_in_a_window_far_from_both_ends() {
        // A side branch whose fork lies 75 rows below its tip, beyond the
        // span the core buckets edges by.
        let mut commits = vec![commit("root", 0, &[]), commit("side", 1000, &["root"])];
        let mut prev = "root".to_string();
        for i in 1..=74 {
            let sha = format!("m{i}");
            commits.push(commit(&sha, i, &[prev.as_str()]));
            prev = sha;
        }
        let g = laid_out(commits);
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
        let g = laid_out(vec![commit("a", 1, &[]), stash, work]);
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
        assert_eq!(render_whole(&g), ["╭─●", "│ ●", "●  "]);
    }

    #[test]
    // Verifies: ADR-0015
    fn an_edge_down_to_a_late_child_is_drawn_between_its_ends() {
        // The parent in row 0 and its late child in row 2, below it, on lane 0; a commit of lane 1
        // between them.
        let g = graph(&[(0, 0), (1, 1), (2, 0)], &[(0, 0, 2, 0, true)]);
        assert_eq!(render_whole(&g), ["●  ", "│ ●", "●  "]);
    }

    #[test]
    // Verifies: ADR-0030, G-9
    fn a_merge_out_of_the_main_line_runs_up_its_track() {
        // `m` (lane 0, row 2) is merged into `f` (lane 1, row 0) past the main line's `n` (lane 0,
        // row 1); the edge leaves lane 0 at m's row, runs up track 2, and turns to f.
        let mut g = graph(&[(0, 1), (1, 0), (2, 0)], &[(2, 0, 1, 0, true)]);
        g.edges.push(Edge {
            source_row: 2,
            source_col: 0,
            target_row: 0,
            target_col: 1,
            is_branch: false,
            min_row: 0,
            max_row: 2,
            track: Some(2),
        });
        assert_eq!(render_whole(&g), ["  ●─╮", "●   │", "●───╯"]);
    }

    #[test]
    // Verifies: ADR-0030
    fn a_fork_to_the_left_bends_at_the_bottom() {
        // `p` (column 1) is the first parent of `d` (column 0).
        let g = graph(&[(0, 0), (1, 1)], &[(1, 1, 0, 0, true)]);
        assert_eq!(render_whole(&g), ["●  ", "╰─●"]);
    }

    #[test]
    fn outside_a_highlighted_lineage_nodes_and_edges_are_dimmed() {
        let g = fork_and_merge();
        let w = g.window(0, 4);
        // Lineage of b: rows 0 (d), 2 (b), 3 (a) — c's row 1 is outside.
        let lineage = g.lineage(2);
        let cells = rasterize(&w, width(g.lanes()), Glyphs::Unicode, Some(&lineage));
        let dim = |row: usize, x: usize| cells[row][x].dim;
        assert!(!dim(0, 0) && !dim(1, 0) && !dim(2, 0), "d—b—a stays bright");
        assert!(dim(1, 2), "c is outside the lineage");
        assert!(dim(0, 1) && dim(0, 2), "the merge edge d←c is dimmed");
        assert!(dim(3, 1) && dim(3, 2), "the fork a→c is dimmed");
        let none = rasterize(&w, width(g.lanes()), Glyphs::Unicode, None);
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
