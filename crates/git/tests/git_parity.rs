//! Each local write of the port against the `git` command it stands for, in every repository
//! state that changes its outcome (ADR-0033). A state is set up once, then copied twice: `git`
//! runs on one copy and the writer on the other. Where `git` goes through, the writer leaves the
//! same `HEAD`, refs, upstreams, stashes, index, operation in progress, and working directory,
//! ignored files included; where `git` refuses with nothing changed, the writer refuses with
//! nothing changed. A case that departs from `git` on purpose is listed in `DEPARTURES` with the
//! ADR that decides it, and a case where the writer is known to differ from `git` without an ADR
//! is listed in `MISMATCHES`, asserted to still differ, so that a fix shows up as a failure that
//! removes the entry. The configuration a write reads is a state too: `CONFIGURATION` lists each
//! key with the values the states set it to, and a value no state sets fails the comparison.
//!
//! The transfers are left out: they reach a remote, which no state here varies, and `write.rs`
//! and `write_credentials.rs` test them. Resolving, continuing, and aborting a stopped operation
//! act on the operation states alone and are compared with `git` in `history.rs`.

#![allow(
    clippy::disallowed_methods,
    reason = "a test sets a repository up by hand, the way a developer would"
)]
#![cfg(unix)]

mod support;
use ocelli_core::port::{RepositoryWriter, ResetMode, Write};
use ocelli_git::GitWriter;
use std::path::Path;
use std::process::Command;
use support::{Scratch, Snapshot, git, git_output, snapshot, snapshots_as_twins, write_file};

/// Copies the directory `from`, a repository and the linked working tree beside it where there
/// is one, to `to`, and points the two at each other again.
fn copy(from: &Path, to: &Path) {
    let status = Command::new("cp")
        .arg("-a")
        .arg(from)
        .arg(to)
        .status()
        .unwrap();
    assert!(status.success());
    let repo = to.join("repo");
    if to.join("wt").exists() {
        git(&repo, &["worktree", "repair", "../wt"]);
    }
    // The copy has new inodes; `git` and libgit2 then read each file again rather than trust the
    // index's record of it. Refreshing the index exits non-zero where a file is changed.
    git_output(&repo, &["update-index", "-q", "--refresh"]);
}

/// The commits every state starts from.
struct Base {
    one: String,
    two: String,
    main: String,
    other: String,
    ahead: String,
}

/// The message of `other`'s commit, kept as written: trailing space, a comment line, a scissors
/// line, and blank lines at its end, which each mode of `commit.cleanup` cleans up otherwise
/// where a write takes the message over.
const OTHER_MESSAGE: &str = "other work  \n\n# a comment line\n\n\
                             # ------------------------ >8 ------------------------\nbelow\n\n";

/// The message the commit write gives, which each mode of `commit.cleanup` cleans up otherwise.
const COMMIT_MESSAGE: &str = "work  \n\n# a comment line\n\n\
                              # ------------------------ >8 ------------------------\nbelow\n\n";

