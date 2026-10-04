//! Application state and key handling, free of any terminal I/O so it can be
//! driven by tests. The functions are those of the desktop application —
//! sidebar, graph with lineage highlight, search, commit detail with
//! parent/child links, flat or tree file list, unified or side-by-side diff,
//! theme switch, open and refresh — with guitar-style keys (ADR-0028). Every
//! key reads; none writes to the repository.

use crate::chips::RefContext;
use crate::explore;
use crate::files::{self, FileRow, FileView};
use crate::loader::{Msg, Request};
use crate::raster::{Glyphs, GraphIndex};
use crate::sidebar::{self, Action, Activated, Item};
use crate::state::{DiffView as StoredDiffView, StoredTheme, ViewState};
use crate::theme::Theme;
use ocelli_core::model::MAX_COMMITS;
use ocelli_core::model::{CommitKind, DiffLine, FileChange, Graph, NodeJson, RepoRefs};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// The pane that receives navigation keys, left to right.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Sidebar,
    Graph,
    Detail,
    Files,
}

const PANES: [Focus; 4] = [Focus::Sidebar, Focus::Graph, Focus::Detail, Focus::Files];

/// A loaded repository: the graph from `ocelli-core` plus the lookups the
/// terminal needs on top of it.
pub struct Loaded {
    pub graph: Graph,
    pub index: GraphIndex,
    pub refs: RefContext,
    pub sidebar: sidebar::Data,
    row_of: HashMap<String, usize>,
}

impl Loaded {
    pub fn new(graph: Graph, repo_refs: RepoRefs) -> Self {
        let index = GraphIndex::new(&graph);
        let row_of = graph
            .nodes
            .iter()
            .map(|n| (n.sha.clone(), n.row as usize))
            .collect();
        let sidebar = sidebar::Data::new(&repo_refs);
        let refs = RefContext {
            head: repo_refs
                .locals
                .iter()
                .find(|b| b.is_head)
                .map(|b| b.name.clone()),
            locals: repo_refs.locals.into_iter().map(|b| b.name).collect(),
        };
        Self {
            graph,
            index,
            refs,
            sidebar,
            row_of,
        }
    }

    pub fn node(&self, row: usize) -> &NodeJson {
        &self.graph.nodes[self.index.node_at(row)]
    }

    pub fn rows(&self) -> usize {
        self.index.rows()
    }

    pub fn row_of(&self, sha: &str) -> Option<usize> {
        self.row_of.get(sha).copied()
    }

    fn by_sha(&self, sha: &str) -> Option<&NodeJson> {
        self.row_of(sha).map(|r| self.node(r))
    }

    /// Whether a sidebar entry can act: a ref needs its commit in the loaded
    /// graph (a ref beyond the commit limit has no row), a switch needs no
    /// blocking reason.
    pub fn can_act(&self, item: &Item) -> bool {
        match &item.action {
            Action::Reveal(sha) => self.row_of.contains_key(sha),
            Action::Open(path) => !path.as_os_str().is_empty(),
            Action::Blocked(_) => false,
        }
    }

    /// The label for a parent or child link. Synthetic nodes have no useful
    /// short SHA, so the working-directory node is named instead.
    pub fn link_label(&self, sha: &str) -> String {
        match self.by_sha(sha) {
            Some(n) if n.kind == CommitKind::Working => "Uncommitted".to_string(),
            Some(n) => n.short_sha.clone(),
            None => sha[..sha.len().min(7)].to_string(),
        }
    }
}

/// The changed files of one commit, as last delivered by the worker.
pub struct Files {
    pub sha: String,
    pub files: Result<Vec<FileChange>, String>,
    /// Selected row of the visible file rows.
    pub selected: usize,
}

/// The diff overlay, covering the graph pane (ADR-0020).
pub struct DiffView {
    pub path: String,
    pub lines: Option<Result<Vec<DiffLine>, String>>,
    pub scroll: usize,
}

/// History search over message, author, committer, SHA and refs.
#[derive(Default)]
pub struct Search {
    pub query: String,
    /// True while keys go into the search field.
    pub editing: bool,
    /// Matching rows, newest first.
    pub matches: Vec<usize>,
    /// Position in `matches` of the last match jumped to.
    pub pos: Option<usize>,
}

