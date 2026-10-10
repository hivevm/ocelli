//! Every kind of write, each from a repository in the state it is performed in: the state made
//! by `git`, and the write made from what that state shows. A test that sets one argument of
//! such a write to a hostile value reaches the write past its checks of what the repository
//! holds, which a write made up from nothing is refused by first; the test performs each write
//! as made first, to show that it could have. Shared by the writer's hostile test and the remote
//! server's.

// A test binary uses what it needs of the module and leaves the rest.
#![allow(dead_code)]

use crate::shown::{kinds_of_write, stage_in, unstage_in};
use crate::support::{git, git_may_fail, write_file};
use ocelli_core::port::{Conflict, Operation, ResetMode, Take, Write};
use std::path::{Path, PathBuf};

/// What a write needs the repository to hold beside the history.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Nothing changed.
    Clean,
    /// `b` changed and `u` untracked.
    Changed,
    /// `b` changed and staged.
    Staged,
    /// A merge of `clash` stopped on its conflict in `k`.
    Merging,
    /// A cherry-pick of `clash` stopped on `k`, which is resolved and staged.
    Picked,
}

const STATES: [State; 5] = [
    State::Clean,
    State::Changed,
    State::Staged,
    State::Merging,
    State::Picked,
];

impl State {
    fn name(self) -> &'static str {
        match self {
            State::Clean => "clean",
            State::Changed => "changed",
            State::Staged => "staged",
            State::Merging => "merging",
            State::Picked => "picked",
        }
    }
}

/// The repositories the writes are made in, below `dir`: `remote.git`, a bare remote, and one
/// repository for each state, named for it, by Ada, whom its configuration names:
///
/// - `main`: `one` (a, b, k), `two` (a), `three` (k), checked out, tracking `origin/main`
/// - `done` at `two`, merged; `topic` from `two`, adding n; `clash` from `one`, changing k as
///   `main` does; `picked` from `one`, adding p
/// - the lightweight tag `old` at `one`, the annotated tag `v1` at `two`
/// - one stash entry, changing b
/// - the remote `origin` at `remote.git`, with `main` and `feature`, at `topic`
pub struct Repositories(PathBuf);

impl Repositories {
    pub fn new(dir: &Path) -> Repositories {
        let clean = dir.join(State::Clean.name());
        std::fs::create_dir_all(&clean).unwrap();
        history(&clean);
        git(
            dir,
            &[
                "init",
                "--quiet",
                "--bare",
                "--template=",
                "-b",
                "main",
                "remote.git",
            ],
        );
        let remote = dir.join("remote.git");
        quiet(&remote);
        git(&remote, &["config", "receive.autoGc", "false"]);
        git(
            &clean,
            &["remote", "add", "origin", remote.to_str().unwrap()],
        );
        git(&clean, &["push", "--quiet", "-u", "origin", "main"]);
        git(&clean, &["push", "--quiet", "origin", "topic:feature"]);
        git(&clean, &["fetch", "--quiet", "origin"]);
        packed(&clean);
        for state in &STATES[1..] {
            let at = dir.join(state.name());
            copy(&clean, &at);
            match state {
                State::Changed => {
                    write_file(&at, "b", "changed\n");
                    write_file(&at, "u", "untracked\n");
                }
                State::Staged => {
                    write_file(&at, "b", "changed\n");
                    git(&at, &["add", "b"]);
                }
                State::Merging => assert!(!git_may_fail(&at, &["merge", "--quiet", "clash"])),
                State::Picked => {
                    assert!(!git_may_fail(&at, &["cherry-pick", "clash"]));
                    write_file(&at, "k", "resolved\n");
                    git(&at, &["add", "k"]);
                }
                State::Clean => unreachable!(),
            }
            packed(&at);
        }
        Repositories(dir.to_path_buf())
    }

    /// The repository in `state`.
    pub fn of(&self, state: State) -> PathBuf {
        self.0.join(state.name())
    }

    /// A fresh copy of the repository in `state` at `to`, what was there before removed.
    pub fn fresh(&self, state: State, to: &Path) {
        removed(to);
        copy(&self.of(state), to);
    }
}