/// The history every state starts from, in `dir/repo`:
///
/// - `main`: `one` (a, b, k, `.gitignore` ignoring `*.log`), `two` (a), `three` (k, c), `four` (d)
/// - `other` from `three`: a, n, `p/f`, and the ignored name `t.log`, forced in, with a message
///   kept as written; `origin/feature` there
/// - `ahead` from `four`: e, so that a merge of it fast-forwards
/// - `clash` from `one`: k, which conflicts with `main`
/// - `done` at `two`, merged; the lightweight tag `old` at `one`; a stash changing a, with n and
///   `p/f` untracked in it
///
/// So the switch, the merge, the cherry-pick, the rebase, and the reset to `other` touch a, n,
/// `p/f`, and `t.log` and leave b alone; the revert of `two` touches a; applying the stash
/// touches a, n, and `p/f`. A directory at n is one a file replaces, a file at p one a
/// directory replaces.
fn base(dir: &Path) -> Base {
    let d = &dir.join("repo");
    std::fs::create_dir_all(d).unwrap();
    git(d, &["init", "--quiet", "-b", "main"]);
    git(d, &["config", "user.name", "Ada"]);
    git(d, &["config", "user.email", "ada@example.invalid"]);
    write_file(d, ".gitignore", "*.log\n");
    for f in ["a", "b", "k"] {
        write_file(d, f, "1\n");
    }
    git(d, &["add", "."]);
    git(d, &["commit", "--quiet", "-m", "one"]);
    let one = git(d, &["rev-parse", "HEAD"]);
    git(d, &["tag", "old"]);
    write_file(d, "a", "2\n");
    git(d, &["commit", "--quiet", "-am", "two"]);
    let two = git(d, &["rev-parse", "HEAD"]);
    git(d, &["branch", "done"]);
    write_file(d, "k", "main\n");
    write_file(d, "c", "c\n");
    git(d, &["add", "k", "c"]);
    git(d, &["commit", "--quiet", "-m", "three"]);

    git(d, &["switch", "--quiet", "-c", "other"]);
    write_file(d, "a", "other\n");
    write_file(d, "n", "n\n");
    write_file(d, "t.log", "t\n");
    write_file(d, "p/f", "p\n");
    git(d, &["add", "-f", "a", "n", "t.log", "p/f"]);
    git(
        d,
        &[
            "commit",
            "--quiet",
            "--cleanup=verbatim",
            "-m",
            OTHER_MESSAGE,
        ],
    );
    let other = git(d, &["rev-parse", "HEAD"]);
    git(d, &["remote", "add", "origin", "../nowhere"]);
    git(d, &["update-ref", "refs/remotes/origin/feature", &other]);

    git(d, &["switch", "--quiet", "-c", "clash", &one]);
    write_file(d, "k", "clash\n");
    git(d, &["commit", "--quiet", "-am", "clash work"]);

    git(d, &["switch", "--quiet", "main"]);
    write_file(d, "d", "d\n");
    git(d, &["add", "d"]);
    git(d, &["commit", "--quiet", "-m", "four"]);
    let main = git(d, &["rev-parse", "HEAD"]);
    git(d, &["switch", "--quiet", "-c", "ahead"]);
    write_file(d, "e", "e\n");
    git(d, &["add", "e"]);
    git(d, &["commit", "--quiet", "-m", "five"]);
    let ahead = git(d, &["rev-parse", "HEAD"]);
    git(d, &["switch", "--quiet", "main"]);
    write_file(d, "a", "stashed\n");
    write_file(d, "n", "stashed\n");
    write_file(d, "p/f", "stashed\n");
    git(d, &["stash", "push", "--quiet", "-u", "-m", "base"]);
    for (key, _) in CONFIGURATION {
        assert!(
            !git_output(d, &["config", "--local", "--get-all", key]).0,
            "the base sets {key}, so no state leaves it unset"
        );
    }
    Base {
        one,
        two,
        main,
        other,
        ahead,
    }
}

/// Resolves the conflict on k that an operation stopped on, as a developer would.
fn resolve_k(d: &Path) {
    write_file(d, "k", "resolved\n");
    git(d, &["add", "k"]);
}

/// Stages a change to b, which no write touches, so that a commit has something to commit.
fn stage_b(d: &Path) {
    write_file(d, "b", "staged\n");
    git(d, &["add", "b"]);
}

/// Sets `key` to `value` in the repository's configuration.
fn set(d: &Path, key: &str, value: &str) {
    git(d, &["config", key, value]);
}

/// The configuration keys a write of the table reads, each with the values some state sets it
/// to; the base sets none, which stands for each key unset. A key listed here that no state sets
/// fails the comparison. `push.default` is not among them: only a push reads it, and the
/// transfers are compared in `write.rs` (`a_push_follows_push_default_as_git_push_does`).
const CONFIGURATION: &[(&str, &[&str])] = &[
    (
        "commit.cleanup",
        &["strip", "whitespace", "verbatim", "scissors"],
    ),
    ("merge.ff", &["false", "only"]),
    ("merge.suppressDest", &["m*", "release"]),
    ("commit.gpgSign", &["true"]),
    ("tag.gpgSign", &["true"]),
];

/// How a state is made from the base.
type MakeState = fn(&Path);

