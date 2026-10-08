//! Application state and key handling, free of any terminal I/O so it can be
//! driven by tests. The functions are those of the desktop application —
//! sidebar, graph with lineage highlight, search, commit detail with
//! parent/child links, flat or tree file list, unified or side-by-side diff,
//! theme switch, open and refresh — with guitar-style keys (ADR-0028). Every
//! key reads but the writes (ADR-0033): `t` tags the selected commit, `s`
//! stashes the uncommitted changes, `a` applies the selected stash, and `p`
//! and `d` on it pop and drop it, as `d` on a tag in the sidebar deletes it,
//! once the developer has answered `y` (ADR-0034).

use crate::chips::RefContext;
use crate::files::{self, FileRow, FileView};
use crate::loader::{Msg, Request};
use crate::raster::Glyphs;
use crate::sidebar::{self, Action, Activated, Item};
use crate::state::{DiffView as StoredDiffView, StoredTheme, ViewState};
use crate::theme::Theme;
use ocelli_core::graph::LaidChunk;
use ocelli_core::history::{History, Lineage};
use ocelli_core::load::{Event, LoadSession};
use ocelli_core::model::{
    CommitKind, DiffLine, FileChange, LocalBranch, NodeJson, RepoRefs, Shown, Tag,
};
use ocelli_core::port::{Confirmation, Write};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::collections::HashSet;
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

/// A loaded repository: the load session the core keeps (ADR-0031), growing
/// chunk by chunk while the load runs, plus the refs the terminal shows with it.
pub struct Loaded {
    pub session: LoadSession,
    /// The most lanes the graph pane has shown, which its width follows.
    pub shown_lanes: usize,
    pub refs: RefContext,
    pub sidebar: sidebar::Data,
    /// The tags as read, for what deleting one discards.
    pub tags: Vec<Tag>,
    /// The local branches as read, for where one points and whether it is merged.
    pub locals: Vec<LocalBranch>,
    /// The remote a fetch reaches without one named.
    pub default_remote: Option<String>,
}

impl Loaded {
    /// The first chunk, shown before the refs arrive (ADR-0016).
    pub fn new(first_chunk: LaidChunk, select_again: Option<String>) -> Self {
        let mut session = LoadSession::new();
        if let Some(sha) = select_again {
            session.select_again(sha);
        }
        session.append(first_chunk);
        Self {
            session,
            shown_lanes: 0,
            refs: RefContext {
                head: None,
                locals: Default::default(),
            },
            sidebar: sidebar::Data::new(&RepoRefs::default()),
            tags: Vec::new(),
            locals: Vec::new(),
            default_remote: None,
        }
    }

    /// Takes in the refs the load read right after the first chunk.
    pub fn set_refs(&mut self, repo_refs: RepoRefs) {
        self.sidebar = sidebar::Data::new(&repo_refs);
        self.tags = repo_refs.tags.clone();
        self.locals = repo_refs.locals.clone();
        self.default_remote = repo_refs.default_remote.clone();
        self.refs = RefContext {
            head: repo_refs
                .locals
                .iter()
                .find(|b| b.is_head)
                .map(|b| b.name.clone()),
            locals: repo_refs.locals.into_iter().map(|b| b.name).collect(),
        };
    }

    pub fn history(&self) -> &History {
        self.session.history()
    }

    pub fn node(&self, row: usize) -> NodeJson {
        self.history().node(row)
    }

    pub fn rows(&self) -> usize {
        self.history().rows()
    }

    pub fn row_of(&self, sha: &str) -> Option<usize> {
        self.history().row_of(sha)
    }

    /// Whether a sidebar entry can act: a ref needs its commit laid out (a ref
    /// to history still loading has no row yet), a switch needs no blocking
    /// reason.
    pub fn can_act(&self, item: &Item) -> bool {
        match &item.action {
            Action::Reveal(sha) => self.row_of(sha).is_some(),
            Action::Open(path) => !path.as_os_str().is_empty(),
            Action::Blocked(_) => false,
        }
    }

    /// The label for a parent or child link.
    pub fn link_label(&self, sha: &str) -> String {
        self.history().link_label(sha)
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

/// The tag asked for after `t` (G-7): its name, then its message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagPrompt {
    pub sha: String,
    pub name: String,
    /// `None` while the name is typed; the message once it is.
    pub message: Option<String>,
}

/// The stash asked for after `s` (G-7): its message, and whether the untracked
/// files go with it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StashPrompt {
    pub message: String,
    pub untracked: bool,
}

/// What takes every key until it closes: a prompt, or the question before a
/// write that discards work. One at a time, by its type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Modal {
    /// The path typed after `o`.
    Open(String),
    /// The tag typed after `t`.
    Tag(TagPrompt),
    /// The stash described after `s`.
    Stash(StashPrompt),
    /// A write that discards work, waiting for the developer's answer (ADR-0034).
    Confirm(Write),
    /// The message typed after `c` for a commit on `head`.
    Commit { head: String, message: String },
    /// The name typed after `b` for a branch at `commit`, switched to where `switch` says so.
    Branch {
        commit: String,
        name: String,
        switch: bool,
    },
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
    pub lineage: Option<Lineage>,
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
    /// The prompt or question open; `None` when keys go to the panes.
    pub modal: Option<Modal>,
    pub quit: bool,
    /// Reads for the worker, drained by the event loop after every key.
    pub requests: Vec<Request>,
    /// A repository to switch to, taken by the event loop, which then starts a
    /// fresh load and drops every result still in flight for the old one.
    pub reload: Option<PathBuf>,
    /// A write for the event loop to run; its result comes back as [`Msg::Written`].
    pub write: Option<(Write, Confirmation)>,
    /// Whether a write runs: no second one starts until its result is in.
    pub writing: bool,
    /// Asks the event loop to stop the write that runs, taken by it.
    pub cancel: bool,
    /// Whether the write that runs can be stopped: one that reaches a remote.
    pub cancellable: bool,
    /// Whether the write that runs reaches a remote, after which the repository is read again
    /// even where it is refused.
    pub transferring: bool,
    /// What the status bar says over the load's own line until the next key: why a transfer
    /// was refused, which the reload after it would otherwise hide.
    pub notice: Option<String>,
    /// The commit a write left selected, handed to the core's load session of the reload with
    /// its first chunk; the session names its row once laid out (ADR-0033).
    pub select_again: Option<String>,
    /// Text for the terminal clipboard, taken by the event loop.
    pub clipboard: Option<String>,
}

