// Verifies: ADR-0026, ADR-0027
//! The repository sidebar: what a repository *contains* — the way back to its
//! parent, local branches, remotes, working trees, tags and submodules
//! (ADR-0026, ADR-0027, in the terminal form of ADR-0028). It mirrors the
//! desktop's `RepoSidebar.svelte`: collapsible sections, names grouped into
//! folders at their slashes, a filter, and a minimised rail of section glyphs.
//!
//! Rows come in two kinds, as on the desktop. Branches and tags *reveal* their
//! commit in the graph. Working trees, submodules and the parent row *open* a
//! repository: a submodule's commits live in another object database, and a
//! working tree has its own `HEAD` and working-directory node. A row that cannot
//! act is kept, and says why, rather than silently doing nothing.

use ocelli_core::model::{ParentKind, RepoRefs, SubmoduleState};
use std::collections::HashSet;
use std::path::PathBuf;

/// What activating a leaf does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Select this commit in the graph.
    Reveal(String),
    /// Load another repository.
    Open(PathBuf),
    /// Nothing, for the stated reason.
    Blocked(&'static str),
}

/// How a leaf is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Look {
    Parent,
    Head,
    Local,
    Remote,
    Tag,
    Worktree,
    Submodule,
}

/// One ref or checkout location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// Full slash-separated name — what grouping and filtering work on.
    pub name: String,
    /// Trailing annotation: tracking counts, a worktree's branch, a state.
    pub badge: String,
    /// "You are here" in words.
    pub mark: Option<&'static str>,
    pub look: Look,
    pub action: Action,
    /// The remote a remote branch belongs to; `None` for every other entry.
    pub remote: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Local,
    Remote,
    Worktrees,
    Tags,
    Submodules,
}