/// Each state, by name, and how it is made from the base.
const STATES: &[(&str, MakeState)] = &[
    ("clean", |_| {}),
    ("merge in progress", |d| {
        assert!(!git_output(d, &["merge", "--no-edit", "clash"]).0);
        resolve_k(d);
    }),
    ("cherry-pick in progress", |d| {
        assert!(!git_output(d, &["cherry-pick", "clash"]).0);
        resolve_k(d);
    }),
    ("revert in progress", |d| {
        assert!(!git_output(d, &["revert", "--no-edit", "clash"]).0);
        resolve_k(d);
    }),
    ("rebase in progress", |d| {
        assert!(!git_output(d, &["rebase", "clash"]).0);
        resolve_k(d);
    }),
    ("bisect in progress", |d| {
        git(d, &["bisect", "start"]);
    }),
    ("staged change on a touched path", |d| {
        write_file(d, "a", "staged\n");
        git(d, &["add", "a"]);
    }),
    // libgit2's checkout compares the working directory alone with the checked-out commit, so
    // this staged change is one it would overwrite.
    (
        "staged change on a touched path, the working directory as committed",
        |d| {
            write_file(d, "a", "staged\n");
            git(d, &["add", "a"]);
            write_file(d, "a", "2\n");
        },
    ),
    ("staged change on a path left alone", |d| {
        write_file(d, "b", "staged\n");
        git(d, &["add", "b"]);
    }),
    ("unstaged change on a touched path", |d| {
        write_file(d, "a", "unstaged\n")
    }),
    ("unstaged change on a path left alone", |d| {
        write_file(d, "b", "unstaged\n")
    }),
    ("untracked file on a touched path", |d| {
        write_file(d, "n", "untracked\n")
    }),
    ("untracked file on a path left alone", |d| {
        write_file(d, "u", "untracked\n")
    }),
    ("ignored file on a touched path", |d| {
        write_file(d, "t.log", "ignored\n")
    }),
    ("ignored file on a path left alone", |d| {
        write_file(d, "x.log", "ignored\n")
    }),
    ("touched file deleted", |d| {
        std::fs::remove_file(d.join("a")).unwrap()
    }),
    ("file left alone deleted", |d| {
        std::fs::remove_file(d.join("b")).unwrap()
    }),
    ("other checked out in a linked working tree", |d| {
        git(d, &["worktree", "add", "--quiet", "../wt", "other"]);
    }),
    ("clashing names", |d| {
        git(d, &["branch", "new/x"]);
        git(d, &["branch", "feature/x"]);
        git(d, &["tag", "v1/x"]);
    }),
    // `git_command` turns `git`'s signing off, so `git` writes unsigned where Ocelli refuses.
    // A lightweight tag is written alike: where `git tag <name>` under `tag.gpgSign` would make
    // a signed annotated tag, Ocelli makes a lightweight one (ADR-0033), which no `git` run here
    // can show, its signing being off.
    ("signing asked", |d| {
        git(d, &["config", "commit.gpgSign", "true"]);
        git(d, &["config", "tag.gpgSign", "true"]);
    }),
    ("signing asked with a change staged", |d| {
        git(d, &["config", "commit.gpgSign", "true"]);
        stage_b(d);
    }),
    // A target that puts a file where a directory holding files the index does not stands, or
    // a directory where such a file stands: n is a file in `other` and in the stash, p a
    // directory in both.
    ("directory holding untracked files where a file goes", |d| {
        write_file(d, "n/u", "untracked\n")
    }),
    ("directory holding ignored files where a file goes", |d| {
        write_file(d, "n/x.log", "ignored\n")
    }),
    ("untracked file where a directory goes", |d| {
        write_file(d, "p", "untracked\n")
    }),
    ("ignored file where a directory goes", |d| {
        write_file(d, "p", "ignored\n");
        write_file(d, ".git/info/exclude", "/p\n");
    }),
    // The configuration a write reads, each value of `CONFIGURATION`; a commit is compared where
    // a change is staged, the operations where none is.
    ("commit.cleanup strip", |d| {
        set(d, "commit.cleanup", "strip")
    }),
    ("commit.cleanup strip with a change staged", |d| {
        set(d, "commit.cleanup", "strip");
        stage_b(d);
    }),
    ("commit.cleanup whitespace", |d| {
        set(d, "commit.cleanup", "whitespace")
    }),
    ("commit.cleanup whitespace with a change staged", |d| {
        set(d, "commit.cleanup", "whitespace");
        stage_b(d);
    }),
    ("commit.cleanup verbatim", |d| {
        set(d, "commit.cleanup", "verbatim")
    }),
    ("commit.cleanup verbatim with a change staged", |d| {
        set(d, "commit.cleanup", "verbatim");
        stage_b(d);
    }),
    ("commit.cleanup scissors", |d| {
        set(d, "commit.cleanup", "scissors")
    }),
    ("commit.cleanup scissors with a change staged", |d| {
        set(d, "commit.cleanup", "scissors");
        stage_b(d);
    }),
    ("merge.ff false", |d| set(d, "merge.ff", "false")),
    ("merge.ff only", |d| set(d, "merge.ff", "only")),
    ("merge.suppressDest matching the branch", |d| {
        set(d, "merge.suppressDest", "m*")
    }),
    ("merge.suppressDest not matching the branch", |d| {
        set(d, "merge.suppressDest", "release")
    }),
];