pub struct App {
    pub repo: PathBuf,
    pub status: String,
    pub data: Option<Loaded>,
    pub glyphs: Glyphs,
    pub theme: Theme,
    pub focus: Focus,
    pub side: sidebar::State,
    /// Selected graph row, also the cursor while nothing is selected.
    pub selected: usize,
    /// Whether a commit is selected. `Esc` deselects, which lifts the lineage
    /// highlight and empties the detail pane, as on the desktop.
    pub active: bool,
    /// Rows of the selected commit's lineage.
    pub lineage: Option<HashSet<usize>>,
    /// First graph row on screen; kept around the selection by [`App::scroll_to_selection`].
    pub offset: usize,
    /// Height of the graph pane's content in rows, as of the last frame.
    pub page: usize,
    /// Selected parent/child link in the detail pane.
    pub link: usize,
    pub files: Option<Files>,
    pub file_view: FileView,
    pub collapsed_dirs: HashSet<String>,
    pub diff: Option<DiffView>,
    /// Side-by-side instead of unified — kept across files.
    pub diff_split: bool,
    pub search: Search,
    /// Path being typed after `o`; `None` when no prompt is open.
    pub prompt: Option<String>,
    pub quit: bool,
    /// Reads for the worker, drained by the event loop after every key.
    pub requests: Vec<Request>,
    /// A repository to switch to, taken by the event loop, which then starts a
    /// fresh load and drops every result still in flight for the old one.
    pub reload: Option<PathBuf>,
    /// Text for the terminal clipboard, taken by the event loop.
    pub clipboard: Option<String>,
}

impl App {
    pub fn new(repo: PathBuf, glyphs: Glyphs) -> Self {
        Self {
            repo,
            status: "Loading…".to_string(),
            data: None,
            glyphs,
            theme: Theme::Dark,
            focus: Focus::Graph,
            side: sidebar::State::default(),
            selected: 0,
            active: false,
            lineage: None,
            offset: 0,
            page: 20,
            link: 0,
            files: None,
            file_view: FileView::Tree,
            collapsed_dirs: HashSet::new(),
            diff: None,
            diff_split: false,
            search: Search::default(),
            prompt: None,
            quit: false,
            requests: Vec::new(),
            reload: None,
            clipboard: None,
        }
    }

    /// The choices remembered across sessions (ADR-0029).
    pub fn view_state(&self) -> ViewState {
        ViewState {
            theme: match self.theme {
                Theme::Dark => StoredTheme::Dark,
                Theme::Light => StoredTheme::Light,
            },
            sidebar_minimised: self.side.minimised,
            diff_view: if self.diff_split {
                StoredDiffView::Split
            } else {
                StoredDiffView::Unified
            },
        }
    }

    pub fn apply_view_state(&mut self, state: ViewState) {
        self.theme = match state.theme {
            StoredTheme::Dark => Theme::Dark,
            StoredTheme::Light => Theme::Light,
        };
        self.side.minimised = state.sidebar_minimised;
        self.diff_split = state.diff_view == StoredDiffView::Split;
    }

    /// Loads `path`: forgets the current repository's history and selection,
    /// keeps the view preferences, and asks the event loop to start the load.
    fn open_repo(&mut self, path: PathBuf, status: &str) {
        let mut next = App::new(path.clone(), self.glyphs);
        next.theme = self.theme;
        next.page = self.page;
        next.file_view = self.file_view;
        next.diff_split = self.diff_split;
        next.side = self.side.clone();
        next.side.selected = 0;
        next.side.rail_selected = 0;
        next.status = status.to_string();
        next.reload = Some(path);
        *self = next;
    }

    pub fn on_msg(&mut self, msg: Msg) {
        match msg {
            Msg::Progress(p) => self.status = p,
            Msg::Loaded(Ok((graph, repo_refs))) => {
                let n = graph.nodes.len();
                // At the commit limit the load was (or may have been) truncated —
                // say so instead of presenting the cut-off as the whole history.
                self.status = if n >= MAX_COMMITS {
                    format!("{n} commits — commit limit reached, older history not shown")
                } else {
                    format!("{n} commits")
                };
                self.data = Some(Loaded::new(graph, repo_refs));
                self.select(0);
            }
            Msg::Loaded(Err(e)) => self.status = format!("Error: {e}"),
            Msg::Files { sha, files } => {
                // Results for a commit that is no longer selected are stale.
                if self.selected_sha().as_deref() == Some(sha.as_str()) {
                    self.files = Some(Files {
                        sha,
                        files,
                        selected: 0,
                    });
                }
            }
            Msg::Diff { sha, path, lines } => {
                let current = self.files.as_ref().map(|f| f.sha.as_str()) == Some(sha.as_str());
                if let Some(diff) = self.diff.as_mut().filter(|d| d.path == path && current) {
                    diff.lines = Some(lines);
                }
            }
        }
    }