/// The character a key types: one without a modifier, or one AltGr types,
/// which some terminals report as Ctrl+Alt (crossterm on Windows), as `[`, `{`
/// and `@` are on many European layouts. A letter or a digit held with Ctrl or
/// Alt types nothing; it is a chord or a slip.
fn typed(key: KeyEvent) -> Option<char> {
    let KeyCode::Char(c) = key.code else {
        return None;
    };
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    match (ctrl, alt) {
        (false, false) => Some(c),
        (true, true) if !c.is_ascii_alphanumeric() => Some(c),
        _ => None,
    }
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
            modal: None,
            quit: false,
            requests: Vec::new(),
            reload: None,
            write: None,
            writing: false,
            cancel: false,
            cancellable: false,
            transferring: false,
            notice: None,
            select_again: None,
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
        next.writing = self.writing;
        next.transferring = self.transferring;
        *self = next;
    }

    pub fn on_msg(&mut self, msg: Msg) {
        match msg {
            Msg::Load(event) => self.on_load(event),
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
            Msg::WriteProgress(p) if self.writing => self.status = p,
            Msg::WriteProgress(_) => {}
            Msg::Written(Err(e)) => {
                self.writing = false;
                self.cancellable = false;
                // A transfer refused may have moved remote branches by its fetch: the
                // repository is read again, the refusal kept in view (ADR-0035).
                if std::mem::take(&mut self.transferring) {
                    let select_again = self.selected_sha();
                    self.open_repo(self.repo.clone(), "Reloading…");
                    self.select_again = select_again;
                    self.notice = Some(format!("Not written: {e}"));
                } else {
                    self.status = format!("Not written: {e}");
                }
            }
            // The repository is read again, the commit the write made, or else
            // the one that was selected, selected again once its row arrives
            // (ADR-0033).
            Msg::Written(Ok(made)) => {
                self.writing = false;
                self.cancellable = false;
                self.transferring = false;
                let select_again = made.or_else(|| self.selected_sha());
                self.open_repo(self.repo.clone(), "Written — reloading…");
                self.select_again = select_again;
            }
            Msg::Diff { sha, path, lines } => {
                let current = self.files.as_ref().map(|f| f.sha.as_str()) == Some(sha.as_str());
                if let Some(diff) = self.diff.as_mut().filter(|d| d.path == path && current) {
                    diff.lines = Some(lines);
                }
            }
        }
    }

    /// Takes in how the load goes (ADR-0016).
    fn on_load(&mut self, event: Event) {
        self.take_in(event);
        if let Some(row) = self.data.as_mut().and_then(|d| d.session.row_to_select()) {
            self.show(row);
        }
    }

    fn take_in(&mut self, event: Event) {
        match (event, self.data.as_mut()) {
            // Once the first chunk is shown, the progress line counts chunks.
            (Event::Progress(p), None) => self.status = p,
            (Event::Progress(_), Some(_)) => {}
            (Event::Chunk(chunk), None) => {
                let data = Loaded::new(chunk, self.select_again.take());
                self.status = data.session.status();
                self.data = Some(data);
                self.show(0);
                // A query typed while the history was loading.
                self.update_matches();
            }
            (Event::Chunk(chunk), Some(data)) => {
                let from = data.session.append(chunk);
                self.status = data.session.status();
                // The rows laid out stay where they are, so the selection and
                // the match jumped to stay too; the search and the lineage grow
                // by what they reach in the rows just added.
                let history = data.session.history();
                if let Some(lineage) = self.lineage.as_mut() {
                    history.extend_lineage(lineage, from);
                }
                let found = history.search_from(&self.search.query, from);
                self.search.matches.extend(found);
            }
            (Event::Refs(refs), Some(data)) => data.set_refs(*refs),
            (Event::Refs(_), None) => {}
            (Event::Ended(Err(e)), None) => self.status = format!("Error: {e}"),
            (Event::Ended(Ok(())), None) => {}
            (Event::Ended(result), Some(data)) => {
                data.session.end(result);
                self.status = data.session.status();
            }
        }
    }

    /// The selected commit, when one is selected.
    pub fn selected_node(&self) -> Option<NodeJson> {
        let data = self.data.as_ref()?;
        (self.active && data.rows() > 0).then(|| data.node(self.selected))
    }

    fn selected_sha(&self) -> Option<String> {
        self.selected_node().map(|n| n.sha.clone())
    }

    /// Selects a graph row the developer chose, which wins over a commit a write left to select
    /// again (ADR-0033).
    fn select(&mut self, row: usize) {
        if let Some(data) = self.data.as_mut() {
            data.session.keep_selection();
        }
        self.show(row);
    }

    /// Selects a graph row: computes its lineage and asks for its files.
    fn show(&mut self, row: usize) {
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
            self.lineage = Some(data.history().lineage(row));
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
        self.notice = None;
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && key.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        // A prompt or a question takes every key until it closes; its handler
        // puts it back while it stays open.
        if let Some(modal) = self.modal.take() {
            return match modal {
                Modal::Open(path) => self.prompt_key(path, key),
                Modal::Tag(tag) => self.tag_key(tag, key),
                Modal::Stash(stash) => self.stash_key(stash, key),
                Modal::Confirm(write) => self.answer(write, key),
                Modal::Commit { head, message } => self.commit_key(head, message, key),
                Modal::Branch {
                    commit,
                    name,
                    switch,
                } => self.branch_key(commit, name, switch, key),
            };
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
        // A character held with Ctrl or Alt reaches the panes only as a chord
        // one knows, the graph's Ctrl+D and Ctrl+U, or as one AltGr typed;
        // every other one does nothing, so no slip of a modifier writes or asks.
        let chord = key.modifiers == KeyModifiers::CONTROL
            && self.focus == Focus::Graph
            && matches!(key.code, KeyCode::Char('d' | 'u'));
        if matches!(key.code, KeyCode::Char(_)) && typed(key).is_none() && !chord {
            return;
        }
        if self.cancellable && key.code == KeyCode::Esc {
            self.cancel = true;
            self.status = "Cancelling…".to_string();
            return;
        }
        match key.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('f') if self.focus != Focus::Sidebar => self.start_fetch(None),
            KeyCode::Char('U') => self.start_pull(),
            KeyCode::Char('P') => self.start_push(),
            KeyCode::Char('1') => self.side.toggle_minimised(),
            KeyCode::Char('T') => self.theme = self.theme.toggled(),
            KeyCode::Char('o') => self.modal = Some(Modal::Open(self.repo.display().to_string())),
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

    /// The modals the tests look into.
    #[cfg(test)]
    pub fn prompt(&self) -> Option<&str> {
        match &self.modal {
            Some(Modal::Open(path)) => Some(path),
            _ => None,
        }
    }

    #[cfg(test)]
    pub fn tag_prompt(&self) -> Option<&TagPrompt> {
        match &self.modal {
            Some(Modal::Tag(tag)) => Some(tag),
            _ => None,
        }
    }

    #[cfg(test)]
    pub fn stash_prompt(&self) -> Option<&StashPrompt> {
        match &self.modal {
            Some(Modal::Stash(stash)) => Some(stash),
            _ => None,
        }
    }

    #[cfg(test)]
    pub fn confirm(&self) -> Option<&Write> {
        match &self.modal {
            Some(Modal::Confirm(write)) => Some(write),
            _ => None,
        }
    }

    fn prompt_key(&mut self, mut text: String, key: KeyEvent) {
        match key.code {
            KeyCode::Char(_) => text.extend(typed(key)),
            KeyCode::Backspace => {
                text.pop();
            }
            KeyCode::Esc => return,
            KeyCode::Enter => {
                let path = PathBuf::from(text.trim());
                if !path.as_os_str().is_empty() {
                    self.open_repo(path, "Opening…");
                }
                return;
            }
            _ => {}
        }
        self.modal = Some(Modal::Open(text));
    }

    /// `t`: asks for a tag on the selected commit. The working-directory node
    /// and stash nodes are no commits a tag could name.
    fn start_tag(&mut self) {
        let Some(node) = self.selected_node() else {
            return;
        };
        if node.kind != CommitKind::Commit {
            self.status = "Only a commit can be tagged".to_string();
            return;
        }
        self.modal = Some(Modal::Tag(TagPrompt {
            sha: node.sha.clone(),
            name: String::new(),
            message: None,
        }));
    }

    fn tag_key(&mut self, mut tag: TagPrompt, key: KeyEvent) {
        let text = tag.message.as_mut().unwrap_or(&mut tag.name);
        match key.code {
            KeyCode::Char(_) => text.extend(typed(key)),
            KeyCode::Backspace => {
                text.pop();
            }
            KeyCode::Esc => return,
            KeyCode::Enter if tag.message.is_none() => {
                if !tag.name.trim().is_empty() {
                    tag.message = Some(String::new());
                }
            }
            KeyCode::Enter => {
                let name = tag.name.trim().to_string();
                self.status = format!("Tagging {name}…");
                // An empty message makes a lightweight tag.
                let write = Write::CreateTag {
                    name,
                    commit: tag.sha,
                    message: tag.message.filter(|m| !m.trim().is_empty()),
                };
                self.queue(write, Confirmation::Absent);
                return;
            }
            _ => {}
        }
        self.modal = Some(Modal::Tag(tag));
    }

    /// `c` on the working-directory node: asks for the message of a commit on
    /// the checked-out commit, its parent.
    fn start_commit(&mut self) {
        let Some(node) = self.selected_node() else {
            return;
        };
        if node.kind != CommitKind::Working {
            return;
        }
        let Some(head) = node.parents.first() else {
            return;
        };
        self.modal = Some(Modal::Commit {
            head: head.clone(),
            message: String::new(),
        });
    }

    fn commit_key(&mut self, head: String, mut message: String, key: KeyEvent) {
        match key.code {
            KeyCode::Char(_) => message.extend(typed(key)),
            KeyCode::Backspace => {
                message.pop();
            }
            KeyCode::Esc => return,
            KeyCode::Enter if !message.trim().is_empty() => {
                self.status = "Committing…".to_string();
                let write = Write::Commit {
                    message,
                    head: Some(head),
                };
                self.queue(write, Confirmation::Absent);
                return;
            }
            _ => {}
        }
        self.modal = Some(Modal::Commit { head, message });
    }

    /// `a` and `u` on a file of the working-directory node: stages or unstages
    /// it whole.
    fn stage(&mut self, stage: bool) {
        let rows = self.file_rows();
        let Some(files) = &self.files else { return };
        if files.sha != ocelli_core::model::WORKDIR_SHA {
            return;
        }
        let (Ok(list), Some(FileRow::File { index, .. })) =
            (&files.files, rows.get(files.selected))
        else {
            return;
        };
        let file = &list[*index];
        let (path, shown) = (
            file.path.clone(),
            file.shown.clone().unwrap_or(Shown {
                workdir: None,
                index: None,
            }),
        );
        self.status = format!("{} {path}…", if stage { "Staging" } else { "Unstaging" });
        let write = if stage {
            Write::StageFile {
                path,
                shown: shown.workdir,
            }
        } else {
            Write::UnstageFile {
                path,
                shown: shown.index,
            }
        };
        self.queue(write, Confirmation::Absent);
    }

    /// Hands `write` to the event loop, unless one runs still: a second press
    /// writes nothing.
    fn queue(&mut self, write: Write, confirmation: Confirmation) {
        if self.writing {
            self.status = "A write is still running".to_string();
            return;
        }
        self.writing = true;
        self.write = Some((write, confirmation));
    }

    /// The answer to a write that discards work: only `y` writes, every other
    /// key keeps the work (ADR-0034).
    fn answer(&mut self, write: Write, key: KeyEvent) {
        if key.code == KeyCode::Char('y') {
            self.status = "Writing…".to_string();
            self.queue(write, Confirmation::Given);
        } else {
            self.status = "Nothing discarded".to_string();
        }
    }

    fn stash_key(&mut self, mut stash: StashPrompt, key: KeyEvent) {
        match key.code {
            KeyCode::Char(_) => stash.message.extend(typed(key)),
            KeyCode::Backspace => {
                stash.message.pop();
            }
            KeyCode::Tab => stash.untracked = !stash.untracked,
            KeyCode::Esc => return,
            KeyCode::Enter => {
                self.status = "Stashing…".to_string();
                let message = Some(stash.message).filter(|m| !m.trim().is_empty());
                let write = Write::CreateStash {
                    message,
                    untracked: stash.untracked,
                };
                self.queue(write, Confirmation::Absent);
                return;
            }
            _ => {}
        }
        self.modal = Some(Modal::Stash(stash));
    }

    /// `a`, `p`, `d` on a selected stash node: applies it, and pops or drops it
    /// once the developer has confirmed it.
    fn on_stash(&mut self, key: KeyCode) {
        let Some(node) = self.selected_node() else {
            return;
        };
        let Some(index) = node.stash else {
            return;
        };
        let commit = node.sha.clone();
        let write = match key {
            KeyCode::Char('d') => Write::DropStash { index, commit },
            KeyCode::Char(c) => Write::ApplyStash {
                index,
                commit,
                drop: c == 'p',
            },
            _ => return,
        };
        self.ask_or_queue(write);
    }

    /// The entry selected in the expanded sidebar, where it is a ref.
    fn sidebar_item(&self) -> Option<Item> {
        let data = self.data.as_ref()?;
        let rows = sidebar::rows(&data.sidebar, &self.side);
        match rows.get(self.side.selected).map(|r| &r.kind) {
            Some(sidebar::RowKind::Leaf(item)) => Some((*item).clone()),
            _ => None,
        }
    }

    /// `d` on a tag or a local branch in the sidebar: deletes it, asking first
    /// where that discards work (ADR-0034).
    fn start_delete(&mut self) {
        let Some(data) = &self.data else { return };
        let Some(item) = self.sidebar_item() else {
            return;
        };
        let write = match item.look {
            sidebar::Look::Tag => {
                let Some(tag) = data.tags.iter().find(|t| t.name == item.name) else {
                    return;
                };
                Write::DeleteTag {
                    name: tag.name.clone(),
                    commit: tag.sha.clone(),
                    annotated: tag.annotated,
                }
            }
            sidebar::Look::Local => {
                let Some(branch) = data.locals.iter().find(|b| b.name == item.name) else {
                    return;
                };
                Write::DeleteBranch {
                    name: branch.name.clone(),
                    tip: branch.sha.clone(),
                    merged: branch.merged,
                }
            }
            sidebar::Look::Head => {
                self.status = format!("{} is checked out", item.name);
                return;
            }
            _ => return,
        };
        self.ask_or_queue(write);
    }

    /// `w` in the sidebar: switches to a local branch, or to a remote one
    /// through a local branch tracking it (G-4).
    fn start_switch(&mut self) {
        let Some(item) = self.sidebar_item() else {
            return;
        };
        let Action::Reveal(tip) = item.action.clone() else {
            return;
        };
        let write = match (item.look, item.remote) {
            (sidebar::Look::Local, _) => Write::SwitchBranch {
                name: item.name.clone(),
                tip,
            },
            (sidebar::Look::Remote, Some(remote)) => Write::TrackRemote {
                remote,
                branch: item.name.clone(),
                tip,
            },
            _ => return,
        };
        self.status = format!("Switching to {}…", item.name);
        self.queue(write, Confirmation::Absent);
    }

    /// A write that discards work goes to the question; any other is queued.
    fn ask_or_queue(&mut self, write: Write) {
        if write.discards().is_some() {
            self.modal = Some(Modal::Confirm(write));
        } else {
            self.status = "Writing…".to_string();
            self.queue(write, Confirmation::Absent);
        }
    }

    /// `f`: fetches `remote`, or the one a fetch reaches without one named
    /// (ADR-0035); `Esc` cancels it while it runs.
    fn start_fetch(&mut self, remote: Option<String>) {
        let Some(remote) = remote.or_else(|| self.data.as_ref()?.default_remote.clone()) else {
            self.status = "No remote to fetch".to_string();
            return;
        };
        if self.writing {
            self.status = "A write is still running".to_string();
            return;
        }
        self.status = format!("Fetching {remote}… Esc cancels");
        self.queue(Write::Fetch { remote }, Confirmation::Absent);
        self.transferring = true;
        self.cancellable = true;
    }

    /// `U`: pulls the checked-out branch from its upstream, fast-forward only
    /// (ADR-0035); `Esc` cancels it while it fetches.
    fn start_pull(&mut self) {
        let Some(head) = self
            .data
            .as_ref()
            .and_then(|d| d.locals.iter().find(|b| b.is_head))
        else {
            self.status = "No branch is checked out to pull".to_string();
            return;
        };
        if head.upstream.is_none() {
            self.status = format!("{} has no upstream to pull from", head.name);
            return;
        }
        if self.writing {
            self.status = "A write is still running".to_string();
            return;
        }
        let write = Write::Pull {
            branch: head.name.clone(),
            tip: head.sha.clone(),
        };
        self.status = format!("Pulling {}… Esc cancels", head.name);
        self.queue(write, Confirmation::Absent);
        self.transferring = true;
        self.cancellable = true;
    }

    /// `P`: pushes the checked-out branch to its upstream, or to the remote a
    /// fetch reaches, under its own name; never by force (ADR-0035).
    fn start_push(&mut self) {
        let Some(data) = self.data.as_ref() else {
            return;
        };
        let Some(head) = data.locals.iter().find(|b| b.is_head) else {
            self.status = "No branch is checked out to push".to_string();
            return;
        };
        if head.upstream.is_none() && data.default_remote.is_none() {
            self.status = format!("There is no remote to push {} to", head.name);
            return;
        }
        if self.writing {
            self.status = "A write is still running".to_string();
            return;
        }
        let write = Write::Push {
            branch: head.name.clone(),
            tip: head.sha.clone(),
        };
        // libgit2 cannot stop a push once its objects are sent, so none is offered.
        self.status = format!("Pushing {}…", head.name);
        self.queue(write, Confirmation::Absent);
        self.transferring = true;
    }

    /// `b` on a commit: asks for the name of a branch there.
    fn start_branch(&mut self) {
        let Some(node) = self.selected_node() else {
            return;
        };
        if node.kind != CommitKind::Commit {
            self.status = "Only a commit can carry a branch".to_string();
            return;
        }
        self.modal = Some(Modal::Branch {
            commit: node.sha.clone(),
            name: String::new(),
            switch: true,
        });
    }

    fn branch_key(&mut self, commit: String, mut name: String, mut switch: bool, key: KeyEvent) {
        match key.code {
            KeyCode::Char(_) => name.extend(typed(key)),
            KeyCode::Backspace => {
                name.pop();
            }
            KeyCode::Tab => switch = !switch,
            KeyCode::Esc => return,
            KeyCode::Enter if !name.trim().is_empty() => {
                let name = name.trim().to_string();
                self.status = format!("Creating {name}…");
                let write = Write::CreateBranch {
                    name,
                    commit,
                    switch,
                };
                self.queue(write, Confirmation::Absent);
                return;
            }
            _ => {}
        }
        self.modal = Some(Modal::Branch {
            commit,
            name,
            switch,
        });
    }

    /// `w` on a commit in the graph or the detail: switches to it with `HEAD`
    /// detached.
    fn start_detach(&mut self) {
        let Some(node) = self.selected_node() else {
            return;
        };
        if node.kind != CommitKind::Commit {
            return;
        }
        self.status = format!("Switching to {}…", node.short_sha);
        self.queue(
            Write::Detach {
                commit: node.sha.clone(),
            },
            Confirmation::Absent,
        );
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
            Some(data) => data.history().search(&self.search.query),
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
            KeyCode::Char('d') if !minimised => self.start_delete(),
            KeyCode::Char('w') if !minimised => self.start_switch(),
            KeyCode::Char('f') => {
                let remote = self.sidebar_item().and_then(|item| item.remote);
                self.start_fetch(remote);
            }
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
                // A ref can point at history not loaded (ADR-0016).
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
            KeyCode::Char('t') => {
                self.start_tag();
                None
            }
            KeyCode::Char('c') => {
                self.start_commit();
                None
            }
            KeyCode::Char('b') => {
                self.start_branch();
                None
            }
            KeyCode::Char('w') => {
                self.start_detach();
                None
            }
            KeyCode::Char('s') => {
                self.modal = Some(Modal::Stash(StashPrompt::default()));
                None
            }
            KeyCode::Char(c @ ('a' | 'p' | 'd')) => {
                self.on_stash(KeyCode::Char(c));
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
            KeyCode::Char('t') => self.start_tag(),
            KeyCode::Char('c') => self.start_commit(),
            KeyCode::Char('b') => self.start_branch(),
            KeyCode::Char('w') => self.start_detach(),
            KeyCode::Char(c @ ('a' | 'p' | 'd')) => self.on_stash(KeyCode::Char(c)),
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
            KeyCode::Char('a') => return self.stage(true),
            KeyCode::Char('u') => return self.stage(false),
            KeyCode::Char('c') => return self.start_commit(),
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
    use ocelli_core::graph::Layout;
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
            stash: None,
        }
    }

    /// Rows: d (merge, `main`), c (side, `🏷 v1`), b, a — with the sidebar
    /// refs of `sidebar::tests::repo_refs`.
    pub(crate) fn loaded_app() -> App {
        let commits = vec![
            commit("d", 4, &["b", "c"], &["main"]),
            commit("c", 3, &["a"], &["🏷 v1"]),
            commit("b", 2, &["a"], &[]),
            commit("a", 1, &[], &[]),
        ];
        let refs = crate::sidebar::tests::repo_refs();
        let mut app = App::new("/repo".into(), Glyphs::Unicode);
        let chunk = Layout::new().lay_out(commits);
        first_chunk(&mut app, chunk, refs);
        app.on_msg(load_ended(Ok(())));
        app
    }

    /// The first chunk of a load, then its refs.
    fn first_chunk(app: &mut App, chunk: LaidChunk, refs: RepoRefs) {
        app.on_msg(Msg::Load(Event::Chunk(chunk)));
        app.on_msg(Msg::Load(Event::Refs(Box::new(refs))));
    }

    /// A further chunk of a load.
    fn next_chunk(chunk: LaidChunk) -> Msg {
        Msg::Load(Event::Chunk(chunk))
    }

    fn load_ended(result: Result<(), String>) -> Msg {
        Msg::Load(Event::Ended(result))
    }

    /// The write handed to the event loop, which then runs it and reports it done.
    fn taken(app: &mut App) -> Option<(Write, Confirmation)> {
        app.writing = false;
        app.write.take()
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
        assert_eq!(
            app.lineage.as_ref().map(Lineage::rows),
            Some(rows_of(&app, &["a", "b", "d"]))
        );
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
                staged: None,
                shown: None,
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
        assert_eq!(app.prompt(), Some("/repo"));
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
    // Verifies: ADR-0033, G-7
    fn t_asks_for_a_name_then_a_message_and_hands_the_tag_to_the_event_loop() {
        let mut app = loaded_app();
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Char('t'));
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.tag_prompt().unwrap().message, None, "a name first");
        type_text(&mut app, "v2");
        press(&mut app, KeyCode::Enter);
        type_text(&mut app, "Second");
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.tag_prompt(), None);
        let tag = Write::CreateTag {
            name: "v2".into(),
            commit: "c".into(),
            message: Some("Second".into()),
        };
        assert_eq!(app.write, Some((tag, Confirmation::Absent)));
        assert_eq!(app.status, "Tagging v2…");
    }

    #[test]
    fn a_tag_without_a_message_is_lightweight_and_esc_writes_nothing() {
        let mut app = loaded_app();
        press(&mut app, KeyCode::Char('t'));
        type_text(&mut app, "mark");
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Enter);
        let Some((Write::CreateTag { message, .. }, _)) = taken(&mut app) else {
            panic!("no tag written");
        };
        assert_eq!(message, None);

        press(&mut app, KeyCode::Char('t'));
        type_text(&mut app, "x");
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.tag_prompt(), None);
        assert_eq!(app.write, None);
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(sha(&app), "c", "keys move again once the prompt is closed");
    }

    #[test]
    fn only_a_selected_commit_can_be_tagged() {
        let mut working = commit(ocelli_core::model::WORKDIR_SHA, 9, &["d"], &[]);
        working.kind = CommitKind::Working;
        let mut app = App::new("/repo".into(), Glyphs::Unicode);
        first_chunk(
            &mut app,
            Layout::new().lay_out(vec![working]),
            RepoRefs::default(),
        );
        press(&mut app, KeyCode::Char('t'));
        assert_eq!(app.tag_prompt(), None);
        assert_eq!(app.status, "Only a commit can be tagged");

        let mut app = loaded_app();
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Char('t'));
        assert_eq!(app.tag_prompt(), None, "nothing selected, nothing to tag");
    }

    #[test]
    // Verifies: ADR-0033
    fn a_write_done_reloads_and_selects_the_commit_again_once_its_row_arrives() {
        let mut app = loaded_app();
        press(&mut app, KeyCode::Char('j'));
        app.on_msg(Msg::Written(Ok(None)));
        assert_eq!(app.reload, Some(PathBuf::from("/repo")));
        assert_eq!(app.select_again.as_deref(), Some("c"));
        let commits = vec![
            commit("d", 4, &["b", "c"], &["main"]),
            commit("c", 3, &["a"], &["🏷 v1", "🏷 v2"]),
            commit("b", 2, &["a"], &[]),
            commit("a", 1, &[], &[]),
        ];
        app.on_msg(next_chunk(Layout::new().lay_out(commits)));
        assert_eq!(sha(&app), "c");
        assert!(
            app.selected_node()
                .unwrap()
                .refs
                .contains(&"🏷 v2".to_string())
        );
        assert_eq!(app.select_again, None);
    }

    #[test]
    // Verifies: ADR-0033
    fn the_commit_a_write_left_selected_arrives_late_and_yields_to_the_developer() {
        let reload = |pick: bool| {
            let mut app = loaded_app();
            app.select_again = Some("c4".to_string());
            app.data = None;
            let mut layout = Layout::new();
            app.on_msg(next_chunk(layout.lay_out(line(8, 3))));
            assert_eq!(sha(&app), "c8", "the newest until the commit arrives");
            if pick {
                press(&mut app, KeyCode::Char('j'));
            }
            app.on_msg(next_chunk(layout.lay_out(line(5, 3))));
            sha(&app)
        };
        assert_eq!(reload(false), "c4");
        assert_eq!(reload(true), "c7", "the developer's own choice stays");
    }

    #[test]
    fn a_write_refused_says_why_and_reloads_nothing() {
        let mut app = loaded_app();
        app.on_msg(Msg::Written(Err("a tag named 'v1' exists already".into())));
        assert_eq!(app.status, "Not written: a tag named 'v1' exists already");
        assert_eq!(app.reload, None);
        assert!(app.data.is_some());
    }

    /// The sidebar's selection on the tag `v1` of `sidebar::tests::repo_refs`.
    fn on_the_tag(app: &mut App) {
        on_the_entry(app, 4, "v1");
    }

    /// The sidebar's selection on the entry `label` of the section the rail
    /// lists at `rail`, which its glyph opens as it expands the sidebar.
    fn on_the_entry(app: &mut App, rail: usize, label: &str) {
        press(app, KeyCode::Char('1'));
        app.focus = Focus::Sidebar;
        app.side.rail_selected = rail;
        press(app, KeyCode::Enter);
        let data = app.data.as_ref().unwrap();
        let rows = sidebar::rows(&data.sidebar, &app.side);
        let labels: Vec<_> = rows.iter().map(|r| r.label.clone()).collect();
        app.side.selected = labels
            .iter()
            .position(|l| l == label)
            .unwrap_or_else(|| panic!("{label} not among {labels:?}"));
    }

    #[test]
    // Verifies: ADR-0034, Q-6
    fn d_on_a_tag_asks_and_deletes_it_only_on_an_explicit_yes() {
        let mut app = loaded_app();
        on_the_tag(&mut app);
        press(&mut app, KeyCode::Char('d'));
        let delete = Write::DeleteTag {
            name: "v1".into(),
            commit: "c".into(),
            annotated: true,
        };
        assert_eq!(app.confirm(), Some(&delete));
        press(&mut app, KeyCode::Char('y'));
        assert_eq!(app.confirm(), None);
        assert_eq!(app.write, Some((delete, Confirmation::Given)));
    }

    #[test]
    // Verifies: ADR-0034, Q-6
    fn any_other_answer_keeps_the_tag_and_writes_nothing() {
        for answer in [
            KeyCode::Char('n'),
            KeyCode::Enter,
            KeyCode::Esc,
            KeyCode::Char('Y'),
        ] {
            let mut app = loaded_app();
            on_the_tag(&mut app);
            press(&mut app, KeyCode::Char('d'));
            press(&mut app, answer);
            assert_eq!(app.confirm(), None);
            assert_eq!(app.write, None, "{answer:?}");
            assert_eq!(app.status, "Nothing discarded");
        }
    }

    #[test]
    fn d_on_anything_but_a_tag_asks_nothing() {
        let mut app = loaded_app();
        app.focus = Focus::Sidebar;
        app.side.selected = 0;
        press(&mut app, KeyCode::Char('d'));
        assert_eq!(app.confirm(), None);
    }

    #[test]
    // Verifies: ADR-0033, G-7
    fn s_asks_for_a_message_and_the_untracked_switch_and_stashes() {
        let mut app = loaded_app();
        press(&mut app, KeyCode::Char('s'));
        type_text(&mut app, "wip");
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.stash_prompt(), None);
        let stash = Write::CreateStash {
            message: Some("wip".into()),
            untracked: true,
        };
        assert_eq!(app.write, Some((stash, Confirmation::Absent)));

        let mut app = loaded_app();
        press(&mut app, KeyCode::Char('s'));
        press(&mut app, KeyCode::Enter);
        let stash = Write::CreateStash {
            message: None,
            untracked: false,
        };
        assert_eq!(
            app.write,
            Some((stash, Confirmation::Absent)),
            "untracked off by default"
        );
    }

    /// A history whose newest node is the stash entry `stash@{1}` on `a`, selected.
    fn on_a_stash() -> App {
        let mut stash = commit("s", 9, &["a"], &["📦 stash@{1}"]);
        stash.kind = CommitKind::Stash;
        stash.stash = Some(1);
        let mut app = App::new("/repo".into(), Glyphs::Unicode);
        let chunk = Layout::new().lay_out(vec![stash, commit("a", 1, &[], &[])]);
        first_chunk(&mut app, chunk, RepoRefs::default());
        assert_eq!(sha(&app), "s");
        app
    }

    #[test]
    // Verifies: ADR-0034, G-7
    fn a_applies_the_selected_stash_and_p_and_d_ask_first() {
        let mut app = on_a_stash();
        press(&mut app, KeyCode::Char('a'));
        let apply = |drop| Write::ApplyStash {
            index: 1,
            commit: "s".into(),
            drop,
        };
        assert_eq!(app.confirm(), None, "applying keeps the stash");
        assert_eq!(taken(&mut app), Some((apply(false), Confirmation::Absent)));

        press(&mut app, KeyCode::Char('p'));
        assert_eq!(app.confirm(), Some(&apply(true)));
        press(&mut app, KeyCode::Char('y'));
        assert_eq!(taken(&mut app), Some((apply(true), Confirmation::Given)));

        let drop = Write::DropStash {
            index: 1,
            commit: "s".into(),
        };
        press(&mut app, KeyCode::Char('d'));
        assert_eq!(app.confirm(), Some(&drop));
        press(&mut app, KeyCode::Char('n'));
        assert_eq!(app.write, None, "declined, nothing written");
        assert_eq!(app.status, "Nothing discarded");
    }

    #[test]
    fn a_stash_key_held_with_ctrl_writes_nothing() {
        let mut app = on_a_stash();
        let ctrl = |code| KeyEvent::new(code, KeyModifiers::CONTROL);
        app.on_key(ctrl(KeyCode::Char('s')));
        assert_eq!(app.stash_prompt(), None);
        app.focus = Focus::Detail;
        app.on_key(ctrl(KeyCode::Char('d')));
        assert_eq!(app.confirm(), None);
    }

    #[test]
    // Verifies: ADR-0034, G-7
    fn no_key_held_with_ctrl_or_alt_writes_or_asks() {
        let on_the_commit = || {
            let mut app = loaded_app();
            press(&mut app, KeyCode::Char('j'));
            app
        };
        let on_a_tag = || {
            let mut app = loaded_app();
            on_the_tag(&mut app);
            app
        };
        let held = [
            KeyModifiers::CONTROL,
            KeyModifiers::ALT,
            KeyModifiers::CONTROL | KeyModifiers::ALT,
        ];
        for (selected, fresh) in [
            ("a commit", &on_the_commit as &dyn Fn() -> App),
            ("a stash", &on_a_stash),
            ("a tag in the sidebar", &on_a_tag),
        ] {
            for focus in PANES {
                for c in ' '..='~' {
                    for modifiers in held {
                        let mut app = fresh();
                        app.focus = focus;
                        app.on_key(KeyEvent::new(KeyCode::Char(c), modifiers));
                        let asked = matches!(
                            app.modal,
                            Some(Modal::Confirm(_) | Modal::Tag(_) | Modal::Stash(_))
                        );
                        assert!(
                            app.write.is_none() && !asked,
                            "{modifiers:?}+{c} on {selected} in {focus:?} writes or asks"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_character_altgr_types_works_as_typed() {
        let altgr = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL | KeyModifiers::ALT);
        let mut app = loaded_app();
        app.on_key(altgr(']'));
        assert_eq!(sha(&app), "b", "first parent, as `]` without AltGr");
        press(&mut app, KeyCode::Char('t'));
        for c in ['v', '@', '{'] {
            app.on_key(if c == 'v' {
                KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
            } else {
                altgr(c)
            });
        }
        assert_eq!(app.tag_prompt().unwrap().name, "v@{");
        app.on_key(altgr('q'));
        assert_eq!(
            app.tag_prompt().unwrap().name,
            "v@{",
            "a letter with Ctrl+Alt types nothing"
        );
    }

    /// The working-directory node on `d`, selected, its files `f` (staged) and
    /// `g` (not) delivered.
    fn on_the_working_node() -> App {
        let mut working = commit(ocelli_core::model::WORKDIR_SHA, 9, &["d"], &[]);
        working.kind = CommitKind::Working;
        let mut app = App::new("/repo".into(), Glyphs::Unicode);
        let chunk = Layout::new().lay_out(vec![working, commit("d", 4, &[], &["main"])]);
        first_chunk(&mut app, chunk, RepoRefs::default());
        let file = |path: &str, staged| FileChange {
            path: path.into(),
            old_path: None,
            status: FileStatus::Modified,
            additions: 1,
            deletions: 0,
            staged: Some(staged),
            shown: Some(Shown {
                workdir: Some(format!("w{path}")),
                index: Some(format!("i{path}")),
            }),
        };
        app.on_msg(Msg::Files {
            sha: ocelli_core::model::WORKDIR_SHA.into(),
            files: Ok(vec![
                file("f", ocelli_core::model::Staged::Yes),
                file("g", ocelli_core::model::Staged::No),
            ]),
        });
        app.file_view = FileView::Flat;
        app
    }

    #[test]
    // Verifies: ADR-0033, G-4
    fn a_and_u_stage_and_unstage_the_file_under_the_cursor() {
        let mut app = on_the_working_node();
        app.focus = Focus::Files;
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Char('a'));
        let stage = Write::StageFile {
            path: "g".into(),
            shown: Some("wg".into()),
        };
        assert_eq!(taken(&mut app), Some((stage, Confirmation::Absent)));
        press(&mut app, KeyCode::Char('k'));
        press(&mut app, KeyCode::Char('u'));
        let unstage = Write::UnstageFile {
            path: "f".into(),
            shown: Some("if".into()),
        };
        assert_eq!(taken(&mut app), Some((unstage, Confirmation::Absent)));

        let mut app = loaded_app();
        app.files = Some(Files {
            sha: "d".into(),
            files: Ok(vec![FileChange {
                path: "f".into(),
                old_path: None,
                status: FileStatus::Modified,
                additions: 1,
                deletions: 0,
                staged: None,
                shown: None,
            }]),
            selected: 0,
        });
        app.focus = Focus::Files;
        press(&mut app, KeyCode::Char('a'));
        assert_eq!(app.write, None, "a commit's files do not stage");
    }

    #[test]
    // Verifies: ADR-0033, G-4
    fn c_on_the_working_node_commits_the_index_on_its_parent() {
        let mut app = on_the_working_node();
        press(&mut app, KeyCode::Char('c'));
        press(&mut app, KeyCode::Enter);
        assert!(app.write.is_none(), "no commit without a message");
        type_text(&mut app, "Change f");
        press(&mut app, KeyCode::Enter);
        let commit = Write::Commit {
            message: "Change f".into(),
            head: Some("d".into()),
        };
        assert_eq!(taken(&mut app), Some((commit, Confirmation::Absent)));

        let mut app = loaded_app();
        press(&mut app, KeyCode::Char('c'));
        assert_eq!(app.modal, None, "a commit is no working directory");
    }

    #[test]
    // Verifies: ADR-0033
    fn the_commit_a_write_made_is_selected_after_the_reload() {
        let mut app = on_the_working_node();
        app.on_msg(Msg::Written(Ok(Some("new".into()))));
        assert_eq!(app.select_again.as_deref(), Some("new"));
    }

    #[test]
    fn no_second_write_starts_while_one_runs() {
        let mut app = on_the_working_node();
        app.focus = Focus::Files;
        press(&mut app, KeyCode::Char('u'));
        assert!(app.write.take().is_some());
        press(&mut app, KeyCode::Char('u'));
        assert_eq!(app.write, None);
        assert_eq!(app.status, "A write is still running");
        app.on_msg(Msg::Written(Err("refused".into())));
        press(&mut app, KeyCode::Char('u'));
        assert!(
            app.write.is_some(),
            "the result is in, the next write starts"
        );
    }

    #[test]
    // Verifies: ADR-0033, G-4
    fn b_asks_for_a_branch_at_the_commit_switched_to_unless_turned_off() {
        let mut app = loaded_app();
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Char('b'));
        type_text(&mut app, "topic");
        press(&mut app, KeyCode::Enter);
        let branch = |switch| Write::CreateBranch {
            name: "topic".into(),
            commit: "c".into(),
            switch,
        };
        assert_eq!(taken(&mut app), Some((branch(true), Confirmation::Absent)));
        press(&mut app, KeyCode::Char('b'));
        type_text(&mut app, "topic");
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Enter);
        assert_eq!(taken(&mut app), Some((branch(false), Confirmation::Absent)));
    }

    #[test]
    // Verifies: ADR-0033, G-4
    fn w_switches_to_a_branch_a_remote_branch_or_a_commit() {
        let mut app = loaded_app();
        press(&mut app, KeyCode::Char('w'));
        let detach = Write::Detach { commit: "d".into() };
        assert_eq!(taken(&mut app), Some((detach, Confirmation::Absent)));

        let mut app = loaded_app();
        on_the_entry(&mut app, 1, "x");
        press(&mut app, KeyCode::Char('w'));
        let switch = Write::SwitchBranch {
            name: "feat/x".into(),
            tip: "b".into(),
        };
        assert_eq!(taken(&mut app), Some((switch, Confirmation::Absent)));

        let mut app = loaded_app();
        on_the_entry(&mut app, 2, "main");
        // The remote's `main`, below the local one.
        let data = app.data.as_ref().unwrap();
        let rows = sidebar::rows(&data.sidebar, &app.side);
        app.side.selected = rows.iter().rposition(|r| r.label == "main").unwrap();
        press(&mut app, KeyCode::Char('w'));
        let track = Write::TrackRemote {
            remote: "origin".into(),
            branch: "main".into(),
            tip: "b".into(),
        };
        assert_eq!(taken(&mut app), Some((track, Confirmation::Absent)));
    }

    #[test]
    // Verifies: ADR-0034, Q-6
    fn d_deletes_a_merged_branch_and_asks_first_for_one_that_is_not() {
        let delete = |merged| Write::DeleteBranch {
            name: "feat/x".into(),
            tip: "b".into(),
            merged,
        };
        for merged in [true, false] {
            let mut app = loaded_app();
            app.data.as_mut().unwrap().locals[1].merged = merged;
            on_the_entry(&mut app, 1, "x");
            press(&mut app, KeyCode::Char('d'));
            if merged {
                assert_eq!(taken(&mut app), Some((delete(true), Confirmation::Absent)));
            } else {
                assert_eq!(app.confirm(), Some(&delete(false)));
                press(&mut app, KeyCode::Char('n'));
                assert_eq!(app.write, None);
            }
        }
        let mut app = loaded_app();
        on_the_entry(&mut app, 1, "main");
        press(&mut app, KeyCode::Char('d'));
        assert_eq!((app.modal.clone(), app.write.clone()), (None, None));
        assert_eq!(app.status, "main is checked out");
    }

    #[test]
    // Verifies: ADR-0035, G-4
    fn f_fetches_the_default_remote_shows_its_progress_and_esc_cancels_it() {
        let mut app = loaded_app();
        press(&mut app, KeyCode::Char('f'));
        let fetch = Write::Fetch {
            remote: "origin".into(),
        };
        assert_eq!(app.write.take(), Some((fetch, Confirmation::Absent)));
        app.on_msg(Msg::WriteProgress("Fetching origin: 3 of 9 objects".into()));
        assert_eq!(app.status, "Fetching origin: 3 of 9 objects");
        press(&mut app, KeyCode::Esc);
        assert!(app.cancel);
        assert!(app.active, "Esc cancels the fetch rather than deselecting");
        app.on_msg(Msg::Written(
            Err("the fetch of origin was cancelled".into()),
        ));
        assert!(!app.writing);

        let mut app = loaded_app();
        app.data.as_mut().unwrap().default_remote = None;
        press(&mut app, KeyCode::Char('f'));
        assert_eq!(
            (app.write.clone(), app.status.as_str()),
            (None, "No remote to fetch")
        );
    }

    #[test]
    // Verifies: ADR-0035, G-4
    fn capital_u_pulls_the_checked_out_branch_where_it_has_an_upstream() {
        let mut app = loaded_app();
        app.data.as_mut().unwrap().locals[0].upstream = Some("origin/main".into());
        press(&mut app, KeyCode::Char('U'));
        let pull = Write::Pull {
            branch: "main".into(),
            tip: "d".into(),
        };
        assert_eq!(app.write.take(), Some((pull, Confirmation::Absent)));
        assert!(app.cancellable);

        let mut app = loaded_app();
        app.data.as_mut().unwrap().locals[0].upstream = None;
        press(&mut app, KeyCode::Char('U'));
        assert_eq!(app.write, None);
        assert_eq!(app.status, "main has no upstream to pull from");
    }

    #[test]
    // Verifies: ADR-0035, G-4
    fn capital_p_pushes_the_checked_out_branch_where_a_remote_takes_it() {
        let mut app = loaded_app();
        press(&mut app, KeyCode::Char('P'));
        let push = Write::Push {
            branch: "main".into(),
            tip: "d".into(),
        };
        assert_eq!(app.write.take(), Some((push, Confirmation::Absent)));

        let mut app = loaded_app();
        app.data.as_mut().unwrap().default_remote = None;
        app.data.as_mut().unwrap().locals[0].upstream = None;
        press(&mut app, KeyCode::Char('P'));
        assert_eq!(app.write, None);
        assert_eq!(app.status, "There is no remote to push main to");
    }

    #[test]
    // Verifies: ADR-0035
    fn a_transfer_refused_reloads_and_keeps_its_reason_in_view_until_the_next_key() {
        let mut app = loaded_app();
        app.data.as_mut().unwrap().locals[0].upstream = Some("origin/main".into());
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Char('U'));
        assert!(app.write.take().is_some());
        app.on_msg(Msg::Written(Err(
            "'main' and its upstream have diverged".into()
        )));
        assert_eq!(
            app.reload,
            Some(PathBuf::from("/repo")),
            "the fetch may have moved refs"
        );
        assert_eq!(app.select_again.as_deref(), Some("c"));
        let text = "Not written: 'main' and its upstream have diverged";
        assert_eq!(app.notice.as_deref(), Some(text));
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.notice, None);

        // A write that reaches no remote is refused without a reload.
        let mut app = loaded_app();
        app.writing = true;
        app.on_msg(Msg::Written(Err("a tag named 'v1' exists already".into())));
        assert_eq!(app.reload, None);
    }

    #[test]
    fn stash_keys_on_a_commit_do_nothing() {
        let mut app = loaded_app();
        for key in ['a', 'p', 'd'] {
            press(&mut app, KeyCode::Char(key));
        }
        assert_eq!(app.confirm(), None);
        assert_eq!(app.write, None);
    }

    #[test]
    fn capital_t_switches_the_theme() {
        let mut app = loaded_app();
        press(&mut app, KeyCode::Char('T'));
        assert_eq!(app.theme, Theme::Light);
    }

    /// A line of `n` commits from `c{top}` down, each on the next, as the walk
    /// yields them.
    fn line(top: usize, n: usize) -> Vec<CommitData> {
        (top + 1 - n..=top)
            .rev()
            .map(|i| {
                let parent = format!("c{}", i.saturating_sub(1));
                let parents: &[&str] = if i > 0 { &[parent.as_str()] } else { &[] };
                commit(&format!("c{i}"), i as i64, parents, &[])
            })
            .collect()
    }

    /// The first chunk of a line of 9 commits, `c8` to `c6`, delivered; the
    /// layout lays out the chunks after it as the loader does.
    fn chunked_app() -> (App, Layout) {
        let mut layout = Layout::new();
        let mut app = App::new("/repo".into(), Glyphs::Unicode);
        let first = layout.lay_out(line(8, 3));
        first_chunk(&mut app, first, RepoRefs::default());
        (app, layout)
    }

    #[test]
    // Verifies: ADR-0016, ADR-0031, G-13
    fn a_history_arriving_in_chunks_grows_below_what_is_shown() {
        let (mut app, mut layout) = chunked_app();
        assert_eq!(app.status, "3 commits in 1 chunk — loading older history…");
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(sha(&app), "c7");
        app.on_msg(Msg::Load(Event::Progress("Loaded 500 commits...".into())));
        assert!(
            app.status.starts_with("3 commits"),
            "the progress line counts chunks"
        );

        app.on_msg(next_chunk(layout.lay_out(line(5, 3))));
        assert_eq!(app.status, "6 commits in 2 chunks — loading older history…");
        assert_eq!(
            (app.selected, sha(&app)),
            (1, "c7".into()),
            "the selection stays"
        );
        press(&mut app, KeyCode::Char('G'));
        assert_eq!(sha(&app), "c3", "the rows added can be scrolled to");

        app.on_msg(next_chunk(layout.lay_out(line(2, 3))));
        app.on_msg(load_ended(Ok(())));
        assert_eq!(app.status, "9 commits");
    }

    #[test]
    // Verifies: ADR-0016, ADR-0026
    fn the_first_chunk_shows_before_its_refs_and_the_refs_fill_the_sidebar_after() {
        let mut layout = Layout::new();
        let mut app = App::new("/repo".into(), Glyphs::Unicode);
        app.on_msg(next_chunk(layout.lay_out(line(8, 3))));
        let data = app.data.as_ref().unwrap();
        assert_eq!((data.rows(), data.refs.head.as_deref()), (3, None));
        app.on_msg(Msg::Load(Event::Refs(Box::new(
            crate::sidebar::tests::repo_refs(),
        ))));
        assert!(
            app.data.as_ref().unwrap().refs.head.is_some(),
            "the head's branch arrived"
        );
    }

    #[test]
    // Verifies: ADR-0016, G-13
    fn a_load_failing_before_its_first_chunk_says_so() {
        let mut app = App::new("/repo".into(), Glyphs::Unicode);
        app.on_msg(load_ended(Err("not a repository".into())));
        assert_eq!(app.status, "Error: not a repository");
        assert!(app.data.is_none());
    }

    #[test]
    // Verifies: ADR-0016, G-13
    fn a_load_stopped_by_an_error_keeps_what_has_arrived_and_says_so() {
        let (mut app, _) = chunked_app();
        app.on_msg(load_ended(Err("disk gone".into())));
        assert_eq!(app.status, "3 commits — loading stopped: disk gone");
        assert_eq!(app.data.as_ref().unwrap().rows(), 3);
    }

    #[test]
    // Verifies: ADR-0031, G-16
    fn search_reaches_the_chunks_arriving_after_it() {
        let (mut app, mut layout) = chunked_app();
        press(&mut app, KeyCode::Char('/'));
        type_text(&mut app, "commit c");
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Enter);
        assert_eq!((app.search.matches.len(), sha(&app)), (3, "c7".into()));
        app.on_msg(next_chunk(layout.lay_out(line(5, 3))));
        assert_eq!(app.search.matches.len(), 6, "the new rows are searched");
        press(&mut app, KeyCode::Enter);
        assert_eq!(sha(&app), "c6", "the next match follows the one jumped to");
    }

    #[test]
    // Verifies: ADR-0031, G-16
    fn a_search_typed_while_loading_finds_the_first_chunk() {
        let mut layout = Layout::new();
        let mut app = App::new("/repo".into(), Glyphs::Unicode);
        press(&mut app, KeyCode::Char('/'));
        type_text(&mut app, "commit c");
        let first = layout.lay_out(line(8, 3));
        first_chunk(&mut app, first, RepoRefs::default());
        app.on_msg(next_chunk(layout.lay_out(line(5, 3))));
        assert_eq!(app.search.matches, (0..6).collect::<Vec<_>>());
    }

    #[test]
    // Verifies: ADR-0031, G-17
    fn the_lineage_reaches_the_chunks_arriving_after_the_selection() {
        let (mut app, mut layout) = chunked_app();
        assert_eq!(app.lineage.as_ref().unwrap().rows().len(), 3);
        app.on_msg(next_chunk(layout.lay_out(line(5, 3))));
        assert_eq!(
            app.lineage.as_ref().map(Lineage::rows),
            Some(rows_of(&app, &["c8", "c7", "c6", "c5", "c4", "c3"]))
        );
    }

    #[test]
    // Verifies: ADR-0015
    fn the_first_parent_key_follows_gits_first_parent_once_it_has_arrived() {
        // `m`'s first parent `x` arrives in the later chunk, so the layout
        // continues `m` on `b` (ADR-0015); `]` still follows git's first
        // parent, as the detail pane lists it, once `x` is laid out.
        let mut layout = Layout::new();
        let mut app = App::new("/repo".into(), Glyphs::Unicode);
        let first = vec![commit("m", 3, &["x", "b"], &[]), commit("b", 2, &[], &[])];
        first_chunk(&mut app, layout.lay_out(first), RepoRefs::default());
        press(&mut app, KeyCode::Char(']'));
        assert_eq!(sha(&app), "m", "the first parent is not laid out yet");
        app.on_msg(next_chunk(layout.lay_out(vec![commit("x", 1, &[], &[])])));
        press(&mut app, KeyCode::Char(']'));
        assert_eq!(sha(&app), "x");
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