/// `dir` removed, if it is there. Windows refuses for a moment to remove a file another process,
/// a virus scanner or the server's last read, still has open: the removal is tried a few times.
fn removed(dir: &Path) {
    for _ in 0..20 {
        match std::fs::remove_dir_all(dir) {
            Ok(()) => return,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
            Err(_) => std::thread::sleep(std::time::Duration::from_millis(50)),
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}

/// `git`'s housekeeping turned off in the repository at `dir`, so that no `git` the setup runs
/// leaves one behind that changes the repository while it is copied.
fn quiet(dir: &Path) {
    git(dir, &["config", "maintenance.auto", "false"]);
    git(dir, &["config", "gc.auto", "0"]);
}

/// The repository at `dir` with its objects and refs packed, fewer files for each copy.
fn packed(dir: &Path) {
    git(dir, &["repack", "-a", "-d", "-q"]);
    git(dir, &["pack-refs", "--all"]);
}

fn history(root: &Path) {
    // No template: no sample hooks and descriptions to copy with each repository.
    git(root, &["init", "--quiet", "--template=", "-b", "main"]);
    quiet(root);
    git(root, &["config", "user.name", "Ada"]);
    git(root, &["config", "user.email", "ada@example.invalid"]);
    for f in ["a", "b", "k"] {
        write_file(root, f, "1\n");
    }
    git(root, &["add", "."]);
    git(root, &["commit", "--quiet", "-m", "one"]);
    git(root, &["tag", "old"]);
    write_file(root, "a", "2\n");
    git(root, &["commit", "--quiet", "-am", "two"]);
    git(root, &["branch", "done"]);
    git(root, &["tag", "-a", "v1", "-m", "the first"]);
    git(root, &["switch", "--quiet", "-c", "topic"]);
    write_file(root, "n", "n\n");
    git(root, &["add", "n"]);
    git(root, &["commit", "--quiet", "-m", "topic work"]);
    git(root, &["switch", "--quiet", "-c", "clash", "old"]);
    write_file(root, "k", "clash\n");
    git(root, &["commit", "--quiet", "-am", "clash work"]);
    git(root, &["switch", "--quiet", "-c", "picked", "old"]);
    write_file(root, "p", "p\n");
    git(root, &["add", "p"]);
    git(root, &["commit", "--quiet", "-m", "picked work"]);
    git(root, &["switch", "--quiet", "main"]);
    write_file(root, "k", "main\n");
    git(root, &["commit", "--quiet", "-am", "three"]);
    write_file(root, "b", "stashed\n");
    git(root, &["stash", "push", "--quiet"]);
}

/// A copy of the directory `from` at `to`, each file's content and mode.
fn copy(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target)
                .unwrap_or_else(|e| panic!("{}: {e}", entry.path().display()));
        }
    }
}

/// The commit `name` names in `dir`.
fn rev(dir: &Path, name: &str) -> String {
    git(dir, &["rev-parse", &format!("{name}^{{commit}}")])
}

/// The conflict on `path` in `dir` as the web interface shows it: the blob ids of the index's
/// stages, read off `git`.
fn conflict(dir: &Path, path: &str) -> Conflict {
    let mut stages = [None, None, None];
    for line in git(dir, &["ls-files", "-u", "--", path]).lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        let stage: usize = fields[2].parse().unwrap();
        stages[stage - 1] = Some(fields[1].to_string());
    }
    let [ancestor, ours, theirs] = stages;
    Conflict {
        path: path.into(),
        ancestor,
        ours,
        theirs,
    }
}

/// The toolbar's write on the checked-out `main`, from what `dir` shows.
fn on_main(dir: &Path) -> (String, String) {
    ("main".into(), rev(dir, "main"))
}

/// A write, made from the repository in the state it needs.
pub type Example = (State, fn(&Path) -> Write);

