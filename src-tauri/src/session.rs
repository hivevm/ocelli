//! The history of the repository open in the desktop application, kept in the host (ADR-0031).
//! A load lays its chunks out on a thread of its own and appends them here; the web interface
//! reads the window of rows it draws, searches, and selects through commands, so it never holds
//! more than the rows on screen. Every load is a generation: a chunk of an older generation is
//! refused, which stops the walk that read it.

use ocelli_core::graph::LaidChunk;
use ocelli_core::history::Lineage;
use ocelli_core::load::LoadSession;
use ocelli_core::model::{Edge, NodeJson};
use serde::Serialize;
use std::sync::RwLock;

/// The rows of one window, as the web interface draws them. With a commit selected, `lit_nodes`
/// and `lit_edges` say which of the window's nodes and edges belong to its lineage (G-17); the
/// rest is dimmed.
#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WindowJson {
    /// The load the rows belong to, so an answer that crossed a newer load is dropped.
    pub generation: u64,
    pub top: usize,
    /// The lanes the window's nodes and edges reach, which the graph column grows to.
    pub lanes: usize,
    pub nodes: Vec<NodeJson>,
    pub edges: Vec<Edge>,
    pub lit_nodes: Option<Vec<bool>>,
    pub lit_edges: Option<Vec<bool>>,
}

/// How far the history of a generation has loaded.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Growth {
    pub generation: u64,
    pub rows: usize,
    pub lanes: usize,
    pub loading: bool,
    pub status: String,
}

/// A parent or child of the selected commit: its label, and its row once it is laid out.
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Relative {
    pub sha: String,
    pub label: String,
    pub row: Option<usize>,
}

/// The selected commit with its parents and children.
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Selected {
    pub generation: u64,
    pub node: NodeJson,
    pub relatives: Vec<Relative>,
}

/// The rows matching a search, newest first, and the nodes of the first `limit` of them, which
/// the results panel lists (G-16).
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Found {
    pub generation: u64,
    pub rows: Vec<usize>,
    pub hits: Vec<NodeJson>,
}

#[derive(Default)]
struct Current {
    /// The generation of the newest load begun.
    begun: u64,
    /// The generation the session holds.
    generation: u64,
    session: LoadSession,
    /// The selected row and its lineage, grown as chunks arrive.
    selected: Option<(usize, Lineage)>,
    /// The last search: its query, the rows it searched, and its matches among them, so asking
    /// again as the history grows searches only the rows added.
    searched: Option<(String, usize, Vec<usize>)>,
}

impl Current {
    fn growth(&self) -> Growth {
        let history = self.session.history();
        Growth {
            generation: self.generation,
            rows: history.rows(),
            lanes: history.lanes(),
            loading: self.session.loading(),
            status: self.session.status(),
        }
    }
}

/// The history of the newest load, shared by the load's thread and the commands.
#[derive(Default)]
pub struct Session {
    current: RwLock<Current>,
}

impl Session {
    /// Begins a load and returns its generation. The history of the load before stays readable
    /// until the new load's first chunk replaces it.
    pub fn begin(&self) -> u64 {
        let mut c = self.current.write().unwrap();
        c.begun += 1;
        c.begun
    }

    /// Appends a chunk of `generation`, the first replacing the history before. `None` when a
    /// newer load has begun, which tells the load to stop.
    pub fn append(&self, generation: u64, chunk: LaidChunk) -> Option<Growth> {
        let mut c = self.current.write().unwrap();
        if generation != c.begun {
            return None;
        }
        if c.generation != generation {
            c.generation = generation;
            c.session = LoadSession::new();
            c.selected = None;
            c.searched = None;
        }
        let from = c.session.append(chunk);
        let Current {
            session, selected, ..
        } = &mut *c;
        if let Some((_, lineage)) = selected {
            session.history().extend_lineage(lineage, from);
        }
        Some(c.growth())
    }

    /// Whether `generation` is the newest load begun.
    pub fn is_current(&self, generation: u64) -> bool {
        self.current.read().unwrap().begun == generation
    }

    /// Ends the load of `generation`, the whole history read or the reason it stopped. `None`
    /// when a newer load has begun, or when the load ended before its first chunk.
    pub fn finish(&self, generation: u64, result: Result<(), String>) -> Option<Growth> {
        let mut c = self.current.write().unwrap();
        if generation != c.begun || c.generation != generation {
            return None;
        }
        c.session.end(result);
        Some(c.growth())
    }

    /// Rows `top..top + rows` with the edges crossing them (ADR-0031).
    pub fn window(&self, top: usize, rows: usize) -> WindowJson {
        let c = self.current.read().unwrap();
        let w = c.session.history().window(top, rows);
        let lineage = c.selected.as_ref().map(|(_, l)| l);
        let lit = |row: i32| lineage.is_none_or(|l| l.contains(row as usize));
        WindowJson {
            lit_nodes: lineage.map(|_| w.nodes.iter().map(|n| lit(n.row)).collect()),
            lit_edges: lineage.map(|_| {
                w.edges
                    .iter()
                    .map(|e| lit(e.source_row) && lit(e.target_row))
                    .collect()
            }),
            generation: c.generation,
            lanes: w.lanes(),
            top: w.top,
            nodes: w.nodes,
            edges: w.edges,
        }
    }

