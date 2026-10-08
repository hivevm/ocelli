//! Each local write of the port against the `git` command it stands for, in every repository
//! state that changes its outcome (docs/CONVENTIONS.md, the entry on writes that do what `git`
//! does). A state is set up once, then copied twice: `git` runs on one copy and the writer on the
//! other. Where `git` goes through, the writer leaves the same `HEAD`, refs, upstreams, stashes,
//! index, operation in progress, and working directory, ignored files included; where `git`
//! refuses with nothing changed, the writer refuses with nothing changed. A case that departs
//! from `git` on purpose is listed in `DEPARTURES` with the ADR that decides it, and a case where
//! the writer is known to differ from `git` without an ADR is listed in `MISMATCHES`, asserted to
//! still differ, so that a fix shows up as a failure that removes the entry.
//!
//! The transfers are left out: they reach a remote, which no state here varies, and `write.rs`
//! and `write_credentials.rs` test them. Resolving, continuing, and aborting a stopped operation
//! act on the operation states alone and are compared with `git` in `history.rs`.

#![allow(
    clippy::disallowed_methods,
    reason = "a test sets a repository up by hand, the way a developer would"
)]
#![cfg(unix)]

mod git_command;
use git_command::git_command;
use ocelli_core::port::{RepositoryWriter, ResetMode, Write};
use ocelli_git::GitWriter;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::Command;

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("ocelli-parity-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Runs `git` and returns whether it succeeded and what it printed.
fn run(dir: &Path, args: &[&str]) -> (bool, String) {
    let out = git_command(dir).args(args).output().unwrap();
    let text = if out.status.success() {
        String::from_utf8_lossy(&out.stdout).into_owned()
    } else {
        String::from_utf8_lossy(&out.stderr).into_owned()
    };
    (out.status.success(), text)
}

fn git(dir: &Path, args: &[&str]) -> String {
    let (ok, text) = run(dir, args);
    assert!(ok, "git {args:?}: {text}");
    text.trim_end().to_string()
}

fn put(dir: &Path, path: &str, content: &str) {
    std::fs::write(dir.join(path), content).unwrap();
}

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
    run(&repo, &["update-index", "-q", "--refresh"]);
}

/// The commits every state starts from.
struct Base {
    one: String,
    two: String,
    main: String,
    other: String,
}

/// The history every state starts from, in `dir/repo`:
///
/// - `main`: `one` (a, b, k, `.gitignore` ignoring `*.log`), `two` (a), `three` (k, c), `four` (d)
/// - `other` from `three`: a, n, and the ignored name `t.log`, forced in; `origin/feature` there
/// - `clash` from `one`: k, which conflicts with `main`
/// - `done` at `two`, merged; the lightweight tag `old` at `one`; a stash changing a
///
/// So the switch, the merge, the cherry-pick, the rebase, and the reset to `other` touch a, n,
/// and `t.log` and leave b alone; the revert of `two` and the stash touch a.
fn base(dir: &Path) -> Base {
    let d = &dir.join("repo");
    std::fs::create_dir_all(d).unwrap();
    git(d, &["init", "--quiet", "-b", "main"]);
    git(d, &["config", "user.name", "Ada"]);
    git(d, &["config", "user.email", "ada@example.invalid"]);
    put(d, ".gitignore", "*.log\n");
    for f in ["a", "b", "k"] {
        put(d, f, "1\n");
    }
    git(d, &["add", "."]);
    git(d, &["commit", "--quiet", "-m", "one"]);
    let one = git(d, &["rev-parse", "HEAD"]);
    git(d, &["tag", "old"]);
    put(d, "a", "2\n");
    git(d, &["commit", "--quiet", "-am", "two"]);
    let two = git(d, &["rev-parse", "HEAD"]);
    git(d, &["branch", "done"]);
    put(d, "k", "main\n");
    put(d, "c", "c\n");
    git(d, &["add", "k", "c"]);
    git(d, &["commit", "--quiet", "-m", "three"]);

    git(d, &["switch", "--quiet", "-c", "other"]);
    put(d, "a", "other\n");
    put(d, "n", "n\n");
    put(d, "t.log", "t\n");
    git(d, &["add", "-f", "a", "n", "t.log"]);
    git(d, &["commit", "--quiet", "-m", "other work"]);
    let other = git(d, &["rev-parse", "HEAD"]);
    git(d, &["remote", "add", "origin", "../nowhere"]);
    git(d, &["update-ref", "refs/remotes/origin/feature", &other]);

    git(d, &["switch", "--quiet", "-c", "clash", &one]);
    put(d, "k", "clash\n");
    git(d, &["commit", "--quiet", "-am", "clash work"]);

    git(d, &["switch", "--quiet", "main"]);
    put(d, "d", "d\n");
    git(d, &["add", "d"]);
    git(d, &["commit", "--quiet", "-m", "four"]);
    let main = git(d, &["rev-parse", "HEAD"]);
    put(d, "a", "stashed\n");
    git(d, &["stash", "push", "--quiet", "-m", "base"]);
    Base {
        one,
        two,
        main,
        other,
    }
}