impl Section {
    pub const ALL: [Section; 5] = [
        Section::Local,
        Section::Remote,
        Section::Worktrees,
        Section::Tags,
        Section::Submodules,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Section::Local => "Local",
            Section::Remote => "Remote",
            Section::Worktrees => "Worktrees",
            Section::Tags => "Tags",
            Section::Submodules => "Submodules",
        }
    }

    /// The glyph the minimised rail shows in place of the title.
    pub fn glyph(self) -> &'static str {
        match self {
            Section::Local => "⎇",
            Section::Remote => "☁",
            Section::Worktrees => "▤",
            Section::Tags => "◈",
            Section::Submodules => "⬢",
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// Everything the sidebar lists, built once per load.
#[derive(Debug, Default)]
pub struct Data {
    pub parent: Option<Item>,
    locals: Vec<Item>,
    remotes: Vec<(String, Vec<Item>)>,
    worktrees: Vec<Item>,
    tags: Vec<Item>,
    submodules: Vec<Item>,
}

fn item(name: &str, badge: impl Into<String>, look: Look, action: Action) -> Item {
    Item {
        name: name.to_string(),
        badge: badge.into(),
        mark: None,
        look,
        action,
        remote: None,
    }
}

/// Ahead/behind only where there is something to say — zeros would be noise.
fn tracking(ahead: usize, behind: usize) -> String {
    let mut parts = Vec::new();
    if ahead > 0 {
        parts.push(format!("{ahead}↑"));
    }
    if behind > 0 {
        parts.push(format!("{behind}↓"));
    }
    parts.join(" ")
}

impl Data {
    pub fn new(refs: &RepoRefs) -> Self {
        let parent = refs.parent.as_ref().map(|p| {
            let badge = match p.kind {
                ParentKind::Superproject => "superproject",
                ParentKind::MainWorktree => "main tree",
            };
            item(
                &p.name,
                badge,
                Look::Parent,
                Action::Open(PathBuf::from(&p.path)),
            )
        });
        let locals = refs
            .locals
            .iter()
            .map(|b| {
                let look = if b.is_head { Look::Head } else { Look::Local };
                let mut it = item(
                    &b.name,
                    tracking(b.ahead, b.behind),
                    look,
                    Action::Reveal(b.sha.clone()),
                );
                it.mark = b.is_head.then_some("checked out");
                it
            })
            .collect();
        let remotes = refs
            .remotes
            .iter()
            .map(|r| {
                let items = r
                    .branches
                    .iter()
                    .map(|b| Item {
                        remote: Some(r.name.clone()),
                        ..item(&b.name, "", Look::Remote, Action::Reveal(b.sha.clone()))
                    })
                    .collect();
                (r.name.clone(), items)
            })
            .collect();
        let worktrees = refs
            .worktrees
            .iter()
            .map(|w| {
                let badge = w.branch.clone().unwrap_or_else(|| "detached".to_string());
                if w.is_current {
                    let mut it = item(
                        &w.name,
                        badge,
                        Look::Head,
                        Action::Blocked("the working tree you are viewing"),
                    );
                    it.mark = Some("viewing");
                    it
                } else {
                    item(
                        &w.name,
                        badge,
                        Look::Worktree,
                        Action::Open(PathBuf::from(&w.path)),
                    )
                }
            })
            .collect();
        let tags = refs
            .tags
            .iter()
            .map(|t| item(&t.name, "", Look::Tag, Action::Reveal(t.sha.clone())))
            .collect();
        let submodules = refs
            .submodules
            .iter()
            .map(|s| match s.state {
                // Opening would need a clone — a write and egress (ADR-0023).
                SubmoduleState::Uninitialized => item(
                    &s.path,
                    "not initialized",
                    Look::Submodule,
                    Action::Blocked("not initialized — no working copy to open"),
                ),
                SubmoduleState::Modified => item(
                    &s.path,
                    "modified",
                    Look::Submodule,
                    Action::Open(PathBuf::from(&s.workdir)),
                ),
                SubmoduleState::InSync => item(
                    &s.path,
                    "",
                    Look::Submodule,
                    Action::Open(PathBuf::from(&s.workdir)),
                ),
            })
            .collect();
        Self {
            parent,
            locals,
            remotes,
            worktrees,
            tags,
            submodules,
        }
    }

    /// Entries of a section that pass the filter.
    fn count(&self, section: Section, filter: &str) -> usize {
        match section {
            Section::Remote => self
                .remotes
                .iter()
                .map(|(_, items)| filtered(items, filter).len())
                .sum(),
            s => filtered(self.items(s), filter).len(),
        }
    }

    fn items(&self, section: Section) -> &[Item] {
        match section {
            Section::Local => &self.locals,
            Section::Remote => &[],
            Section::Worktrees => &self.worktrees,
            Section::Tags => &self.tags,
            Section::Submodules => &self.submodules,
        }
    }
}

/// Case-insensitive substring match over the full name, so an entry stays
/// findable by its folder as well as its last segment. Blank matches all.
fn filtered<'a>(items: &'a [Item], filter: &str) -> Vec<&'a Item> {
    let needle = filter.trim().to_lowercase();
    items
        .iter()
        .filter(|i| needle.is_empty() || i.name.to_lowercase().contains(&needle))
        .collect()
}

/// The view state: which sections, remotes and folders are open, the filter,
/// the selection, and whether the sidebar is minimised to its rail.
#[derive(Debug, Clone)]
pub struct State {
    pub minimised: bool,
    /// Local branches are what a user reaches for first, so that section starts
    /// open and the rest stay out of the way until asked for.
    open: [bool; 5],
    /// Remotes and folders are open unless explicitly closed, so a fresh
    /// repository shows its structure rather than a wall of collapsed rows.
    closed: HashSet<String>,
    pub filter: String,
    /// True while keys go into the filter field.
    pub editing: bool,
    pub selected: usize,
    pub rail_selected: usize,
}

impl Default for State {
    fn default() -> Self {
        Self {
            minimised: false,
            open: [true, false, false, false, false],
            closed: HashSet::new(),
            filter: String::new(),
            editing: false,
            selected: 0,
            rail_selected: 0,
        }
    }
}

impl State {
    pub fn toggle_minimised(&mut self) {
        self.minimised = !self.minimised;
        // The filter field goes away with the lists, and a filter nobody can
        // see must not silently thin the rail's counts.
        if self.minimised {
            self.filter.clear();
            self.editing = false;
        }
    }

    fn group_open(&self, key: &str) -> bool {
        // A filter overrides collapsed folders: every match should be visible.
        !self.filter.trim().is_empty() || !self.closed.contains(key)
    }