/// What a state holds that a write names as the state it expects.
struct Shown<'b> {
    base: &'b Base,
    head: Option<String>,
    stash: String,
    a_workdir: Option<String>,
    a_index: Option<String>,
}

fn shown<'b>(base: &'b Base, d: &Path) -> Shown<'b> {
    let repo = git2::Repository::open(d).unwrap();
    let a = d.join("a");
    Shown {
        base,
        head: repo
            .head()
            .ok()
            .and_then(|h| h.target())
            .map(|o| o.to_string()),
        stash: git(d, &["rev-parse", "stash@{0}"]),
        a_workdir: a.exists().then(|| {
            git2::Oid::hash_file(git2::ObjectType::Blob, &a)
                .unwrap()
                .to_string()
        }),
        a_index: repo
            .index()
            .unwrap()
            .get_path(Path::new("a"), 0)
            .map(|e| e.id.to_string()),
    }
}

type MakeWrite = fn(&Shown) -> Write;

fn reset(s: &Shown, mode: ResetMode) -> Write {
    Write::Reset {
        branch: "main".into(),
        tip: s.base.main.clone(),
        commit: s.base.other.clone(),
        mode,
    }
}

/// Each write, by name, as the port names it and as the `git` command it stands for.
const WRITES: &[(&str, MakeWrite, &[&str])] = &[
    (
        "tag",
        |s| Write::CreateTag {
            name: "v1".into(),
            commit: s.base.main.clone(),
            message: None,
        },
        &["tag", "v1", "main"],
    ),
    (
        "annotated tag",
        |s| Write::CreateTag {
            name: "v1".into(),
            commit: s.base.main.clone(),
            message: Some("release".into()),
        },
        &["tag", "-a", "-m", "release", "v1", "main"],
    ),
    (
        "delete tag",
        |s| Write::DeleteTag {
            name: "old".into(),
            commit: s.base.one.clone(),
            annotated: false,
        },
        &["tag", "-d", "old"],
    ),
    (
        "stash",
        |_| Write::CreateStash {
            message: Some("keep".into()),
            untracked: false,
        },
        &["stash", "push", "-m", "keep"],
    ),
    (
        "stash with untracked files",
        |_| Write::CreateStash {
            message: Some("keep".into()),
            untracked: true,
        },
        &["stash", "push", "-u", "-m", "keep"],
    ),
    (
        "apply stash",
        |s| Write::ApplyStash {
            index: 0,
            commit: s.stash.clone(),
            drop: false,
        },
        &["stash", "apply"],
    ),
    (
        "pop stash",
        |s| Write::ApplyStash {
            index: 0,
            commit: s.stash.clone(),
            drop: true,
        },
        &["stash", "pop"],
    ),
    (
        "drop stash",
        |s| Write::DropStash {
            index: 0,
            commit: s.stash.clone(),
        },
        &["stash", "drop"],
    ),
    (
        "stage",
        |s| Write::StageFile {
            path: "a".into(),
            shown: s.a_workdir.clone(),
        },
        &["add", "--", "a"],
    ),
    (
        "unstage",
        |s| Write::UnstageFile {
            path: "a".into(),
            shown: s.a_index.clone(),
        },
        &["restore", "--staged", "--", "a"],
    ),
    (
        "commit",
        |s| Write::Commit {
            message: COMMIT_MESSAGE.into(),
            head: s.head.clone(),
        },
        &["commit", "-m", COMMIT_MESSAGE],
    ),
    (
        "create branch",
        |s| Write::CreateBranch {
            name: "new".into(),
            commit: s.base.other.clone(),
            switch: false,
        },
        &["branch", "new", "other"],
    ),
    (
        "create and switch",
        |s| Write::CreateBranch {
            name: "new".into(),
            commit: s.base.other.clone(),
            switch: true,
        },
        &["switch", "-c", "new", "other"],
    ),
    (
        "switch",
        |s| Write::SwitchBranch {
            name: "other".into(),
            tip: s.base.other.clone(),
        },
        &["switch", "other"],
    ),
    (
        "detach",
        |s| Write::Detach {
            commit: s.base.other.clone(),
        },
        &["switch", "--detach", "other"],
    ),
    (
        "track",
        |s| Write::TrackRemote {
            remote: "origin".into(),
            branch: "feature".into(),
            tip: s.base.other.clone(),
        },
        &["switch", "--track", "origin/feature"],
    ),
    (
        "delete branch",
        |s| Write::DeleteBranch {
            name: "other".into(),
            tip: s.base.other.clone(),
            merged: false,
        },
        &["branch", "-D", "other"],
    ),
    (
        "delete merged branch",
        |s| Write::DeleteBranch {
            name: "done".into(),
            tip: s.base.two.clone(),
            merged: true,
        },
        &["branch", "-d", "done"],
    ),
    (
        "merge",
        |s| Write::Merge {
            branch: "main".into(),
            tip: s.base.main.clone(),
            commit: s.base.other.clone(),
            name: Some("other".into()),
        },
        &["merge", "--no-edit", "other"],
    ),
    (
        "fast-forward merge",
        |s| Write::Merge {
            branch: "main".into(),
            tip: s.base.main.clone(),
            commit: s.base.ahead.clone(),
            name: Some("ahead".into()),
        },
        &["merge", "--no-edit", "ahead"],
    ),
    (
        "rebase",
        |s| Write::Rebase {
            branch: "main".into(),
            tip: s.base.main.clone(),
            onto: s.base.other.clone(),
        },
        &["rebase", "other"],
    ),
    (
        "cherry-pick",
        |s| Write::CherryPick {
            branch: "main".into(),
            tip: s.base.main.clone(),
            commit: s.base.other.clone(),
        },
        &["cherry-pick", "other"],
    ),
    (
        "revert",
        |s| Write::Revert {
            branch: "main".into(),
            tip: s.base.main.clone(),
            commit: s.base.two.clone(),
        },
        &["revert", "--no-edit", "done"],
    ),
    (
        "soft reset",
        |s| reset(s, ResetMode::Soft),
        &["reset", "--soft", "other"],
    ),
    (
        "mixed reset",
        |s| reset(s, ResetMode::Mixed),
        &["reset", "--mixed", "-q", "other"],
    ),
    (
        "hard reset",
        |s| reset(s, ResetMode::Hard),
        &["reset", "--hard", "-q", "other"],
    ),
];