/// Resolves the conflict on k that an operation stopped on, as a developer would.
fn resolve_k(d: &Path) {
    put(d, "k", "resolved\n");
    git(d, &["add", "k"]);
}

/// How a state is made from the base.
type MakeState = fn(&Path);

/// Each state, by name, and how it is made from the base.
const STATES: &[(&str, MakeState)] = &[
    ("clean", |_| {}),
    ("merge in progress", |d| {
        assert!(!run(d, &["merge", "--no-edit", "clash"]).0);
        resolve_k(d);
    }),
    ("cherry-pick in progress", |d| {
        assert!(!run(d, &["cherry-pick", "clash"]).0);
        resolve_k(d);
    }),
    ("revert in progress", |d| {
        assert!(!run(d, &["revert", "--no-edit", "clash"]).0);
        resolve_k(d);
    }),
    ("rebase in progress", |d| {
        assert!(!run(d, &["rebase", "clash"]).0);
        resolve_k(d);
    }),
    ("bisect in progress", |d| {
        git(d, &["bisect", "start"]);
    }),
    ("staged change on a touched path", |d| {
        put(d, "a", "staged\n");
        git(d, &["add", "a"]);
    }),
    // libgit2's checkout compares the working directory alone with the checked-out commit, so
    // this staged change is one it would overwrite.
    (
        "staged change on a touched path, the working directory as committed",
        |d| {
            put(d, "a", "staged\n");
            git(d, &["add", "a"]);
            put(d, "a", "2\n");
        },
    ),
    ("staged change on a path left alone", |d| {
        put(d, "b", "staged\n");
        git(d, &["add", "b"]);
    }),
    ("unstaged change on a touched path", |d| {
        put(d, "a", "unstaged\n")
    }),
    ("unstaged change on a path left alone", |d| {
        put(d, "b", "unstaged\n")
    }),
    ("untracked file on a touched path", |d| {
        put(d, "n", "untracked\n")
    }),
    ("untracked file on a path left alone", |d| {
        put(d, "u", "untracked\n")
    }),
    ("ignored file on a touched path", |d| {
        put(d, "t.log", "ignored\n")
    }),
    ("ignored file on a path left alone", |d| {
        put(d, "x.log", "ignored\n")
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
        put(d, "b", "staged\n");
        git(d, &["add", "b"]);
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
            message: "work".into(),
            head: s.head.clone(),
        },
        &["commit", "-m", "work"],
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

/// The operations of ADR-0033 that write a history from the checked-out branch.
const OPERATIONS: &[&str] = &["merge", "rebase", "cherry-pick", "revert"];
/// The operations of ADR-0033, the resets included.
const HISTORY: &[&str] = &[
    "merge",
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
            "touched file deleted",
        ],
        &["apply stash", "pop stash"],
        "ADR-0033",
    ),
    // A refusal changes nothing, where `git switch -c` writes the target's tree to the index and
    // the working directory before it finds the name taken, and stops there.
    (
        &["clashing names"],
        &["create and switch", "track"],
        "ADR-0033",
    ),
];

/// Where the writer differs from `git` and no ADR decides it: each group a finding for the
/// maintainer, asserted case by case to still differ, so that a fix fails here and removes it.
const MISMATCHES: &[Cases] = &[];

/// Everything a write can change, read with `git`.
#[derive(Debug, PartialEq, Eq, Clone)]
struct Snapshot {
    head: String,
    refs: Vec<String>,
    stashes: Vec<String>,
    upstreams: String,
    index: String,
    status: String,
    in_progress: Vec<&'static str>,
    files: BTreeMap<String, String>,
}

/// A commit as content, its id aside: two copies make the same commit at different times.
fn fingerprint(sha: &str, commits: &HashMap<String, (String, Vec<String>)>) -> String {
    match commits.get(sha) {
        Some((own, parents)) => {
            let parents: Vec<_> = parents.iter().map(|p| fingerprint(p, commits)).collect();
            format!("{own} <- [{}]", parents.join(", "))
        }
        None => format!("unknown {sha}"),
    }
}

fn files(root: &Path, dir: &Path, found: &mut BTreeMap<String, String>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let name = path.strip_prefix(root).unwrap().display().to_string();
        if name == ".git" {
            continue;
        }
        if path.is_dir() {
            files(root, &path, found);
        } else {
            let content = std::fs::read(&path).unwrap();
            found.insert(name, String::from_utf8_lossy(&content).into_owned());
        }
    }
}