    /// Selects the commit in `row` and computes its lineage. The lineage is computed while the
    /// history is only read, so windows are read meanwhile; it grows by any chunk appended before
    /// it is kept.
    pub fn select(&self, row: usize) -> Option<Selected> {
        let (generation, rows, mut lineage) = {
            let c = self.current.read().unwrap();
            if row >= c.session.history().rows() {
                return None;
            }
            (
                c.generation,
                c.session.history().rows(),
                c.session.history().lineage(row),
            )
        };
        let mut c = self.current.write().unwrap();
        if c.generation != generation {
            return None;
        }
        c.session.history().extend_lineage(&mut lineage, rows);
        c.selected = Some((row, lineage));
        Some(selected(&c, row))
    }

    /// The selected commit, its relatives as far as they are laid out by now.
    pub fn selection(&self) -> Option<Selected> {
        let c = self.current.read().unwrap();
        c.selected.as_ref().map(|&(row, _)| selected(&c, row))
    }

    pub fn deselect(&self) {
        self.current.write().unwrap().selected = None;
    }

    /// The rows matching `query`, with the nodes of the first `limit`.
    ///
    /// The search runs while the history is only read, so windows are read meanwhile; its
    /// result is kept for the next search only if no chunk was appended in between.
    pub fn search(&self, query: &str, limit: usize) -> Found {
        let (found, searched) = {
            let c = self.current.read().unwrap();
            let (mut from, mut rows) = (0, Vec::new());
            if let Some((q, searched, found)) = &c.searched
                && q == query
            {
                (from, rows) = (*searched, found.clone());
            }
            rows.extend(c.session.history().search_from(query, from));
            let hits = rows
                .iter()
                .take(limit)
                .map(|&r| c.session.history().node(r))
                .collect();
            let searched = (c.generation, c.session.history().rows());
            let found = Found {
                generation: c.generation,
                rows,
                hits,
            };
            (found, searched)
        };
        let mut c = self.current.write().unwrap();
        if (c.generation, c.session.history().rows()) == searched {
            c.searched = Some((query.to_string(), searched.1, found.rows.clone()));
        }
        found
    }

    /// The row of each SHA, `None` for one not laid out.
    pub fn rows_of(&self, shas: &[String]) -> Vec<Option<usize>> {
        let c = self.current.read().unwrap();
        shas.iter().map(|s| c.session.history().row_of(s)).collect()
    }
}