/// The history operations that write a history from the checked-out branch (ADR-0033).
const OPERATIONS: &[&str] = &[
    "merge",
    "fast-forward merge",
    "rebase",
    "cherry-pick",
    "revert",
];
/// The history operations, the resets included (ADR-0033).
const HISTORY: &[&str] = &[
    "merge",
    "fast-forward merge",
    "rebase",
    "cherry-pick",
    "revert",
    "soft reset",
    "mixed reset",
    "hard reset",
];
/// The states with an operation in progress that `git commit` would conclude.
const STOPPED: &[&str] = &[
    "merge in progress",
    "cherry-pick in progress",
    "revert in progress",
    "rebase in progress",
];
/// The states with an operation in progress, a bisect included.
const IN_PROGRESS: &[&str] = &[
    "merge in progress",
    "cherry-pick in progress",
    "revert in progress",
    "rebase in progress",
    "bisect in progress",
];
/// The states with a change to a tracked file.
const TRACKED_CHANGES: &[&str] = &[
    "staged change on a touched path",
    "staged change on a touched path, the working directory as committed",
    "staged change on a path left alone",
    "unstaged change on a touched path",
    "unstaged change on a path left alone",
    "touched file deleted",
    "file left alone deleted",
    "signing asked with a change staged",
    "commit.cleanup strip with a change staged",
    "commit.cleanup whitespace with a change staged",
    "commit.cleanup verbatim with a change staged",
    "commit.cleanup scissors with a change staged",
];
/// The states with a change to a tracked file or an operation stopped with its own.
const STASHED: &[&str] = &[
    "staged change on a touched path",
    "staged change on a touched path, the working directory as committed",
    "staged change on a path left alone",
    "unstaged change on a touched path",
    "unstaged change on a path left alone",
    "touched file deleted",
    "file left alone deleted",
    "signing asked with a change staged",
    "commit.cleanup strip with a change staged",
    "commit.cleanup whitespace with a change staged",
    "commit.cleanup verbatim with a change staged",
    "commit.cleanup scissors with a change staged",
];