fn snapshot(d: &Path) -> Snapshot {
    let mut commits = HashMap::new();
    let log = git(
        d,
        &[
            "log",
            "--all",
            "--reflog",
            "-z",
            "--format=%H%x1f%T%x1f%P%x1f%an <%ae>%x1f%B",
        ],
    );
    for record in log.split('\0').filter(|r| !r.is_empty()) {
        let fields: Vec<_> = record.splitn(5, '\x1f').collect();
        let own = format!("{} by {}: {:?}", fields[1], fields[3], fields[4]);
        let parents = fields[2].split_whitespace().map(str::to_string).collect();
        commits.insert(fields[0].trim().to_string(), (own, parents));
    }
    let head = match run(d, &["symbolic-ref", "-q", "HEAD"]) {
        (true, name) => name.trim().to_string(),
        _ => "detached".to_string(),
    };
    let head_sha = run(d, &["rev-parse", "-q", "--verify", "HEAD"]).1;
    let head = format!("{head} at {}", fingerprint(head_sha.trim(), &commits));
    let refs = git(
        d,
        &[
            "for-each-ref",
            "--format=%(refname)%1f%(objecttype)%1f%(objectname)%1f%(*objectname)%1f%(contents)%1e",
        ],
    );
    let refs = refs
        .split('\x1e')
        .map(str::trim)
        .filter(|r| !r.is_empty() && !r.starts_with("refs/stash"))
        .map(|r| {
            let f: Vec<_> = r.splitn(5, '\x1f').collect();
            match f[1] {
                "tag" => format!("{} tag {:?} on {}", f[0], f[4], fingerprint(f[3], &commits)),
                _ => format!("{} {}", f[0], fingerprint(f[2], &commits)),
            }
        })
        .collect();
    let stashes = git(d, &["stash", "list", "--format=%H"])
        .lines()
        .map(|s| fingerprint(s, &commits))
        .collect();
    let upstreams = run(d, &["config", "--get-regexp", r"^branch\."]).1;
    let status = git(
        d,
        &[
            "status",
            "--porcelain=v1",
            "--ignored",
            "--untracked-files=all",
        ],
    );
    let index = git(d, &["ls-files", "--stage"]);
    let git_dir = d.join(".git");
    let in_progress = [
        "MERGE_HEAD",
        "CHERRY_PICK_HEAD",
        "REVERT_HEAD",
        "rebase-merge",
        "rebase-apply",
        "BISECT_LOG",
        "sequencer",
    ]
    .into_iter()
    .filter(|f| git_dir.join(f).exists())
    .collect();
    let mut found = BTreeMap::new();
    files(d, d, &mut found);
    Snapshot {
        head,
        refs,
        stashes,
        upstreams,
        index,
        status,
        in_progress,
        files: found,
    }
}

/// What each side did with one write in one state.
struct Outcome {
    before: Snapshot,
    git_ok: bool,
    git_said: String,
    by_git: Snapshot,
    ocelli: Result<(), String>,
    by_ocelli: Snapshot,
}

impl Outcome {
    /// The writer left what `git` left: where `git` refused with nothing changed, it refused
    /// with nothing changed; where `git` went through, or stopped halfway, it left the same.
    fn as_git(&self) -> bool {
        if !self.git_ok && self.by_git == self.before {
            self.refused()
        } else {
            self.by_ocelli == self.by_git
        }
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
        let mut diff = Vec::new();
        let (g, o) = (&self.by_git, &self.by_ocelli);
        let fields: [(&str, String, String); 8] = [
            ("HEAD", g.head.clone(), o.head.clone()),
            ("refs", format!("{:#?}", g.refs), format!("{:#?}", o.refs)),
            (
                "stashes",
                format!("{:#?}", g.stashes),
                format!("{:#?}", o.stashes),
            ),
            ("upstreams", g.upstreams.clone(), o.upstreams.clone()),
            ("index", g.index.clone(), o.index.clone()),
            ("status", g.status.clone(), o.status.clone()),
            (
                "in progress",
                format!("{:?}", g.in_progress),
                format!("{:?}", o.in_progress),
            ),
            (
                "files",
                format!("{:#?}", g.files),
                format!("{:#?}", o.files),
            ),
        ];
        for (name, by_git, by_ocelli) in fields {
            if by_git != by_ocelli {
                diff.push(format!(
                    "    {name} by git:\n{by_git}\n    {name} by Ocelli:\n{by_ocelli}"
                ));
            }
        }
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
/// with a case where `git` changed the repository.
struct Verdicts {
    failures: Vec<String>,
    departures_taken: Vec<usize>,
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
    };
    for (case, (write, make_write, args)) in WRITES.iter().enumerate() {
        let by_git_dir = root.join(format!("state-{index}-{case}-git"));
        let by_ocelli_dir = root.join(format!("state-{index}-{case}-ocelli"));
        copy(&dir, &by_git_dir);
        copy(&dir, &by_ocelli_dir);
        let (git_ok, git_said) = run(&by_git_dir.join("repo"), args);
        let ocelli = GitWriter
            .write(&by_ocelli_dir.join("repo"), &make_write(&shown))
            .map(|_| ())
            .map_err(|e| e.to_string());
        let outcome = Outcome {
            before: before.clone(),
            git_ok,
            git_said,
            by_git: snapshot(&by_git_dir.join("repo")),
            ocelli,
            by_ocelli: snapshot(&by_ocelli_dir.join("repo")),
        };
        let _ = std::fs::remove_dir_all(&by_git_dir);
        let _ = std::fs::remove_dir_all(&by_ocelli_dir);
        let verdict = expected(state, write);
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
    assert!(
        failures.is_empty(),
        "{} cases differ from what the tables expect:\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}