fn selected(c: &Current, row: usize) -> Selected {
    let history = c.session.history();
    let node = history.node(row);
    let relatives = node
        .parents
        .iter()
        .chain(&node.children)
        .map(|sha| Relative {
            sha: sha.clone(),
            label: history.link_label(sha),
            row: history.row_of(sha),
        })
        .collect();
    Selected {
        generation: c.generation,
        node,
        relatives,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use ocelli_core::graph::Layout;
    use ocelli_core::model::{CommitData, CommitKind};

    pub(crate) fn commit(sha: &str, time: i64, parents: &[&str]) -> CommitData {
        CommitData {
            sha: sha.to_string(),
            message: format!("commit {sha}"),
            author_name: "Ada".to_string(),
            author_email: String::new(),
            author_time: time,
            committer_name: "Ada".to_string(),
            committer_email: String::new(),
            commit_time: time,
            refs: vec![],
            parent_shas: parents.iter().map(|p| p.to_string()).collect(),
            kind: CommitKind::Commit,
        }
    }

    /// Three commits `c{top}` down to `c{top - 2}`, each on the next.
    pub(crate) fn three(top: usize) -> Vec<CommitData> {
        (top - 2..=top)
            .rev()
            .map(|i| commit(&format!("c{i}"), i as i64, &[&format!("c{}", i - 1)]))
            .collect()
    }

    fn shas(w: &WindowJson) -> Vec<&str> {
        w.nodes.iter().map(|n| n.sha.as_str()).collect()
    }

    #[test]
    // Verifies: ADR-0031, G-14
    fn a_window_reads_the_rows_of_every_chunk_appended() {
        let session = Session::default();
        let mut layout = Layout::new();
        let g = session.begin();
        let growth = session.append(g, layout.lay_out(three(9))).unwrap();
        assert_eq!((growth.rows, growth.loading), (3, true));
        assert_eq!(
            growth.status,
            "3 commits in 1 chunk — loading older history…"
        );
        session.append(g, layout.lay_out(three(6))).unwrap();
        let w = session.window(2, 2);
        assert_eq!((w.top, shas(&w)), (2, vec!["c7", "c6"]));
        assert_eq!(w.edges.len(), 3, "c8→c7, c7→c6, and c6→c5 below the window");
        assert_eq!(w.lanes, 1);
        assert_eq!((w.lit_nodes, w.lit_edges), (None, None), "nothing selected");
        let done = session.finish(g, Ok(())).unwrap();
        assert_eq!(
            (done.rows, done.loading, done.status.as_str()),
            (6, false, "6 commits")
        );
    }

    #[test]
    // Verifies: ADR-0031, ADR-0016
    fn a_newer_load_refuses_the_older_ones_chunks_and_replaces_its_history_on_its_first() {
        let session = Session::default();
        let old = session.begin();
        session
            .append(old, Layout::new().lay_out(three(9)))
            .unwrap();
        session.select(0).unwrap();
        let new = session.begin();
        assert!(
            session
                .append(old, Layout::new().lay_out(three(6)))
                .is_none()
        );
        assert!(session.finish(old, Ok(())).is_none());
        assert_eq!(
            shas(&session.window(0, 9)),
            ["c9", "c8", "c7"],
            "shown until replaced"
        );
        let growth = session
            .append(new, Layout::new().lay_out(three(4)))
            .unwrap();
        assert_eq!((growth.generation, growth.rows), (new, 3));
        assert_eq!(shas(&session.window(0, 9)), ["c4", "c3", "c2"]);
        assert!(
            session.selection().is_none(),
            "the selection went with the history"
        );
    }

    #[test]
    // Verifies: ADR-0016
    fn a_load_ending_before_its_first_chunk_reports_no_growth() {
        let session = Session::default();
        let g = session.begin();
        assert!(session.finish(g, Ok(())).is_none());
    }

    #[test]
    // Verifies: ADR-0031, G-17
    fn the_lineage_lights_the_window_and_grows_with_the_chunks() {
        let session = Session::default();
        let mut layout = Layout::new();
        let g = session.begin();
        let mut first = three(9);
        first.push(commit("side", 1, &[]));
        session.append(g, layout.lay_out(first)).unwrap();
        let picked = session.select(1).unwrap();
        assert_eq!(picked.node.sha, "c8");
        let w = session.window(0, 4);
        assert_eq!(shas(&w), ["c9", "c8", "c7", "side"]);
        assert_eq!(w.lit_nodes, Some(vec![true, true, true, false]));
        assert!(w.lit_edges.unwrap().iter().all(|&lit| lit));

        session.append(g, layout.lay_out(three(6))).unwrap();
        let w = session.window(4, 3);
        assert_eq!(
            w.lit_nodes,
            Some(vec![true, true, true]),
            "c6 down to c4 are ancestors"
        );
        session.deselect();
        assert_eq!(session.window(0, 2).lit_nodes, None);
    }

    #[test]
    // Verifies: ADR-0031, G-17
    fn the_relatives_of_the_selection_gain_their_rows_as_they_arrive() {
        let session = Session::default();
        let mut layout = Layout::new();
        let g = session.begin();
        session.append(g, layout.lay_out(three(9))).unwrap();
        let picked = session.select(2).unwrap();
        let parent = &picked.relatives[0];
        assert_eq!(
            (parent.sha.as_str(), parent.label.as_str(), parent.row),
            ("c6", "c6", None)
        );
        assert_eq!(picked.relatives[1].row, Some(1), "the child c8");
        session.append(g, layout.lay_out(three(6))).unwrap();
        assert_eq!(session.selection().unwrap().relatives[0].row, Some(3));
        assert_eq!(
            session.rows_of(&["c6".into(), "nope".into()]),
            [Some(3), None]
        );
    }

    #[test]
    // Verifies: ADR-0031, G-16
    fn a_search_finds_every_row_and_lists_the_first_ones() {
        let session = Session::default();
        let mut layout = Layout::new();
        let g = session.begin();
        session.append(g, layout.lay_out(three(9))).unwrap();
        session.append(g, layout.lay_out(three(6))).unwrap();
        let found = session.search("COMMIT", 2);
        assert_eq!(found.rows, [0, 1, 2, 3, 4, 5]);
        let hits: Vec<&str> = found.hits.iter().map(|n| n.sha.as_str()).collect();
        assert_eq!(hits, ["c9", "c8"]);
        assert!(session.search("  ", 2).rows.is_empty());
    }

    #[test]
    // Verifies: ADR-0031, G-16
    fn a_search_asked_again_as_the_history_grows_finds_what_a_fresh_one_does() {
        let session = Session::default();
        let mut layout = Layout::new();
        let g = session.begin();
        session.append(g, layout.lay_out(three(9))).unwrap();
        assert_eq!(session.search("c", 9).rows, [0, 1, 2]);
        session.append(g, layout.lay_out(three(6))).unwrap();
        let grown = session.search("c", 9);
        assert_eq!((grown.generation, grown.rows), (g, vec![0, 1, 2, 3, 4, 5]));
        assert_eq!(
            session.search("c8", 9).rows,
            [1],
            "another query searches afresh"
        );
    }
}