    /// The selected commit, when one is selected.
    pub fn selected_node(&self) -> Option<&NodeJson> {
        let data = self.data.as_ref()?;
        (self.active && data.rows() > 0).then(|| data.node(self.selected))
    }

    fn selected_sha(&self) -> Option<String> {
        self.selected_node().map(|n| n.sha.clone())
    }

    /// Selects a graph row: computes its lineage and asks for its files.
    fn select(&mut self, row: usize) {
        let Some(data) = &self.data else { return };
        if data.rows() == 0 {
            return;
        }
        let row = row.min(data.rows() - 1);
        let changed = !self.active || row != self.selected || self.files.is_none();
        self.selected = row;
        self.active = true;
        if changed {
            let node = data.node(row);
            self.lineage = Some(explore::lineage(node, |sha| data.by_sha(sha)));
            self.files = None;
            self.diff = None;
            self.link = 0;
            self.requests.push(Request::Files {
                sha: node.sha.clone(),
            });
        }
    }

    /// Clears the selection — the lineage highlight lifts, detail empties.
    fn deselect(&mut self) {
        self.active = false;
        self.lineage = None;
        self.files = None;
        self.diff = None;
    }

    /// Keeps the selection inside a viewport of `height` rows.
    pub fn scroll_to_selection(&mut self, height: usize) {
        self.page = height.max(1);
        if self.selected < self.offset {
            self.offset = self.selected;
        } else if self.selected >= self.offset + self.page {
            self.offset = self.selected + 1 - self.page;
        }
    }

    /// The parent links, then the child links, of the selected commit.
    pub fn links(&self) -> Vec<String> {
        self.selected_node()
            .map(|n| n.parents.iter().chain(&n.children).cloned().collect())
            .unwrap_or_default()
    }

    /// The visible rows of the files pane.
    pub fn file_rows(&self) -> Vec<FileRow> {
        match self.files.as_ref().map(|f| &f.files) {
            Some(Ok(list)) => files::rows(list, self.file_view, &self.collapsed_dirs),
            _ => Vec::new(),
        }
    }