    fn toggle_group(&mut self, key: &str) {
        if !self.closed.remove(key) {
            self.closed.insert(key.to_string());
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowKind<'a> {
    Section {
        section: Section,
        open: bool,
        count: usize,
    },
    /// A remote, or a folder of slash-grouped names.
    Group {
        key: String,
        open: bool,
        count: usize,
        /// The checked-out branch lies inside — a collapsed folder must not
        /// hide the one row the user most needs to find.
        has_head: bool,
    },
    Leaf(&'a Item),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row<'a> {
    pub depth: usize,
    /// The leaf's last name segment, a folder's segment, or a title.
    pub label: String,
    pub kind: RowKind<'a>,
}

/// The visible rows of the expanded sidebar.
pub fn rows<'a>(data: &'a Data, state: &State) -> Vec<Row<'a>> {
    let mut out = Vec::new();
    if let Some(parent) = &data.parent {
        out.push(Row {
            depth: 0,
            label: format!("↑ {}", parent.name),
            kind: RowKind::Leaf(parent),
        });
    }
    for section in Section::ALL {
        let count = data.count(section, &state.filter);
        // An empty section has nothing to open; leave it out. While a filter
        // is active it stays, so its zero count says the filter found nothing.
        if count == 0 && state.filter.trim().is_empty() {
            continue;
        }
        let open = state.open[section.index()];
        out.push(Row {
            depth: 0,
            label: section.title().to_string(),
            kind: RowKind::Section {
                section,
                open,
                count,
            },
        });
        if !open {
            continue;
        }
        if section == Section::Remote {
            for (name, items) in &data.remotes {
                let items = filtered(items, &state.filter);
                if items.is_empty() {
                    continue;
                }
                let key = format!("remote/{name}");
                let open = state.group_open(&key);
                out.push(Row {
                    depth: 1,
                    label: name.clone(),
                    kind: RowKind::Group {
                        key: key.clone(),
                        open,
                        count: items.len(),
                        has_head: false,
                    },
                });
                if open {
                    tree(&items, &key, 0, 2, state, &mut out);
                }
            }
        } else {
            let items = filtered(data.items(section), &state.filter);
            tree(&items, section.title(), 0, 1, state, &mut out);
        }
    }
    out
}

/// Groups `items` into folders at their slashes and appends the visible rows.
/// `level` is the number of name segments the enclosing folders already
/// consumed. Names without a slash stay at the top, so flat names stay flat.
fn tree<'a>(
    items: &[&'a Item],
    key: &str,
    level: usize,
    depth: usize,
    state: &State,
    out: &mut Vec<Row<'a>>,
) {
    // Folders first, in the order their first entry appears, then leaves.
    let mut folders: Vec<(String, Vec<&'a Item>)> = Vec::new();
    let mut leaves = Vec::new();
    for &it in items {
        // Empty segments (a leading or doubled slash) would make nameless folders.
        let segments: Vec<&str> = it.name.split('/').filter(|s| !s.is_empty()).collect();
        let rest = &segments[level.min(segments.len())..];
        match rest {
            [] => {}
            [_leaf] => leaves.push(it),
            [folder, ..] => match folders.iter_mut().find(|(f, _)| f == folder) {
                Some((_, v)) => v.push(it),
                None => folders.push((folder.to_string(), vec![it])),
            },
        }
    }
    for (folder, members) in folders {
        let folder_key = format!("{key}/{folder}");
        let open = state.group_open(&folder_key);
        out.push(Row {
            depth,
            label: folder,
            kind: RowKind::Group {
                key: folder_key.clone(),
                open,
                count: members.len(),
                has_head: members.iter().any(|i| i.look == Look::Head),
            },
        });
        if open {
            tree(&members, &folder_key, level + 1, depth + 1, state, out);
        }
    }
    for it in leaves {
        out.push(Row {
            depth,
            label: it.name.rsplit('/').next().unwrap_or(&it.name).to_string(),
            kind: RowKind::Leaf(it),
        });
    }
}

/// One entry of the minimised rail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RailRow<'a> {
    Parent(&'a Item),
    Section { section: Section, count: usize },
}

/// The rail: the way back out, then every section with its count.
pub fn rail<'a>(data: &'a Data) -> Vec<RailRow<'a>> {
    let mut out: Vec<RailRow> = data.parent.iter().map(RailRow::Parent).collect();
    out.extend(Section::ALL.iter().map(|&section| RailRow::Section {
        section,
        count: data.count(section, ""),
    }));
    out
}

/// What the app should do after a row was activated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Activated {
    /// The sidebar changed its own state (a section or folder toggled).
    Nothing,
    Item(Item),
}