/// The states with a change a stash with the untracked files takes.
const STASHED_UNTRACKED: &[&str] = &[
    "staged change on a touched path",
    "staged change on a touched path, the working directory as committed",
    "staged change on a path left alone",
    "unstaged change on a touched path",
    "unstaged change on a path left alone",
    "touched file deleted",
    "file left alone deleted",
    "signing asked with a change staged",
    "commit.cleanup strip with a change staged",
    "commit.cleanup whitespace with a change staged",
    "commit.cleanup verbatim with a change staged",
    "commit.cleanup scissors with a change staged",
    "untracked file on a touched path",
    "untracked file on a path left alone",
    "directory holding untracked files where a file goes",
    "untracked file where a directory goes",
];

/// The states where `git stash apply` writes part of the stash, its untracked files or its
/// changes, before it finds a file in its way and stops.
const APPLIED_IN_PART: &[&str] = &[
    "staged change on a touched path, the working directory as committed",
    "unstaged change on a touched path",
    "untracked file on a touched path",
    "directory holding untracked files where a file goes",
    "directory holding ignored files where a file goes",
    "untracked file where a directory goes",
    "ignored file where a directory goes",
];

/// A group of cases: the states, the writes, and why they are listed.
type Cases = (
    &'static [&'static str],
    &'static [&'static str],
    &'static str,
);

/// Where the writer refuses, on purpose, what `git` does, and the ADR that decides it. A case
/// listed here is refused with nothing changed; `git` may refuse it too, but each group holds a
/// case where `git` changes the repository, or the group decides nothing.
const DEPARTURES: &[Cases] = &[
    // An operation starts from a clean tree, stricter than `git`, which carries along changes it
    // does not touch.
    (TRACKED_CHANGES, OPERATIONS, "ADR-0033"),
    // An operation, a reset included, starts where nothing is in progress, not even a bisect.
    (IN_PROGRESS, HISTORY, "ADR-0033"),
    // A stopped operation is concluded by continuing it, which commits as the operation would.
    (STOPPED, &["commit"], "ADR-0033"),
    // A commit and an annotated tag are refused where signing is asked; the operations commit.
    (
        &["signing asked"],
        &["annotated tag", "merge", "rebase", "cherry-pick", "revert"],
        "ADR-0033",
    ),
    (
        &["signing asked with a change staged"],
        &["commit"],
        "ADR-0033",
    ),
    // A write that would leave the working directory in conflict refuses, where `git stash
    // apply` stops in the conflict.
    (
        &["staged change on a touched path"],
        &["apply stash", "pop stash"],
        "ADR-0033",
    ),
    // A stash is made, applied, and popped only where nothing is in progress, which `git stash`
    // would drop, and applied only over an index that holds no staged change and a working
    // directory that holds every file it changes (ADR-0033).
    (
        STOPPED,
        &[
            "stash",
            "stash with untracked files",
            "apply stash",
            "pop stash",
        ],
        "ADR-0033",
    ),
    (
        &[
            "staged change on a path left alone",
            "signing asked with a change staged",
            "commit.cleanup strip with a change staged",
            "commit.cleanup whitespace with a change staged",
            "commit.cleanup verbatim with a change staged",
            "commit.cleanup scissors with a change staged",
            "touched file deleted",
        ],
        &["apply stash", "pop stash"],
        "ADR-0033",
    ),
    // A write that would replace an ignored file, a directory holding one where a file goes or
    // one where a directory goes, is refused, where `git` deletes it and goes on (ADR-0033).
    (
        &[
            "directory holding ignored files where a file goes",
            "ignored file where a directory goes",
        ],
        &[
            "create and switch",
            "switch",
            "detach",
            "track",
            "merge",
            "rebase",
            "cherry-pick",
        ],
        "ADR-0033",
    ),
    // A refusal changes nothing, where `git switch -c` writes the target's tree to the index and
    // the working directory before it finds the name taken, and stops there.
    (
        &["clashing names"],
        &["create and switch", "track"],
        "ADR-0033",
    ),
    // A refusal changes nothing, where `git stash apply` writes the stash's untracked files, or
    // its changes, before it finds a file in its way, and stops there.
    (APPLIED_IN_PART, &["apply stash", "pop stash"], "ADR-0033"),
];