    pub fn on_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && key.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        // Text inputs take every key until they close.
        if self.prompt.is_some() {
            return self.prompt_key(key);
        }
        if self.search.editing {
            return self.search_key(key);
        }
        if self.side.editing {
            return self.filter_key(key);
        }
        if ctrl && key.code == KeyCode::Char('f') {
            return self.start_search();
        }
        if self.diff.is_some() {
            return self.diff_key(key, ctrl);
        }
        match key.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('1') => self.side.toggle_minimised(),
            KeyCode::Char('T') => self.theme = self.theme.toggled(),
            KeyCode::Char('o') => self.prompt = Some(self.repo.display().to_string()),
            KeyCode::Char('r') => self.open_repo(self.repo.clone(), "Refreshing…"),
            KeyCode::Char('n') => self.jump_to_match(1),
            KeyCode::Char('N') => self.jump_to_match(-1),
            KeyCode::Char('/') if self.focus != Focus::Sidebar => self.start_search(),
            KeyCode::Tab => self.cycle_focus(1),
            KeyCode::BackTab => self.cycle_focus(-1),
            KeyCode::Esc if self.focus == Focus::Graph => self.deselect(),
            KeyCode::Esc => self.focus = Focus::Graph,
            _ => match self.focus {
                Focus::Sidebar => self.sidebar_key(key),
                Focus::Graph => self.graph_key(key, ctrl),
                Focus::Detail => self.detail_key(key),
                Focus::Files => self.files_key(key),
            },
        }
    }

    fn cycle_focus(&mut self, dir: isize) {
        let at = PANES.iter().position(|&f| f == self.focus).unwrap_or(1) as isize;
        let len = PANES.len() as isize;
        self.focus = PANES[(at + dir).rem_euclid(len) as usize];
    }

    /// `h` / `l`: the pane to the left or right.
    fn move_focus(&mut self, dir: isize) {
        let at = PANES.iter().position(|&f| f == self.focus).unwrap_or(1) as isize;
        let to = (at + dir).clamp(0, PANES.len() as isize - 1);
        self.focus = PANES[to as usize];
    }

    // ── Text inputs ─────────────────────────────────────────────────────────

    fn prompt_key(&mut self, key: KeyEvent) {
        let Some(text) = &mut self.prompt else { return };
        match key.code {
            KeyCode::Char(c) => text.push(c),
            KeyCode::Backspace => {
                text.pop();
            }
            KeyCode::Esc => self.prompt = None,
            KeyCode::Enter => {
                let path = PathBuf::from(text.trim());
                self.prompt = None;
                if !path.as_os_str().is_empty() {
                    self.open_repo(path, "Opening…");
                }
            }
            _ => {}
        }
    }

    fn start_search(&mut self) {
        self.search.editing = true;
    }

    fn search_key(&mut self, key: KeyEvent) {
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        match key.code {
            KeyCode::Char(c) => {
                self.search.query.push(c);
                self.update_matches();
            }
            KeyCode::Backspace => {
                self.search.query.pop();
                self.update_matches();
            }
            // Enter jumps to the next match, Shift+Enter to the previous; both wrap.
            KeyCode::Enter => self.jump_to_match(if shift { -1 } else { 1 }),
            KeyCode::Down => self.jump_to_match(1),
            KeyCode::Up => self.jump_to_match(-1),
            // Leave the field; the query and its matches stay for `n` / `N`.
            KeyCode::Tab => self.search.editing = false,
            KeyCode::Esc => self.search = Search::default(),
            _ => {}
        }
    }

    fn update_matches(&mut self) {
        self.search.pos = None;
        self.search.matches = match &self.data {
            Some(data) => explore::search(&data.graph.nodes, &self.search.query),
            None => Vec::new(),
        };
    }

    fn jump_to_match(&mut self, dir: isize) {
        let len = self.search.matches.len();
        if len == 0 {
            return;
        }
        let pos = match self.search.pos {
            None if dir < 0 => len - 1,
            None => 0,
            Some(p) => (p as isize + dir).rem_euclid(len as isize) as usize,
        };
        self.search.pos = Some(pos);
        self.select(self.search.matches[pos]);
    }

    fn filter_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char(c) => self.side.filter.push(c),
            KeyCode::Backspace => {
                self.side.filter.pop();
            }
            KeyCode::Enter => self.side.editing = false,
            KeyCode::Esc => {
                self.side.filter.clear();
                self.side.editing = false;
            }
            _ => {}
        }
        self.side.selected = 0;
    }

    // ── Panes ───────────────────────────────────────────────────────────────

    fn sidebar_key(&mut self, key: KeyEvent) {
        let Some(data) = &self.data else { return };
        let minimised = self.side.minimised;
        let len = if minimised {
            sidebar::rail(&data.sidebar).len()
        } else {
            sidebar::rows(&data.sidebar, &self.side).len()
        };
        let sel = if minimised {
            &mut self.side.rail_selected
        } else {
            &mut self.side.selected
        };
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => *sel = (*sel + 1).min(len.saturating_sub(1)),
            KeyCode::Char('k') | KeyCode::Up => *sel = sel.saturating_sub(1),
            KeyCode::Char('g') | KeyCode::Home => *sel = 0,
            KeyCode::Char('G') | KeyCode::End => *sel = len.saturating_sub(1),
            KeyCode::Char('/') if !minimised => self.side.editing = true,
            KeyCode::Char('l') | KeyCode::Right if minimised => self.activate_sidebar(),
            KeyCode::Char('l') | KeyCode::Right => self.move_focus(1),
            KeyCode::Enter | KeyCode::Char(' ') => self.activate_sidebar(),
            _ => {}
        }
    }

    fn activate_sidebar(&mut self) {
        let Some(data) = &self.data else { return };
        let activated = if self.side.minimised {
            sidebar::activate_rail(&data.sidebar, &mut self.side)
        } else {
            sidebar::activate(&data.sidebar, &mut self.side)
        };
        let Activated::Item(item) = activated else {
            return;
        };
        match item.action {
            Action::Reveal(sha) => match data.row_of(&sha) {
                Some(row) => {
                    self.select(row);
                    self.focus = Focus::Graph;
                }
                // At the commit limit a ref can point at history never read (ADR-0016).
                None => self.status = format!("{} — outside the loaded history", item.name),
            },
            Action::Open(path) => self.open_repo(path, "Opening…"),
            Action::Blocked(why) => self.status = format!("{} — {why}", item.name),
        }
    }

    fn graph_key(&mut self, key: KeyEvent, ctrl: bool) {
        let Some(data) = &self.data else { return };
        let row = self.selected;
        let half = (self.page / 2).max(1);
        let target = match key.code {
            // With nothing selected, the first move selects the cursor's row.
            KeyCode::Char('j') | KeyCode::Down if !self.active => Some(row),
            KeyCode::Char('j') | KeyCode::Down => Some(row + 1),
            KeyCode::Char('k') | KeyCode::Up if !self.active => Some(row),
            KeyCode::Char('k') | KeyCode::Up => Some(row.saturating_sub(1)),
            KeyCode::Char('d') if ctrl => Some(row + half),
            KeyCode::Char('u') if ctrl => Some(row.saturating_sub(half)),
            KeyCode::PageDown => Some(row + self.page),
            KeyCode::PageUp => Some(row.saturating_sub(self.page)),
            KeyCode::Char('g') | KeyCode::Home => Some(0),
            KeyCode::Char('G') | KeyCode::End => Some(usize::MAX),
            // First parent / newest child, when it is part of the loaded history.
            KeyCode::Char(']') => data.node(row).parents.first().and_then(|p| data.row_of(p)),
            KeyCode::Char('[') => data.node(row).children.first().and_then(|c| data.row_of(c)),
            // Next / previous commit that carries a ref.
            KeyCode::Char('}') => (row + 1..data.rows()).find(|&r| !data.node(r).refs.is_empty()),
            KeyCode::Char('{') => (0..row).rev().find(|&r| !data.node(r).refs.is_empty()),
            KeyCode::Char('y') => {
                self.copy_sha();
                None
            }
            KeyCode::Enter => {
                if !self.active {
                    self.select(row);
                }
                self.focus = Focus::Detail;
                None
            }
            KeyCode::Char('l') | KeyCode::Right => {
                self.move_focus(1);
                None
            }
            KeyCode::Char('h') | KeyCode::Left => {
                self.move_focus(-1);
                None
            }
            _ => None,
        };
        if let Some(t) = target {
            self.select(t);
        }
    }

    /// Puts the full SHA of the selected commit on the clipboard. The
    /// working-directory node has no object id to copy.
    fn copy_sha(&mut self) {
        let Some(node) = self.selected_node() else {
            return;
        };
        if node.kind == CommitKind::Working {
            return;
        }
        let sha = node.sha.clone();
        self.status = format!("Copied {sha}");
        self.clipboard = Some(sha);
    }

    fn detail_key(&mut self, key: KeyEvent) {
        let links = self.links();
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                self.link = (self.link + 1).min(links.len().saturating_sub(1))
            }
            KeyCode::Char('k') | KeyCode::Up => self.link = self.link.saturating_sub(1),
            KeyCode::Char('y') => self.copy_sha(),
            KeyCode::Enter => {
                let Some(sha) = links.get(self.link) else {
                    return;
                };
                match self.data.as_ref().and_then(|d| d.row_of(sha)) {
                    Some(row) => self.select(row),
                    None => {
                        let short = &sha[..sha.len().min(7)];
                        self.status = format!("{short} — outside the loaded history");
                    }
                }
            }
            KeyCode::Char('h') | KeyCode::Left => self.move_focus(-1),
            KeyCode::Char('l') | KeyCode::Right => self.move_focus(1),
            _ => {}
        }
    }

    fn files_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('h') | KeyCode::Left => return self.move_focus(-1),
            KeyCode::Char('t') => {
                self.file_view = match self.file_view {
                    FileView::Flat => FileView::Tree,
                    FileView::Tree => FileView::Flat,
                };
                if let Some(f) = &mut self.files {
                    f.selected = 0;
                }
                return;
            }
            _ => {}
        }
        let rows = self.file_rows();
        let Some(files) = &mut self.files else { return };
        if rows.is_empty() {
            return;
        }
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                files.selected = (files.selected + 1).min(rows.len() - 1)
            }
            KeyCode::Char('k') | KeyCode::Up => files.selected = files.selected.saturating_sub(1),
            KeyCode::Char('g') | KeyCode::Home => files.selected = 0,
            KeyCode::Char('G') | KeyCode::End => files.selected = rows.len() - 1,
            KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right | KeyCode::Char(' ') => {
                match &rows[files.selected.min(rows.len() - 1)] {
                    FileRow::Dir { path, .. } => {
                        if !self.collapsed_dirs.remove(path) {
                            self.collapsed_dirs.insert(path.clone());
                        }
                    }
                    FileRow::File { index, .. } => {
                        let Ok(list) = &files.files else { return };
                        let path = list[*index].path.clone();
                        self.requests.push(Request::Diff {
                            sha: files.sha.clone(),
                            path: path.clone(),
                        });
                        self.diff = Some(DiffView {
                            path,
                            lines: None,
                            scroll: 0,
                        });
                    }
                }
            }
            _ => {}
        }
    }

    fn diff_key(&mut self, key: KeyEvent, ctrl: bool) {
        let page = self.page;
        if key.code == KeyCode::Char('s') {
            self.diff_split = !self.diff_split;
            return;
        }
        let Some(diff) = &mut self.diff else { return };
        let len = match &diff.lines {
            Some(Ok(lines)) => lines.len(),
            _ => 0,
        };
        let max = len.saturating_sub(page);
        let half = (page / 2).max(1);
        diff.scroll = match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('h') | KeyCode::Left => {
                self.diff = None;
                return;
            }
            KeyCode::Char('j') | KeyCode::Down => diff.scroll + 1,
            KeyCode::Char('k') | KeyCode::Up => diff.scroll.saturating_sub(1),
            KeyCode::Char('d') if ctrl => diff.scroll + half,
            KeyCode::Char('u') if ctrl => diff.scroll.saturating_sub(half),
            KeyCode::PageDown | KeyCode::Char(' ') => diff.scroll + page,
            KeyCode::PageUp => diff.scroll.saturating_sub(page),
            KeyCode::Char('g') | KeyCode::Home => 0,
            KeyCode::Char('G') | KeyCode::End => max,
            _ => diff.scroll,
        }
        .min(max);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use ocelli_core::graph::build_graph;
    use ocelli_core::model::{CommitData, FileStatus};

    pub(crate) fn commit(sha: &str, time: i64, parents: &[&str], refs: &[&str]) -> CommitData {
        CommitData {
            sha: sha.to_string(),
            message: format!("commit {sha}"),
            author_name: "Ada".to_string(),
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

    /// Rows: d (merge, `main`), c (side, `🏷 v1`), b, a — with the sidebar
    /// refs of `sidebar::tests::repo_refs`.
    pub(crate) fn loaded_app() -> App {
        let graph = build_graph(vec![
            commit("a", 1, &[], &[]),
            commit("b", 2, &["a"], &[]),
            commit("c", 3, &["a"], &["🏷 v1"]),
            commit("d", 4, &["b", "c"], &["main"]),
        ]);
        let refs = crate::sidebar::tests::repo_refs();
        let mut app = App::new("/repo".into(), Glyphs::Unicode);
        app.on_msg(Msg::Loaded(Ok((graph, refs))));
        app
    }

    pub(crate) fn press(app: &mut App, code: KeyCode) {
        app.on_key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    fn type_text(app: &mut App, text: &str) {
        for c in text.chars() {
            press(app, KeyCode::Char(c));
        }
    }

    fn sha(app: &App) -> String {
        app.data.as_ref().unwrap().node(app.selected).sha.clone()
    }

    fn rows_of(app: &App, shas: &[&str]) -> HashSet<usize> {
        let data = app.data.as_ref().unwrap();
        shas.iter().map(|s| data.row_of(s).unwrap()).collect()
    }

    #[test]
    fn loading_selects_the_newest_commit_and_requests_its_files() {
        let app = loaded_app();
        assert_eq!(app.status, "4 commits");
        assert_eq!(sha(&app), "d");
        assert!(app.active);
        assert_eq!(app.requests, [Request::Files { sha: "d".into() }]);
    }

    #[test]
    fn brackets_walk_to_first_parent_and_newest_child() {
        let mut app = loaded_app();
        press(&mut app, KeyCode::Char(']'));
        assert_eq!(sha(&app), "b", "first parent of the merge");
        press(&mut app, KeyCode::Char(']'));
        assert_eq!(sha(&app), "a");
        press(&mut app, KeyCode::Char('['));
        assert_eq!(sha(&app), "c", "newest child of a");
    }

    #[test]
    fn braces_jump_between_ref_bearing_commits() {
        let mut app = loaded_app();
        press(&mut app, KeyCode::Char('}'));
        assert_eq!(sha(&app), "c");
        press(&mut app, KeyCode::Char('}'));
        assert_eq!(sha(&app), "c", "no further ref below");
        press(&mut app, KeyCode::Char('{'));
        assert_eq!(sha(&app), "d");
    }

    #[test]
    fn movement_is_clamped_and_the_viewport_follows() {
        let mut app = loaded_app();
        press(&mut app, KeyCode::Char('k'));
        assert_eq!(app.selected, 0);
        press(&mut app, KeyCode::Char('G'));
        assert_eq!(app.selected, 3);
        app.scroll_to_selection(2);
        assert_eq!(app.offset, 2);
        press(&mut app, KeyCode::Char('g'));
        app.scroll_to_selection(2);
        assert_eq!(app.offset, 0);
    }

    #[test]
    fn selecting_highlights_the_lineage_and_escape_lifts_it() {
        let mut app = loaded_app();
        press(&mut app, KeyCode::Char('G')); // a
        press(&mut app, KeyCode::Char('k')); // b
        assert_eq!(app.lineage, Some(rows_of(&app, &["a", "b", "d"])));
        press(&mut app, KeyCode::Esc);
        assert!(!app.active && app.lineage.is_none() && app.selected_node().is_none());
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(sha(&app), "b", "the first move reselects the cursor's row");
    }

    fn deliver_files(app: &mut App, sha: &str, paths: &[&str]) {
        let files = paths
            .iter()
            .map(|p| FileChange {
                path: p.to_string(),
                old_path: None,
                status: FileStatus::Modified,
                additions: 1,
                deletions: 0,
            })
            .collect();
        app.on_msg(Msg::Files {
            sha: sha.into(),
            files: Ok(files),
        });
    }

    #[test]
    fn stale_file_lists_are_ignored() {
        let mut app = loaded_app();
        press(&mut app, KeyCode::Char('j'));
        deliver_files(&mut app, "d", &["x.rs"]);
        assert!(app.files.is_none());
        let current = sha(&app);
        deliver_files(&mut app, &current, &["x.rs"]);
        assert!(app.files.is_some());
    }

    #[test]
    fn tree_folders_collapse_and_files_open_their_diff() {
        let mut app = loaded_app();
        deliver_files(&mut app, "d", &["src/a.rs", "src/b.rs", "README.md"]);
        app.requests.clear();
        app.focus = Focus::Files;
        press(&mut app, KeyCode::Enter); // the `src` folder
        assert_eq!(app.file_rows().len(), 2, "src collapsed: src/, README.md");
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Enter);
        assert_eq!(
            app.requests,
            [Request::Diff {
                sha: "d".into(),
                path: "src/b.rs".into()
            }]
        );
        press(&mut app, KeyCode::Char('s'));
        assert!(app.diff_split, "s switches the diff to side-by-side");
        press(&mut app, KeyCode::Esc);
        assert!(app.diff.is_none() && !app.quit);
    }

    #[test]
    fn t_switches_between_tree_and_flat_files() {
        let mut app = loaded_app();
        deliver_files(&mut app, "d", &["src/a.rs"]);
        app.focus = Focus::Files;
        assert_eq!(app.file_rows().len(), 2);
        press(&mut app, KeyCode::Char('t'));
        assert_eq!(app.file_view, FileView::Flat);
        assert_eq!(app.file_rows().len(), 1);
    }

    #[test]
    fn search_cycles_through_matches_and_n_continues_after_leaving_the_field() {
        let mut app = loaded_app();
        app.on_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL));
        assert!(app.search.editing);
        type_text(&mut app, "commit");
        assert_eq!(app.search.matches.len(), 4);
        press(&mut app, KeyCode::Enter);
        assert_eq!(sha(&app), "d");
        press(&mut app, KeyCode::Enter);
        assert_eq!(sha(&app), "c");
        app.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT));
        assert_eq!(sha(&app), "d", "Shift+Enter goes back");
        press(&mut app, KeyCode::Tab);
        assert!(!app.search.editing);
        press(&mut app, KeyCode::Char('N'));
        assert_eq!(sha(&app), "a", "N wraps to the last match");
        press(&mut app, KeyCode::Char('/'));
        press(&mut app, KeyCode::Esc);
        assert!(app.search.query.is_empty() && app.search.matches.is_empty());
    }

    #[test]
    fn detail_links_jump_to_parents_and_children() {
        let mut app = loaded_app();
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.focus, Focus::Detail);
        assert_eq!(app.links(), ["b", "c"], "parents of the merge, no children");
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Enter);
        assert_eq!(sha(&app), "c");
        assert_eq!(app.links(), ["a", "d"], "parent a, child d");
    }

    #[test]
    fn y_copies_the_full_sha() {
        let mut app = loaded_app();
        press(&mut app, KeyCode::Char('y'));
        assert_eq!(app.clipboard.as_deref(), Some("d"));
        assert_eq!(app.status, "Copied d");
    }

    /// Focuses the sidebar and selects the row whose label is `label`.
    fn side_select(app: &mut App, label: &str) {
        app.focus = Focus::Sidebar;
        let data = app.data.as_ref().unwrap();
        let rows = sidebar::rows(&data.sidebar, &app.side);
        app.side.selected = rows
            .iter()
            .position(|r| r.label == label)
            .unwrap_or_else(|| panic!("no row {label:?}"));
    }

    #[test]
    fn a_tag_in_the_sidebar_reveals_its_commit() {
        let mut app = loaded_app();
        side_select(&mut app, "Tags");
        press(&mut app, KeyCode::Enter);
        side_select(&mut app, "v1");
        press(&mut app, KeyCode::Enter);
        assert_eq!(sha(&app), "c");
        assert_eq!(app.focus, Focus::Graph);
    }

    #[test]
    fn the_sidebar_filter_takes_typed_text() {
        let mut app = loaded_app();
        app.focus = Focus::Sidebar;
        press(&mut app, KeyCode::Char('/'));
        type_text(&mut app, "v1q");
        press(&mut app, KeyCode::Backspace);
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.side.filter, "v1");
        assert!(!app.side.editing && !app.quit, "q went into the filter");
    }

    #[test]
    fn a_submodule_opens_its_repository_and_keeps_view_preferences() {
        let mut app = loaded_app();
        app.theme = Theme::Light;
        app.file_view = FileView::Flat;
        side_select(&mut app, "Submodules");
        press(&mut app, KeyCode::Enter);
        side_select(&mut app, "a");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.reload, Some(PathBuf::from("/super/lib/a")));
        assert_eq!(app.repo, PathBuf::from("/super/lib/a"));
        assert!(app.data.is_none() && app.files.is_none() && app.requests.is_empty());
        assert_eq!((app.theme, app.file_view), (Theme::Light, FileView::Flat));
    }

    #[test]
    fn blocked_rows_explain_themselves_instead_of_acting() {
        let mut app = loaded_app();
        side_select(&mut app, "Submodules");
        press(&mut app, KeyCode::Enter);
        side_select(&mut app, "b");
        press(&mut app, KeyCode::Enter);
        assert!(app.reload.is_none());
        assert_eq!(
            app.status,
            "lib/b — not initialized — no working copy to open"
        );
    }

    #[test]
    fn the_parent_row_leads_back_out() {
        let mut app = loaded_app();
        side_select(&mut app, "↑ super");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.reload, Some(PathBuf::from("/super")));
    }

    #[test]
    fn key_1_minimises_the_sidebar_to_its_rail_and_a_glyph_expands_it() {
        let mut app = loaded_app();
        press(&mut app, KeyCode::Char('1'));
        assert!(app.side.minimised);
        app.focus = Focus::Sidebar;
        app.side.rail_selected = 4; // Tags
        press(&mut app, KeyCode::Enter);
        assert!(!app.side.minimised, "a rail glyph expands the sidebar");
        let data = app.data.as_ref().unwrap();
        let rows = sidebar::rows(&data.sidebar, &app.side);
        assert_eq!(rows[app.side.selected].label, "Tags");
    }

    #[test]
    fn o_opens_a_path_prompt_and_r_reloads_the_current_repository() {
        let mut app = loaded_app();
        press(&mut app, KeyCode::Char('o'));
        assert_eq!(app.prompt.as_deref(), Some("/repo"));
        for _ in 0.."/repo".len() {
            press(&mut app, KeyCode::Backspace);
        }
        type_text(&mut app, "/other");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.reload, Some(PathBuf::from("/other")));
        assert_eq!(app.status, "Opening…");

        let mut app = loaded_app();
        press(&mut app, KeyCode::Char('r'));
        assert_eq!(app.reload, Some(PathBuf::from("/repo")));
        assert_eq!(app.status, "Refreshing…");
    }

    #[test]
    fn capital_t_switches_the_theme() {
        let mut app = loaded_app();
        press(&mut app, KeyCode::Char('T'));
        assert_eq!(app.theme, Theme::Light);
    }

    #[test]
    // Verifies: G-13
    fn the_commit_limit_is_reported() {
        let mut commits = vec![commit("c0", 0, &[], &[])];
        for i in 1..MAX_COMMITS {
            let parent = format!("c{}", i - 1);
            commits.push(commit(&format!("c{i}"), i as i64, &[parent.as_str()], &[]));
        }
        let mut app = App::new("/repo".into(), Glyphs::Unicode);
        app.on_msg(Msg::Loaded(Ok((build_graph(commits), RepoRefs::default()))));
        assert!(
            app.status.contains("commit limit reached"),
            "{}",
            app.status
        );
    }

    #[test]
    fn the_remembered_view_state_round_trips_through_the_app() {
        let mut app = loaded_app();
        assert_eq!(app.view_state(), ViewState::default());
        press(&mut app, KeyCode::Char('T'));
        press(&mut app, KeyCode::Char('1'));
        let state = app.view_state();
        assert_eq!(state.theme, StoredTheme::Light);
        assert!(state.sidebar_minimised);

        let mut fresh = App::new("/repo".into(), Glyphs::Unicode);
        fresh.apply_view_state(ViewState {
            diff_view: StoredDiffView::Split,
            ..state
        });
        assert_eq!(fresh.theme, Theme::Light);
        assert!(fresh.side.minimised && fresh.diff_split);
    }
}