/// Activates the selected row of the expanded sidebar.
pub fn activate(data: &Data, state: &mut State) -> Activated {
    let rows = rows(data, state);
    let Some(row) = rows.get(state.selected) else {
        return Activated::Nothing;
    };
    match &row.kind {
        RowKind::Section { section, .. } => {
            let i = section.index();
            state.open[i] = !state.open[i];
        }
        RowKind::Group { key, .. } => {
            let key = key.clone();
            state.toggle_group(&key);
        }
        RowKind::Leaf(item) => return Activated::Item((*item).clone()),
    }
    Activated::Nothing
}

/// Activates the selected rail entry: a section glyph is the only way back
/// from the rail, so it expands the sidebar *and* opens that section.
pub fn activate_rail(data: &Data, state: &mut State) -> Activated {
    let rail = rail(data);
    match rail.get(state.rail_selected) {
        Some(RailRow::Parent(item)) => Activated::Item((*item).clone()),
        Some(RailRow::Section { section, .. }) => {
            let section = *section;
            state.minimised = false;
            state.open[section.index()] = true;
            state.selected = rows(data, state)
                .iter()
                .position(|r| matches!(r.kind, RowKind::Section { section: s, .. } if s == section))
                .unwrap_or(0);
            Activated::Nothing
        }
        None => Activated::Nothing,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use ocelli_core::model::{
        LocalBranch, ParentRepo, Remote, RemoteBranch, Submodule, Tag, Worktree,
    };

    fn local(name: &str, sha: &str, head: bool) -> LocalBranch {
        LocalBranch {
            merged: true,
            name: name.into(),
            sha: sha.into(),
            short_sha: sha.into(),
            is_head: head,
            upstream: None,
            ahead: if head { 2 } else { 0 },
            behind: 0,
        }
    }

    fn submodule(path: &str, state: SubmoduleState) -> Submodule {
        Submodule {
            name: path.into(),
            path: path.into(),
            workdir: format!("/super/{path}"),
            url: None,
            sha: "s".into(),
            short_sha: "s".into(),
            checked_out_short_sha: None,
            state,
        }
    }

    fn worktree(name: &str, current: bool) -> Worktree {
        Worktree {
            name: name.into(),
            path: format!("/trees/{name}"),
            branch: Some(name.into()),
            sha: "d".into(),
            short_sha: "d".into(),
            is_main: !current,
            is_current: current,
        }
    }

    /// Branch `main` (HEAD, 2 ahead) on `d` and `feat/x` on `b`; remote
    /// `origin/main` on `b`; tag `v1` on `c`; two worktrees; two submodules;
    /// and a superproject.
    pub(crate) fn repo_refs() -> RepoRefs {
        RepoRefs {
            locals: vec![local("main", "d", true), local("feat/x", "b", false)],
            remotes: vec![
                Remote {
                    name: "origin".into(),
                    branches: vec![RemoteBranch {
                        name: "main".into(),
                        sha: "b".into(),
                        short_sha: "b".into(),
                    }],
                },
                Remote {
                    name: "empty".into(),
                    branches: vec![],
                },
            ],
            worktrees: vec![worktree("repo", true), worktree("hotfix", false)],
            tags: vec![Tag {
                name: "v1".into(),
                sha: "c".into(),
                short_sha: "c".into(),
                annotated: true,
            }],
            submodules: vec![
                submodule("lib/a", SubmoduleState::InSync),
                submodule("lib/b", SubmoduleState::Uninitialized),
            ],
            parent: Some(ParentRepo {
                kind: ParentKind::Superproject,
                name: "super".into(),
                path: "/super".into(),
            }),
            default_remote: Some("origin".into()),
            in_progress: None,
        }
    }

    pub(crate) fn show(data: &Data, state: &State) -> Vec<String> {
        rows(data, state)
            .iter()
            .map(|r| {
                let indent = "  ".repeat(r.depth);
                match &r.kind {
                    RowKind::Section { open, count, .. } => {
                        format!("{}{} {count}", if *open { "v " } else { "> " }, r.label)
                    }
                    RowKind::Group { open, count, .. } => format!(
                        "{indent}{}{} {count}",
                        if *open { "v " } else { "> " },
                        r.label
                    ),
                    RowKind::Leaf(item) => format!("{indent}{} {}", r.label, item.badge)
                        .trim_end()
                        .to_string(),
                }
            })
            .collect()
    }

    fn open_all(state: &mut State) {
        state.open = [true; 5];
    }

    #[test]
    fn only_local_branches_start_open_and_folders_group_at_slashes() {
        let data = Data::new(&repo_refs());
        let state = State::default();
        assert_eq!(
            show(&data, &state),
            [
                "↑ super superproject",
                "v Local 2",
                "  v feat 1",
                "    x",
                "  main 2↑",
                "> Remote 1",
                "> Worktrees 2",
                "> Tags 1",
                "> Submodules 2",
            ]
        );
    }

    #[test]
    fn every_section_open_shows_remotes_grouped_and_submodule_folders() {
        let data = Data::new(&repo_refs());
        let mut state = State::default();
        open_all(&mut state);
        let shown = show(&data, &state);
        assert!(shown.contains(&"  v origin 1".to_string()), "{shown:#?}");
        assert!(shown.contains(&"    main".to_string()), "{shown:#?}");
        assert!(
            !shown.iter().any(|s| s.contains("empty")),
            "a remote without branches is left out: {shown:#?}"
        );
        assert!(
            shown.contains(&"    b not initialized".to_string()),
            "{shown:#?}"
        );
    }

    #[test]
    fn activating_a_section_or_folder_toggles_it() {
        let data = Data::new(&repo_refs());
        let mut state = State {
            selected: 2, // the `feat` folder
            ..State::default()
        };
        assert_eq!(activate(&data, &mut state), Activated::Nothing);
        assert_eq!(show(&data, &state)[2], "  > feat 1");
        state.selected = 1; // Local
        activate(&data, &mut state);
        assert_eq!(show(&data, &state)[1], "> Local 2");
    }

    #[test]
    fn activating_leaves_hands_back_their_action() {
        let data = Data::new(&repo_refs());
        let mut state = State::default();
        let Activated::Item(parent) = activate(&data, &mut state) else {
            panic!()
        };
        assert_eq!(parent.action, Action::Open("/super".into()));
        state.selected = 4;
        let Activated::Item(main) = activate(&data, &mut state) else {
            panic!()
        };
        assert_eq!(main.action, Action::Reveal("d".into()));
    }

    #[test]
    fn a_filter_matches_full_names_and_opens_closed_folders() {
        let data = Data::new(&repo_refs());
        let mut state = State {
            selected: 2,
            ..State::default()
        };
        activate(&data, &mut state); // close `feat`
        state.filter = "FEAT".into();
        assert_eq!(
            show(&data, &state),
            [
                "↑ super superproject",
                "v Local 1",
                "  v feat 1",
                "    x",
                "> Remote 0",
                "> Worktrees 0",
                "> Tags 0",
                "> Submodules 0",
            ]
        );
    }

    #[test]
    fn the_rail_lists_the_way_back_and_every_section_with_counts() {
        let data = Data::new(&repo_refs());
        let rail = rail(&data);
        assert!(matches!(rail[0], RailRow::Parent(_)));
        assert_eq!(
            rail[1..]
                .iter()
                .map(|r| match r {
                    RailRow::Section { section, count } => format!("{} {count}", section.title()),
                    RailRow::Parent(_) => unreachable!(),
                })
                .collect::<Vec<_>>(),
            [
                "Local 2",
                "Remote 1",
                "Worktrees 2",
                "Tags 1",
                "Submodules 2"
            ]
        );
    }

    #[test]
    fn a_rail_glyph_expands_the_sidebar_on_its_section() {
        let data = Data::new(&repo_refs());
        let mut state = State {
            filter: "x".into(),
            ..State::default()
        };
        state.toggle_minimised();
        assert!(state.minimised && state.filter.is_empty());
        state.rail_selected = 4; // Tags
        activate_rail(&data, &mut state);
        assert!(!state.minimised);
        let shown = show(&data, &state);
        assert_eq!(shown[state.selected], "v Tags 1");
        assert!(shown.contains(&"  v1".to_string()));
    }

    #[test]
    fn rows_that_cannot_act_say_why() {
        let data = Data::new(&repo_refs());
        let blocked: Vec<&'static str> = [&data.worktrees, &data.submodules]
            .into_iter()
            .flatten()
            .filter_map(|i| match i.action {
                Action::Blocked(why) => Some(why),
                _ => None,
            })
            .collect();
        assert_eq!(
            blocked,
            [
                "the working tree you are viewing",
                "not initialized — no working copy to open"
            ]
        );
    }
}