/// One write of every kind the core takes, each with the state it is performed in: each is
/// written, and is not refused, in a fresh copy of the repository in that state.
pub fn examples(repositories: &Repositories) -> Vec<Example> {
    let examples: Vec<Example> = vec![
        (State::Clean, |d| Write::CreateTag {
            name: "made".into(),
            commit: rev(d, "HEAD"),
            message: Some("made".into()),
        }),
        (State::Clean, |d| Write::DeleteTag {
            name: "v1".into(),
            commit: rev(d, "v1"),
            annotated: true,
        }),
        (State::Changed, |_| Write::CreateStash {
            message: Some("wip".into()),
            untracked: true,
        }),
        (State::Clean, |d| Write::ApplyStash {
            index: 0,
            commit: rev(d, "stash@{0}"),
            drop: false,
        }),
        (State::Clean, |d| Write::DropStash {
            index: 0,
            commit: rev(d, "stash@{0}"),
        }),
        (State::Changed, |d| stage_in(d, "b")),
        (State::Staged, |d| unstage_in(d, "b")),
        (State::Staged, |d| Write::Commit {
            message: "made".into(),
            head: Some(rev(d, "HEAD")),
        }),
        (State::Clean, |d| Write::CreateBranch {
            name: "made".into(),
            commit: rev(d, "HEAD"),
            switch: true,
        }),
        (State::Clean, |d| Write::SwitchBranch {
            name: "topic".into(),
            tip: rev(d, "topic"),
        }),
        (State::Clean, |d| Write::Detach {
            commit: rev(d, "old"),
        }),
        (State::Clean, |d| Write::TrackRemote {
            remote: "origin".into(),
            branch: "feature".into(),
            tip: rev(d, "origin/feature"),
        }),
        (State::Clean, |d| Write::DeleteBranch {
            name: "done".into(),
            tip: rev(d, "done"),
            merged: true,
        }),
        (State::Clean, |_| Write::Fetch {
            remote: "origin".into(),
        }),
        (State::Clean, |d| {
            let (branch, tip) = on_main(d);
            Write::Pull { branch, tip }
        }),
        (State::Clean, |d| {
            let (branch, tip) = on_main(d);
            Write::Push { branch, tip }
        }),
        (State::Clean, |d| {
            let (branch, tip) = on_main(d);
            Write::Merge {
                branch,
                tip,
                commit: rev(d, "topic"),
                name: Some("topic".into()),
            }
        }),
        (State::Clean, |d| {
            let (branch, tip) = on_main(d);
            Write::Rebase {
                branch,
                tip,
                onto: rev(d, "picked"),
            }
        }),
        (State::Clean, |d| {
            let (branch, tip) = on_main(d);
            Write::CherryPick {
                branch,
                tip,
                commit: rev(d, "picked"),
            }
        }),
        (State::Clean, |d| {
            let (branch, tip) = on_main(d);
            Write::Revert {
                branch,
                tip,
                commit: rev(d, "main"),
            }
        }),
        (State::Clean, |d| {
            let (branch, tip) = on_main(d);
            Write::Reset {
                branch,
                tip,
                commit: rev(d, "main~1"),
                mode: ResetMode::Hard,
            }
        }),
        (State::Merging, |d| Write::Resolve {
            path: "k".into(),
            conflict: conflict(d, "k"),
            take: Take::Theirs,
        }),
        (State::Picked, |d| Write::Continue {
            operation: Operation::CherryPick,
            head: rev(d, "HEAD"),
        }),
        (State::Merging, |d| Write::Abort {
            operation: Operation::Merge,
            head: rev(d, "HEAD"),
        }),
    ];
    let mut kinds: Vec<String> = examples
        .iter()
        .map(|(state, make)| kind(&make(&repositories.of(*state))))
        .collect();
    kinds.sort();
    let mut all = kinds_of_write();
    all.sort();
    assert_eq!(kinds, all, "one example of every kind of write");
    examples
}

/// The kind of `write`, as serde names it.
pub fn kind(write: &Write) -> String {
    serde_json::to_value(write).unwrap()["kind"]
        .as_str()
        .unwrap()
        .to_string()
}