/// Where the writer differs from `git` and no ADR decides it: each group a finding for the
/// maintainer, asserted case by case to still differ, so that a fix fails here and removes it.
const MISMATCHES: &[Cases] = &[
    (
        STASHED,
        &["stash"],
        "the stash commit's message ends in a line break and its index commit's in two, where \
         git's end in none and in one",
    ),
    (
        STASHED_UNTRACKED,
        &["stash with untracked files"],
        "the stash, index, and untracked commits' messages end in one line break more than \
         git's",
    ),
];

/// The files of an operation the writer is known to leave otherwise than `git` in every state
/// without an ADR, left out where the two are compared: `git`'s merge leaves `AUTO_MERGE`, the
/// tree it merged, after a cherry-pick, a revert, a rebase, and a stash applied, which libgit2's
/// merge never writes. A case where it differs is asserted to occur, so that a fix shows up as a
/// failure that removes the entry.
const UNCOMPARED: &[&str] = &["AUTO_MERGE"];

/// What each side did with one write in one state.
struct Outcome {
    before: Snapshot,
    git_ok: bool,
    git_said: String,
    by_git: Snapshot,
    ocelli: Result<(), String>,
    by_ocelli: Snapshot,
    /// What `git` and the writer left, read as twins: a commit one side made alone by content.
    alike: (Snapshot, Snapshot),
}

impl Outcome {
    /// The writer left what `git` left: where `git` refused with nothing changed, it refused
    /// with nothing changed; where `git` went through, or stopped halfway, it left the same.
    fn as_git(&self) -> bool {
        if !self.git_ok && self.by_git == self.before {
            self.refused()
        } else {
            self.alike.0.without(UNCOMPARED) == self.alike.1.without(UNCOMPARED)
        }
    }

    /// The writer left a file of `UNCOMPARED` otherwise than `git`.
    fn uncompared_differ(&self) -> bool {
        self.alike.0 != self.alike.1
            && self.alike.0.without(UNCOMPARED) == self.alike.1.without(UNCOMPARED)
    }

    /// The writer refused with nothing changed.
    fn refused(&self) -> bool {
        self.ocelli.is_err() && self.by_ocelli == self.before
    }

    /// `git` changed the repository, going through or stopping halfway.
    fn git_changed(&self) -> bool {
        self.by_git != self.before
    }

    fn describe(&self) -> String {
        let diff = self
            .alike
            .0
            .differences(&self.alike.1, ("by git", "by Ocelli"));
        format!(
            "  git {}{}, Ocelli {:?}; git changed {}, Ocelli changed {}\n{}",
            if self.git_ok {
                "went through"
            } else {
                "refused: "
            },
            if self.git_ok {
                ""
            } else {
                self.git_said.trim()
            },
            self.ocelli,
            self.by_git != self.before,
            self.by_ocelli != self.before,
            diff.join("\n")
        )
    }
}

/// The verdict the tables give a write in a state.
#[derive(Debug, PartialEq)]
enum Expected {
    AsGit,
    /// Refused, as the group of `DEPARTURES` at this index decides.
    RefusedPer(usize, &'static str),
    KnownMismatch(&'static str),
}

fn listed(cases: &Cases, state: &str, write: &str) -> bool {
    cases.0.contains(&state) && cases.1.contains(&write)
}

fn expected(state: &str, write: &str) -> Expected {
    let departure = DEPARTURES.iter().position(|c| listed(c, state, write));
    let mismatch = MISMATCHES.iter().find(|c| listed(c, state, write));
    match (departure, mismatch) {
        (Some(_), Some(_)) => panic!("{write} in '{state}' is both a departure and a mismatch"),
        (Some(group), None) => Expected::RefusedPer(group, DEPARTURES[group].2),
        (None, Some(cases)) => Expected::KnownMismatch(cases.2),
        (None, None) => Expected::AsGit,
    }
}

/// What one state's cases came to: the cases that do not hold, and the groups of `DEPARTURES`
/// with a case where `git` changed the repository, and whether a case differed in `UNCOMPARED`
/// alone, and the state's configuration, as `git config --local --list` prints it.
struct Verdicts {
    failures: Vec<String>,
    departures_taken: Vec<usize>,
    uncompared_differ: bool,
    config: String,
}

/// Runs every write in one state.
fn run_state(root: &Path, origin: &Path, base: &Base, index: usize) -> Verdicts {
    let (state, make) = STATES[index];
    let dir = root.join(format!("state-{index}"));
    copy(origin, &dir);
    make(&dir.join("repo"));
    let before = snapshot(&dir.join("repo"));
    let shown = shown(base, &dir.join("repo"));
    let mut verdicts = Verdicts {
        failures: Vec::new(),
        departures_taken: Vec::new(),
        uncompared_differ: false,
        config: before.config.clone(),
    };
    for (case, (write, make_write, args)) in WRITES.iter().enumerate() {
        let by_git_dir = root.join(format!("state-{index}-{case}-git"));
        let by_ocelli_dir = root.join(format!("state-{index}-{case}-ocelli"));
        copy(&dir, &by_git_dir);
        copy(&dir, &by_ocelli_dir);
        let (git_ok, git_said) = git_output(&by_git_dir.join("repo"), args);
        let ocelli = GitWriter
            .write(&by_ocelli_dir.join("repo"), &make_write(&shown))
            .map(|_| ())
            .map_err(|e| e.to_string());
        let (git_repo, ocelli_repo) = (by_git_dir.join("repo"), by_ocelli_dir.join("repo"));
        let (by_git, by_ocelli) = (snapshot(&git_repo), snapshot(&ocelli_repo));
        let alike = snapshots_as_twins(
            (&git_repo, by_git.clone()),
            (&ocelli_repo, by_ocelli.clone()),
        );
        let outcome = Outcome {
            before: before.clone(),
            git_ok,
            git_said,
            by_git,
            ocelli,
            by_ocelli,
            alike,
        };
        let _ = std::fs::remove_dir_all(&by_git_dir);
        let _ = std::fs::remove_dir_all(&by_ocelli_dir);
        let verdict = expected(state, write);
        verdicts.uncompared_differ |= outcome.uncompared_differ();
        let holds = match verdict {
            Expected::AsGit => outcome.as_git(),
            Expected::RefusedPer(group, _) => {
                if outcome.git_changed() {
                    verdicts.departures_taken.push(group);
                }
                outcome.refused()
            }
            Expected::KnownMismatch(_) => !outcome.as_git(),
        };
        if !holds {
            verdicts.failures.push(format!(
                "{write} in the state '{state}', expected {verdict:?}:\n{}",
                outcome.describe()
            ));
        }
    }
    verdicts
}

#[test]
// Verifies: ADR-0033
fn each_write_does_what_git_does_in_every_state_or_an_adr_says_why_not() {
    for (states, writes, _) in DEPARTURES.iter().chain(MISMATCHES) {
        for s in *states {
            assert!(STATES.iter().any(|(n, _)| n == s), "no state '{s}'");
        }
        for w in *writes {
            assert!(WRITES.iter().any(|(n, _, _)| n == w), "no write '{w}'");
        }
    }
    let scratch = Scratch::new("states");
    let origin = scratch.0.join("origin");
    let base = base(&origin);
    let verdicts: Vec<Verdicts> = std::thread::scope(|scope| {
        let runs: Vec<_> = (0..STATES.len())
            .map(|i| {
                let (root, origin, base) = (&scratch.0, &origin, &base);
                scope.spawn(move || run_state(root, origin, base, i))
            })
            .collect();
        runs.into_iter().map(|r| r.join().unwrap()).collect()
    });
    let mut failures: Vec<String> = verdicts.iter().flat_map(|v| v.failures.clone()).collect();
    for (group, (states, writes, adr)) in DEPARTURES.iter().enumerate() {
        if !verdicts.iter().any(|v| v.departures_taken.contains(&group)) {
            failures.push(format!(
                "{writes:?} in {states:?}: git refuses every case with nothing changed, so \
                 {adr} decides nothing here"
            ));
        }
    }
    for (key, values) in CONFIGURATION {
        for value in *values {
            // `git config --list` prints a key's section and name in lower case.
            let line = format!("{}={value}", key.to_lowercase());
            if !verdicts.iter().any(|v| v.config.lines().any(|l| l == line)) {
                failures.push(format!("no state sets {key} to {value}"));
            }
        }
    }
    if !verdicts.iter().any(|v| v.uncompared_differ) {
        failures.push(format!(
            "{UNCOMPARED:?}: the writer leaves them as git does in every case, so they can be \
             compared"
        ));
    }
    assert!(
        failures.is_empty(),
        "{} cases differ from what the tables expect:\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}
