//! Verifies: ADR-0033, G-7
//!
//! The writes of the repository port on repositories made with git: each does what it says, and
//! one whose expected state the repository does not hold is refused with nothing changed. Writing
//! runs none of the programs the repository names (Q-4).

#![allow(
    clippy::disallowed_methods,
    reason = "a test sets a repository up by hand, the way a developer would"
)]
#![cfg(unix)]

mod git_command;
use git_command::git_command;
use ocelli_core::port::{RepositoryReader, RepositoryWriter, Write};
use ocelli_git::GitWriter;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

mod programs;

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("ocelli-write-{name}-{}", std::process::id()));
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

fn git(dir: &Path, args: &[&str]) -> String {
    raw(dir, args).trim().to_string()
}

/// What `git` prints, untrimmed, for output whose leading space or NUL separators are read.
fn raw(dir: &Path, args: &[&str]) -> String {
    let out = git_command(dir)
        .args(["-c", "user.name=F", "-c", "user.email=f@example.invalid"])
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

/// The files `git` keeps in its directory while an operation is in progress, and `ORIG_HEAD`,
/// which the operations set.
const IN_PROGRESS: &[&str] = &[
    "MERGE_HEAD",
    "MERGE_MSG",
    "MERGE_MODE",
    "MERGE_AUTOSTASH",
    "AUTO_MERGE",
    "CHERRY_PICK_HEAD",
    "REVERT_HEAD",
    "ORIG_HEAD",
    "BISECT_LOG",
    "rebase-merge",
    "rebase-apply",
    "sequencer",
];

/// The repository as `git` reads it, everything a write can change: a refused write leaves each
/// part as it was (`docs/CONVENTIONS.md`, a refusal changes nothing).
#[derive(Debug, PartialEq)]
struct Snapshot {
    head: String,
    /// Every ref, the tags, the remote branches, and the stash included.
    refs: String,
    /// The stash's entries, those below its newest included, which no ref names.
    stash: String,
    index: String,
    /// Each path the status names, ignored and untracked files included, with its content.
    status: Vec<(String, Option<String>)>,
    /// The repository's own configuration, where a branch's upstream is kept.
    config: String,
    /// Every directory of the working directory, an empty one included, which the status omits.
    dirs: Vec<String>,
    in_progress: Vec<(String, String)>,
}

fn snapshot(dir: &Path) -> Snapshot {
    let status = raw(
        dir,
        &[
            "status",
            "--porcelain",
            "-z",
            "--ignored",
            "--untracked-files=all",
        ],
    );
    let mut fields = status.split('\0').filter(|f| !f.is_empty());
    let mut entries = Vec::new();
    while let Some(entry) = fields.next() {
        let path = &entry[3..];
        // A rename or a copy names its source in the field after.
        let entry = match entry.as_bytes()[0] {
            b'R' | b'C' => format!("{entry} from {}", fields.next().unwrap()),
            _ => entry.to_string(),
        };
        entries.push((entry, content_of(&dir.join(path))));
    }
    let git_dir = PathBuf::from(git(dir, &["rev-parse", "--absolute-git-dir"]));
    let mut in_progress = Vec::new();
    for name in IN_PROGRESS {
        files_below(&git_dir, Path::new(name), &mut in_progress);
    }
    Snapshot {
        head: head_of(dir),
        refs: git(dir, &["for-each-ref", "--format=%(refname) %(objectname)"]),
        stash: git(dir, &["stash", "list", "--format=%gd %H %gs"]),
        index: git(dir, &["ls-files", "--stage"]),
        status: entries,
        config: git(dir, &["config", "--local", "--list"]),
        dirs: dirs_below(dir, Path::new("")),
        in_progress,
    }
}

/// The content of the file at `path`, by blob id, a link's target, or `None` for a directory or
/// nothing.
fn content_of(path: &Path) -> Option<String> {
    let meta = path.symlink_metadata().ok()?;
    if meta.is_symlink() {
        return Some(format!(
            "-> {}",
            std::fs::read_link(path).unwrap().display()
        ));
    }
    meta.is_file().then(|| {
        git2::Oid::hash_file(git2::ObjectType::Blob, path)
            .unwrap()
            .to_string()
    })
}

/// Every file at or below `path` in `root`, with its content.
/// The directories below `path` of the working directory `root`, `.git` left out.
fn dirs_below(root: &Path, path: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut names: Vec<_> = std::fs::read_dir(root.join(path))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.symlink_metadata().is_ok_and(|m| m.is_dir()))
        .filter(|p| p.file_name().is_some_and(|n| n != ".git"))
        .collect();
    names.sort();
    for dir in names {
        let name = dir.strip_prefix(root).unwrap().to_path_buf();
        found.push(name.display().to_string());
        found.extend(dirs_below(root, &name));
    }
    found
}

fn files_below(root: &Path, path: &Path, found: &mut Vec<(String, String)>) {
    let full = root.join(path);
    if full.is_dir() {
        let mut names: Vec<_> = std::fs::read_dir(&full)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        names.sort();
        for name in names {
            files_below(root, &path.join(name), found);
        }
    } else if let Ok(content) = std::fs::read(&full) {
        let content = String::from_utf8_lossy(&content).into_owned();
        found.push((path.display().to_string(), content));
    }
}

/// Writes `write` and expects it refused with `why` among its words, and the repository's
/// [`Snapshot`] as it was before. Every refusal test of the writer goes through here, so that
/// none compares less than the whole snapshot.
fn refused(dir: &Path, write: &Write, why: &str) {
    refused_reporting(dir, write, why, true, |_| {});
}

/// [`refused`], with the write's progress answered with `go_on`, and `meanwhile` run on the
/// repository at the first report, as another program would change it while the write waits:
/// the snapshot expected after is the one taken once `meanwhile` ran.
fn refused_reporting(
    dir: &Path,
    write: &Write,
    why: &str,
    go_on: bool,
    meanwhile: impl FnOnce(&Path),
) {
    let mut before = snapshot(dir);
    let mut meanwhile = Some(meanwhile);
    let mut reports = 0;
    let error = GitWriter
        .write_reporting(dir, write, &mut |_| {
            reports += 1;
            if let Some(change) = meanwhile.take() {
                change(dir);
                before = snapshot(dir);
            }
            go_on
        })
        .unwrap_err();
    assert!(error.to_string().contains(why), "{write:?}: {error}");
    assert!(
        go_on || reports > 0,
        "{write:?}: the write was asked to stop, so it reported"
    );
    assert_eq!(snapshot(dir), before, "{write:?}");
}

/// The lines of the Rust source `text` that take a refusal of a write outside
/// [`refused_reporting`], by number, so that no test of the writer compares less than the
/// snapshot.
fn refusals_outside_the_helper(text: &str) -> Vec<usize> {
    let mut inside = false;
    let mut found = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if line.starts_with("fn refused_reporting(") {
            inside = true;
        } else if inside && line == "}" {
            inside = false;
        } else if !inside {
            let takes_an_error = [
                concat!(".unwrap", "_err()"),
                concat!(".expect", "_err("),
                concat!(".is", "_err()"),
                concat!(".err", "()"),
                concat!("let ", "Err("),
            ]
            .iter()
            .any(|p| line.contains(p));
            // A refusal of a write on this line, the one before, or the one after.
            let writes = |l: &str| l.contains(".write(") || l.contains("write_reporting(");
            let near = |j: Option<usize>| j.and_then(|j| text.lines().nth(j)).is_some_and(writes);
            let of_a_write = writes(line) || near(i.checked_sub(1)) || near(Some(i + 1));
            let unconditional = line.contains(concat!(".unwrap", "_err()"))
                || line.contains(concat!(".expect", "_err("));
            if takes_an_error && (unconditional || of_a_write) {
                found.push(i + 1);
            }
        }
    }
    found
}

#[test]
fn every_refusal_test_of_the_writer_compares_the_snapshot() {
    let text = include_str!("write.rs");
    assert!(
        text.contains("fn refused_reporting("),
        "the helper is where the check looks for it"
    );
    let found = refusals_outside_the_helper(text);
    assert!(
        found.is_empty(),
        "crates/git/tests/write.rs takes a refusal outside refused() on lines {found:?}"
    );
}

#[test]
fn the_check_names_each_line_that_takes_a_refusal_outside_the_helper() {
    let unwrap = concat!(".unwrap", "_err()");
    let expect = concat!(".expect", "_err(\"refused\")");
    let text = format!(
        "fn refused_reporting(d: &Path) {{\n    let e = w(){unwrap};\n    if x {{\n    }}\n}}\n\
         fn a() {{\n    let e = GitWriter.write(d, &w){unwrap};\n}}\n\
         let f = w(){expect};\n"
    );
    assert_eq!(refusals_outside_the_helper(&text), vec![7, 9]);
    assert_eq!(
        refusals_outside_the_helper("fn a() {\n    refused(d, &w, \"why\");\n}\n"),
        Vec::<usize>::new()
    );
    let is_err = concat!(".is", "_err()");
    let let_err = concat!("let ", "Err(e)");
    let text = format!(
        "assert!(GitWriter.write(d, &w){is_err});\n\
         {let_err} = GitWriter\n    .write(d, &w) else {{ return }};\n\
         let r = GitWriter.write_reporting(d, &w, &mut go);\nif {let_err} = r {{}}\n\
         assert!(std::fs::read(p){is_err});\n"
    );
    assert_eq!(refusals_outside_the_helper(&text), vec![1, 2, 5]);
}

#[test]
// Verifies: ADR-0033
fn the_snapshot_sees_every_part_a_write_can_change() {
    let scratch = with_file("snapshot");
    let d = &scratch.0;
    write_file(d, ".git/info/exclude", "ignored\n");
    write_file(d, "ignored", "one\n");
    for content in ["first\n", "second\n"] {
        write_file(d, "f", content);
        git(d, &["stash", "push", "--quiet"]);
    }
    let sees = |part: &str, change: &dyn Fn()| {
        let before = snapshot(d);
        change();
        assert_ne!(snapshot(d), before, "{part}");
    };
    sees("HEAD", &|| {
        git(d, &["checkout", "--quiet", "--detach"]);
    });
    sees("a tag", &|| {
        git(d, &["tag", "t"]);
    });
    sees("an older stash", &|| {
        git(d, &["stash", "drop", "--quiet", "stash@{1}"]);
    });
    sees("the index", &|| {
        write_file(d, "f", "staged\n");
        git(d, &["add", "f"]);
    });
    sees("a file's content", &|| write_file(d, "f", "other\n"));
    sees("an ignored file", &|| write_file(d, "ignored", "two\n"));
    sees("an upstream", &|| {
        git(d, &["config", "branch.main.remote", "origin"]);
    });
    sees("a merge in progress", &|| {
        let head = git(d, &["rev-parse", "HEAD"]);
        write_file(d, ".git/MERGE_HEAD", &format!("{head}\n"));
    });
    sees("a sequencer", &|| {
        write_file(d, ".git/sequencer/todo", "pick x\n");
    });
}

/// A repository with two commits on main, its identity in its own configuration, and the SHAs.
fn repository(dir: &Path) -> (String, String) {
    git(dir, &["init", "--quiet", "-b", "main"]);
    git(dir, &["config", "user.name", "Ada"]);
    git(dir, &["config", "user.email", "ada@example.invalid"]);
    git(dir, &["commit", "--quiet", "--allow-empty", "-m", "one"]);
    let one = git(dir, &["rev-parse", "HEAD"]);
    git(dir, &["commit", "--quiet", "--allow-empty", "-m", "two"]);
    (one, git(dir, &["rev-parse", "HEAD"]))
}

fn tag(name: &str, commit: &str, message: Option<&str>) -> Write {
    Write::CreateTag {
        name: name.to_string(),
        commit: commit.to_string(),
        message: message.map(str::to_string),
    }
}

fn tags(dir: &Path) -> String {
    git(
        dir,
        &[
            "tag",
            "--list",
            "--format=%(refname:short) %(objecttype) %(*objectname)%(objectname)",
        ],
    )
}

#[test]
fn a_tag_with_a_message_is_annotated_by_the_configured_identity() {
    let scratch = Scratch::new("annotated");
    let (one, _) = repository(&scratch.0);
    GitWriter
        .write(&scratch.0, &tag("v1", &one, Some("The first release")))
        .unwrap();
    assert_eq!(git(&scratch.0, &["rev-parse", "v1^{commit}"]), one);
    assert_eq!(git(&scratch.0, &["cat-file", "-t", "v1"]), "tag");
    let object = git(&scratch.0, &["cat-file", "tag", "v1"]);
    assert!(object.ends_with("\n\nThe first release"), "{object:?}");
    assert_eq!(
        git(
            &scratch.0,
            &[
                "tag",
                "--list",
                "--format=%(taggername) %(contents:subject)",
                "v1"
            ]
        ),
        "Ada The first release"
    );
}

#[test]
fn a_tag_without_a_message_is_lightweight() {
    let scratch = Scratch::new("lightweight");
    let (one, _) = repository(&scratch.0);
    GitWriter
        .write(&scratch.0, &tag("mark", &one, Some("  ")))
        .unwrap();
    assert_eq!(git(&scratch.0, &["cat-file", "-t", "mark"]), "commit");
    assert_eq!(git(&scratch.0, &["rev-parse", "mark"]), one);
}

#[test]
fn a_tag_whose_name_is_taken_is_refused_with_nothing_changed() {
    let scratch = Scratch::new("taken");
    let (one, two) = repository(&scratch.0);
    git(&scratch.0, &["tag", "v1", &one]);
    refused(&scratch.0, &tag("v1", &two, None), "exists already");
}

#[test]
fn a_tag_on_a_commit_no_longer_there_or_with_a_bad_name_is_refused() {
    let scratch = Scratch::new("refused");
    let (one, _) = repository(&scratch.0);
    let gone = "0123456789abcdef0123456789abcdef01234567";
    refused(
        &scratch.0,
        &tag("v1", gone, None),
        "no longer in the repository",
    );
    for bad in ["a..b", "-v1"] {
        refused(&scratch.0, &tag(bad, &one, None), "not a name");
    }
}

#[test]
// Verifies: ADR-0033, ADR-0035, Q-4
fn writing_a_repository_runs_none_of_its_programs() {
    use ocelli_core::model::Operation;
    use ocelli_core::port::{ResetMode, Take};
    let scratch = with_file("programs");
    let d = &scratch.0;
    let one = git(d, &["rev-parse", "HEAD"]);
    let remote = Scratch::new("programs-remote");
    std::fs::remove_dir_all(&remote.0).unwrap();
    git(
        Path::new("/"),
        &[
            "clone",
            "--quiet",
            "--bare",
            d.to_str().unwrap(),
            remote.0.to_str().unwrap(),
        ],
    );
    git(d, &["remote", "add", "origin", remote.0.to_str().unwrap()]);
    // Outside the repository, so that stashing the untracked files leaves them be.
    let tools = Scratch::new("programs-ran");
    let markers = programs::set_everywhere(d, &tools.0);
    // Ocelli refuses what these would have git sign (ADR-0033); the writes here go through.
    let mut config = git2::Repository::open(d).unwrap().config().unwrap();
    for (key, _) in programs::SWITCHES {
        config.remove(key).unwrap();
    }
    // Read with git2, which runs none of the programs, where git would run fsmonitor.
    let rev = |r: &str| {
        let repository = git2::Repository::open(d).unwrap();
        repository.revparse_single(r).unwrap().id().to_string()
    };
    let mut performed = BTreeSet::new();
    let mut write = |write: Write| {
        performed.insert(kind_of(&write));
        GitWriter.write(d, &write).unwrap()
    };

    write(tag("v1", &one, Some("release")));
    write(tag("v2", &one, None));
    write(delete("v2", &one, false));
    std::fs::write(d.join("f"), "more\n").unwrap();
    std::fs::write(d.join("new"), "untracked\n").unwrap();
    write(create_stash(None, true));
    write(apply(0, &rev("refs/stash"), true));
    write(stage_in(d, "f"));
    write(unstage_in(d, "f"));
    write(stage_in(d, "f"));
    let made = write(commit_on(&one, "commit")).unwrap();
    write(create_stash(None, true));
    write(Write::DropStash {
        index: 0,
        commit: rev("refs/stash"),
    });
    // A switch is where git runs post-checkout.
    let switch = |name: &str, tip: &str| Write::SwitchBranch {
        name: name.into(),
        tip: tip.into(),
    };
    let branch = |name: &str, commit: &str| Write::CreateBranch {
        name: name.into(),
        commit: commit.into(),
        switch: true,
    };
    let delete_branch = |name: &str, tip: &str| Write::DeleteBranch {
        name: name.into(),
        tip: tip.into(),
        merged: true,
    };
    write(branch("b", &one));
    write(switch("main", &made));
    write(Write::Detach {
        commit: one.clone(),
    });
    write(delete_branch("b", &one));
    write(Write::Fetch {
        remote: "origin".into(),
    });
    write(switch("main", &made));
    // A push is where git runs pre-push.
    let push = |branch: &str| Write::Push {
        branch: branch.into(),
        tip: made.clone(),
    };
    write(push("main"));
    // The push set main's upstream; origin moves on, so a pull fetches it and fast-forwards
    // main, a checkout, where git runs post-checkout and post-merge.
    let ahead = git(
        &remote.0,
        &[
            "commit-tree",
            &format!("{made}^{{tree}}"),
            "-p",
            &made,
            "-m",
            "ahead",
        ],
    );
    git(&remote.0, &["update-ref", "refs/heads/main", &ahead]);
    write(Write::Pull {
        branch: "main".into(),
        tip: made.clone(),
    });
    assert_eq!(rev("HEAD"), ahead, "the pull fast-forwarded main");
    // Back where the writes below expect main, by the test's own git, which runs nothing.
    git(d, &["reset", "--quiet", "--hard", &made]);
    // A branch pushed and deleted here is one of origin's to track.
    write(branch("t", &made));
    write(push("t"));
    write(switch("main", &made));
    write(delete_branch("t", &made));
    write(Write::TrackRemote {
        remote: "origin".into(),
        branch: "t".into(),
        tip: made.clone(),
    });
    write(switch("main", &made));

    // Two sides that conflict in f, for the operations on the history to stop on.
    write(branch("side", &made));
    std::fs::write(d.join("f"), "theirs\n").unwrap();
    write(stage_in(d, "f"));
    let theirs = write(commit_on(&made, "theirs")).unwrap();
    write(switch("main", &made));
    std::fs::write(d.join("f"), "ours\n").unwrap();
    write(stage_in(d, "f"));
    let ours = write(commit_on(&made, "ours")).unwrap();
    // A merge that stops, resolved and continued: pre-merge-commit, post-merge, commit-msg.
    let stopped = write(Write::Merge {
        branch: "main".into(),
        tip: ours.clone(),
        commit: theirs.clone(),
        name: Some("side".into()),
    });
    assert_eq!(stopped, None, "the merge stopped on f");
    let conflict = ocelli_git::GitReader
        .load_refs(d)
        .unwrap()
        .in_progress
        .unwrap()
        .conflicts[0]
        .clone();
    write(Write::Resolve {
        path: "f".into(),
        conflict,
        take: Take::Ours,
    });
    let merged = write(Write::Continue {
        operation: Operation::Merge,
        head: ours.clone(),
    })
    .unwrap();
    write(Write::Reset {
        branch: "main".into(),
        tip: merged,
        commit: ours.clone(),
        mode: ResetMode::Hard,
    });
    // A rebase and a cherry-pick that stop, and are aborted: pre-rebase, post-rewrite.
    let stopped = write(Write::Rebase {
        branch: "main".into(),
        tip: ours.clone(),
        onto: theirs.clone(),
    });
    assert_eq!(stopped, None, "the rebase stopped on f");
    write(Write::Abort {
        operation: Operation::Rebase,
        head: rev("HEAD"),
    });
    let stopped = write(Write::CherryPick {
        branch: "main".into(),
        tip: ours.clone(),
        commit: theirs,
    });
    assert_eq!(stopped, None, "the cherry-pick stopped on f");
    write(Write::Abort {
        operation: Operation::CherryPick,
        head: rev("HEAD"),
    });
    write(Write::Revert {
        branch: "main".into(),
        tip: ours.clone(),
        commit: ours,
    })
    .unwrap();

    let ran = markers.ran();
    assert!(ran.is_empty(), "programs ran: {ran:?}");
    let missing = not_performed(&performed, &kinds_of_write());
    assert!(
        missing.is_empty(),
        "the test of Q-4 for writing performs no {missing:?}"
    );
}

/// The kind of `write`, as the core names it in JSON.
fn kind_of(write: &Write) -> String {
    serde_json::to_value(write).unwrap()["kind"]
        .as_str()
        .unwrap()
        .to_string()
}

/// Every kind of `Write` the core takes, as serde lists them in its refusal of a kind it does not
/// know: the one list a test can read without repeating it, and longer with each new write.
fn kinds_of_write() -> Vec<String> {
    let Err(refusal) = serde_json::from_str::<Write>(r#"{"kind":"?"}"#) else {
        panic!("a kind the core does not know is refused");
    };
    let refusal = refusal.to_string();
    let (_, expected) = refusal
        .split_once("expected one of ")
        .unwrap_or_else(|| panic!("the refusal lists the kinds: {refusal}"));
    expected
        .split(", ")
        .map(|kind| kind.split('`').nth(1).unwrap().to_string())
        .collect()
}

/// The `kinds` of write not among those `performed`.
fn not_performed(performed: &BTreeSet<String>, kinds: &[String]) -> Vec<String> {
    kinds
        .iter()
        .filter(|kind| !performed.contains(*kind))
        .cloned()
        .collect()
}

#[test]
fn the_check_names_a_write_the_test_of_q4_does_not_perform() {
    let kinds = kinds_of_write();
    for kind in [
        "createTag",
        "pull",
        "dropStash",
        "trackRemote",
        "revert",
        "abort",
    ] {
        assert!(kinds.iter().any(|k| k == kind), "{kind} in {kinds:?}");
    }
    let mut performed: BTreeSet<String> = kinds.iter().cloned().collect();
    assert_eq!(not_performed(&performed, &kinds), Vec::<String>::new());
    performed.remove("dropStash");
    assert_eq!(not_performed(&performed, &kinds), ["dropStash"]);
}

fn delete(name: &str, commit: &str, annotated: bool) -> Write {
    Write::DeleteTag {
        name: name.to_string(),
        commit: commit.to_string(),
        annotated,
    }
}

#[test]
fn a_tag_still_as_shown_is_deleted() {
    let scratch = Scratch::new("delete");
    let (one, two) = repository(&scratch.0);
    git(&scratch.0, &["tag", "-a", "-m", "release", "v1", &one]);
    git(&scratch.0, &["tag", "mark", &two]);
    GitWriter
        .write(&scratch.0, &delete("v1", &one, true))
        .unwrap();
    GitWriter
        .write(&scratch.0, &delete("mark", &two, false))
        .unwrap();
    assert_eq!(tags(&scratch.0), "");
}

#[test]
fn a_tag_no_longer_as_shown_is_refused_with_nothing_changed() {
    let scratch = Scratch::new("moved");
    let (one, two) = repository(&scratch.0);
    git(&scratch.0, &["tag", "v1", &two]);
    for (write, why) in [
        (delete("v1", &one, false), "no longer the one shown"),
        (delete("v1", &two, true), "no longer the one shown"),
        (delete("gone", &two, false), "no tag named 'gone'"),
    ] {
        refused(&scratch.0, &write, why);
    }
}

/// The stash's entries, newest first, as `stash@{n} message`.
fn stashes(dir: &Path) -> String {
    git(dir, &["stash", "list", "--format=%gd %gs"])
}

fn status(dir: &Path) -> String {
    git(dir, &["status", "--porcelain", "--untracked-files=all"])
}

fn stash_at(dir: &Path, index: usize) -> String {
    git(dir, &["rev-parse", &format!("stash@{{{index}}}")])
}

/// A repository whose tracked `f` holds `base`, committed.
fn with_file(name: &str) -> Scratch {
    let scratch = Scratch::new(name);
    repository(&scratch.0);
    std::fs::write(scratch.0.join("f"), "base\n").unwrap();
    git(&scratch.0, &["add", "f"]);
    git(&scratch.0, &["commit", "--quiet", "-m", "f"]);
    scratch
}

fn create_stash(message: Option<&str>, untracked: bool) -> Write {
    Write::CreateStash {
        message: message.map(str::to_string),
        untracked,
    }
}

fn apply(index: usize, commit: &str, drop: bool) -> Write {
    Write::ApplyStash {
        index,
        commit: commit.to_string(),
        drop,
    }
}

#[test]
// Verifies: ADR-0033, G-7
fn stashing_puts_the_changes_aside_and_the_untracked_files_only_when_asked() {
    let scratch = with_file("stash");
    std::fs::write(scratch.0.join("f"), "changed\n").unwrap();
    std::fs::write(scratch.0.join("new"), "untracked\n").unwrap();
    GitWriter
        .write(&scratch.0, &create_stash(Some("half done"), false))
        .unwrap();
    assert!(
        stashes(&scratch.0).ends_with("half done"),
        "{}",
        stashes(&scratch.0)
    );
    assert_eq!(status(&scratch.0), "?? new", "the untracked file stays");

    std::fs::write(scratch.0.join("f"), "again\n").unwrap();
    GitWriter
        .write(&scratch.0, &create_stash(None, true))
        .unwrap();
    assert_eq!(status(&scratch.0), "", "both went into the stash");
    assert_eq!(stashes(&scratch.0).lines().count(), 2);
}

#[test]
fn a_clean_working_directory_has_nothing_to_stash() {
    let scratch = with_file("clean");
    refused(&scratch.0, &create_stash(None, false), "nothing to stash");
}

#[test]
// Verifies: ADR-0033, G-7
fn a_stash_applies_and_stays_or_is_dropped_after() {
    let scratch = with_file("apply");
    std::fs::write(scratch.0.join("f"), "changed\n").unwrap();
    std::fs::write(scratch.0.join("new"), "untracked\n").unwrap();
    git(
        &scratch.0,
        &["stash", "push", "--quiet", "--include-untracked"],
    );
    let stash = stash_at(&scratch.0, 0);

    GitWriter
        .write(&scratch.0, &apply(0, &stash, false))
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(scratch.0.join("f")).unwrap(),
        "changed\n"
    );
    assert_eq!(
        std::fs::read_to_string(scratch.0.join("new")).unwrap(),
        "untracked\n"
    );
    assert_eq!(stashes(&scratch.0).lines().count(), 1, "applying keeps it");

    git(&scratch.0, &["checkout", "--quiet", "--", "f"]);
    std::fs::remove_file(scratch.0.join("new")).unwrap();
    GitWriter
        .write(&scratch.0, &apply(0, &stash, true))
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(scratch.0.join("f")).unwrap(),
        "changed\n"
    );
    assert_eq!(stashes(&scratch.0), "", "applied and dropped");
}

#[test]
// Verifies: ADR-0033, G-7
fn a_stash_that_would_conflict_is_refused_with_the_working_directory_unchanged() {
    let scratch = with_file("conflict");
    std::fs::write(scratch.0.join("f"), "stashed\n").unwrap();
    std::fs::write(scratch.0.join("new"), "untracked\n").unwrap();
    git(
        &scratch.0,
        &["stash", "push", "--quiet", "--include-untracked"],
    );
    let stash = stash_at(&scratch.0, 0);
    std::fs::write(scratch.0.join("f"), "committed\n").unwrap();
    git(&scratch.0, &["commit", "--quiet", "-am", "f again"]);

    refused(&scratch.0, &apply(0, &stash, true), "conflicts");
}

#[test]
fn a_stash_over_changes_of_the_working_directory_is_refused() {
    let scratch = with_file("overwrite");
    std::fs::write(scratch.0.join("f"), "stashed\n").unwrap();
    git(&scratch.0, &["stash", "push", "--quiet"]);
    let stash = stash_at(&scratch.0, 0);
    std::fs::write(scratch.0.join("f"), "mine\n").unwrap();

    refused(
        &scratch.0,
        &apply(0, &stash, false),
        "f has changes of its own",
    );
}

#[test]
// Verifies: ADR-0033
fn a_stash_no_longer_at_its_place_is_neither_applied_nor_dropped() {
    let scratch = with_file("moved-stash");
    std::fs::write(scratch.0.join("f"), "first\n").unwrap();
    git(&scratch.0, &["stash", "push", "--quiet"]);
    let seen = stash_at(&scratch.0, 0);
    std::fs::write(scratch.0.join("f"), "second\n").unwrap();
    git(&scratch.0, &["stash", "push", "--quiet"]);

    for write in [
        apply(0, &seen, false),
        Write::DropStash {
            index: 0,
            commit: seen.clone(),
        },
    ] {
        refused(&scratch.0, &write, "no longer the stash shown");
    }

    GitWriter
        .write(
            &scratch.0,
            &Write::DropStash {
                index: 1,
                commit: seen,
            },
        )
        .unwrap();
    assert_eq!(
        stashes(&scratch.0).lines().count(),
        1,
        "where it is now, it drops"
    );
}

/// Stashes everything with the untracked files, then sets the working directory up with `then`,
/// and expects applying and popping the stash to be [`refused`] with `why`.
fn a_stash_refused_over(name: &str, set_up: impl Fn(&Path), then: impl Fn(&Path), why: &str) {
    let scratch = with_file(name);
    set_up(&scratch.0);
    git(
        &scratch.0,
        &["stash", "push", "--quiet", "--include-untracked"],
    );
    let stash = stash_at(&scratch.0, 0);
    then(&scratch.0);
    for drop in [false, true] {
        refused(&scratch.0, &apply(0, &stash, drop), why);
    }
}

fn write_file(dir: &Path, path: &str, content: &str) {
    let path = dir.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

#[test]
// Verifies: ADR-0033, G-7
fn a_stash_with_anything_in_its_way_is_refused_with_nothing_changed() {
    // The stash's new file n/x, and a file n where it needs a directory.
    a_stash_refused_over(
        "file-for-dir",
        |d| {
            write_file(d, "u", "untracked\n");
            write_file(d, "n/x", "x\n");
            git(d, &["add", "n/x"]);
        },
        |d| write_file(d, "n", "blocker\n"),
        "n stands where stash@{0} needs a directory",
    );
    // The stash's new file d, and an untracked directory d.
    a_stash_refused_over(
        "dir-for-file",
        |d| {
            write_file(d, "u", "untracked\n");
            write_file(d, "d", "tracked\n");
            git(d, &["add", "d"]);
        },
        |d| write_file(d, "d/y", "y\n"),
        "d is a directory where stash@{0} writes a file",
    );
    // An untracked file the stash took along, since ignored and written anew.
    a_stash_refused_over(
        "ignored-file",
        |d| write_file(d, ".env", "stashed\n"),
        |d| {
            write_file(d, ".git/info/exclude", ".env\n");
            write_file(d, ".env", "PRECIOUS\n");
        },
        ".env is not tracked",
    );
    // An untracked file the stash took along, and an ignored directory of that name.
    a_stash_refused_over(
        "ignored-dir",
        |d| write_file(d, "build", "stashed\n"),
        |d| {
            write_file(d, ".git/info/exclude", "build/\n");
            write_file(d, "build/out.o", "object\n");
        },
        "build is a directory",
    );
    // An untracked file the stash took along, tracked by now.
    a_stash_refused_over(
        "tracked-by-now",
        |d| write_file(d, "later", "stashed\n"),
        |d| {
            write_file(d, "later", "committed\n");
            git(d, &["add", "later"]);
            git(d, &["commit", "--quiet", "-m", "later"]);
        },
        "later is tracked by now",
    );
    // An untracked file the stash took along, and one whose name differs only in case, where
    // the file system folds case and the two are one file.
    let probe = Scratch::new("case-probe");
    std::fs::write(probe.0.join("Case"), "").unwrap();
    if probe.0.join("CASE").exists() {
        a_stash_refused_over(
            "case",
            |d| write_file(d, "Readme", "stashed\n"),
            |d| write_file(d, "README", "mine\n"),
            "Readme is not tracked",
        );
    } else {
        eprintln!("this file system tells Readme from README; the case-folding case is skipped");
    }
    // Changes staged in the index.
    a_stash_refused_over(
        "staged",
        |d| write_file(d, "f", "stashed\n"),
        |d| {
            write_file(d, "g", "staged\n");
            git(d, &["add", "g"]);
        },
        "the index holds staged changes",
    );
}

/// Whether `git` takes `name` as a tag on `commit`; the tag it made is deleted again.
fn git_takes(dir: &Path, name: &str, commit: &str) -> bool {
    let made = git_command(dir)
        .args(["tag", "--", name, commit])
        .output()
        .unwrap()
        .status
        .success();
    if made {
        git(dir, &["tag", "-d", "--", name]);
    }
    made
}

/// The message of the annotated tag `name`, as `git` reads it back.
fn tag_message(dir: &Path, name: &str) -> String {
    let out = git_command(dir)
        .args(["cat-file", "tag", &format!("refs/tags/{name}")])
        .output()
        .unwrap();
    let object = String::from_utf8(out.stdout).unwrap();
    object.split_once("\n\n").map_or("", |(_, m)| m).to_string()
}

#[test]
// Verifies: ADR-0033, G-7
fn the_writer_takes_the_tag_names_git_takes() {
    let scratch = Scratch::new("names");
    let (one, _) = repository(&scratch.0);
    let names = [
        "v1",
        "v1.0",
        "release/1",
        "ünï",
        "-v1",
        "v1-",
        ".v1",
        "v1.",
        "v..1",
        "v1.lock",
        "a/.b",
        "a//b",
        "/a",
        "a/",
        "v@{1",
        "@",
        "HEAD",
        "v 1",
        "v~1",
        "v^1",
        "v:1",
        "v?1",
        "v*1",
        "v[1",
        "v\\1",
        "v\x07",
        "v\x7f",
        &one,
    ];
    let mut differ = Vec::new();
    for name in names {
        let git_ok = git_takes(&scratch.0, name, &one);
        let ours = GitWriter.write(&scratch.0, &tag(name, &one, None));
        if ours.is_ok() {
            git(&scratch.0, &["tag", "-d", "--", name]);
        }
        if git_ok != ours.is_ok() {
            differ.push((name.to_string(), git_ok));
        }
    }
    assert!(
        differ.is_empty(),
        "taken by git, not by the writer, or back: {differ:?}"
    );
}

#[test]
// Verifies: ADR-0033, G-7
fn the_writer_cleans_a_tag_message_as_git_does() {
    let scratch = Scratch::new("messages");
    let (one, _) = repository(&scratch.0);
    let messages = [
        "plain",
        "\n\nblank lines around\n\n\n",
        "inside\n\n\n\nblank lines",
        "trailing spaces   \nand tabs\t\t",
        "# a comment\nkept?",
        "subject\n# comment inside\nbody",
        "crlf\r\nlines\r\n",
        "   \n\t\n",
    ];
    let mut lightweight = 0;
    for (i, message) in messages.iter().enumerate() {
        let theirs = format!("git-{i}");
        let ours = format!("ours-{i}");
        git(&scratch.0, &["tag", "-a", "-m", message, &theirs, &one]);
        GitWriter
            .write(&scratch.0, &tag(&ours, &one, Some(message)))
            .unwrap();
        let expected = tag_message(&scratch.0, &theirs);
        if expected.trim().is_empty() {
            // G-7 decides that a tag without a message is lightweight, where git writes an
            // annotated tag with an empty one.
            assert_eq!(
                git(&scratch.0, &["cat-file", "-t", &ours]),
                "commit",
                "{message:?}"
            );
            lightweight += 1;
            continue;
        }
        assert_eq!(tag_message(&scratch.0, &ours), expected, "{message:?}");
    }
    assert_eq!(
        lightweight, 1,
        "only the message of whitespace alone makes no annotated tag"
    );

    // The repository's own comment string, a character of more than one byte among them.
    for (key, value) in [("core.commentChar", "§"), ("core.commentString", "//")] {
        git(&scratch.0, &["config", key, value]);
        let message = format!("{value} gone\nkept\n# stays");
        let (theirs, ours) = (format!("git-{key}"), format!("ours-{key}"));
        git(&scratch.0, &["tag", "-a", "-m", &message, &theirs, &one]);
        GitWriter
            .write(&scratch.0, &tag(&ours, &one, Some(&message)))
            .unwrap();
        let expected = tag_message(&scratch.0, &theirs);
        assert_eq!(expected, "kept\n# stays\n", "{key}");
        assert_eq!(tag_message(&scratch.0, &ours), expected, "{key}");
        git(&scratch.0, &["config", "--unset", key]);
    }
}

#[test]
// Verifies: ADR-0024, ADR-0033
fn each_stash_node_carries_its_place_in_the_stash() {
    use ocelli_core::model::CommitKind;
    let scratch = with_file("places");
    for content in ["first\n", "second\n"] {
        std::fs::write(scratch.0.join("f"), content).unwrap();
        git(&scratch.0, &["stash", "push", "--quiet"]);
    }
    let commits = ocelli_git::GitReader
        .load_first_chunk(&scratch.0, &mut |_| {})
        .unwrap();
    let mut places: Vec<_> = commits
        .iter()
        .filter(|c| c.kind == CommitKind::Stash)
        .map(|c| (c.sha.clone(), c.stash))
        .collect();
    places.sort_by_key(|(_, place)| *place);
    let expected = [
        (stash_at(&scratch.0, 0), Some(0)),
        (stash_at(&scratch.0, 1), Some(1)),
    ];
    assert_eq!(places, expected);
}

/// The content of `path` in the working directory and in the index, read with git2, which runs
/// none of the programs the repository names, as the git binary would.
fn shown_in(dir: &Path, path: &str) -> (Option<String>, Option<String>) {
    let workdir = dir.join(path).symlink_metadata().is_ok().then(|| {
        git2::Oid::hash_file(git2::ObjectType::Blob, dir.join(path))
            .unwrap()
            .to_string()
    });
    let index = git2::Repository::open(dir)
        .unwrap()
        .index()
        .unwrap()
        .get_path(Path::new(path), 0)
        .map(|e| e.id.to_string());
    (workdir, index)
}

/// Staging `path` as shown now.
fn stage_in(dir: &Path, path: &str) -> Write {
    Write::StageFile {
        path: path.to_string(),
        shown: shown_in(dir, path).0,
    }
}

/// Unstaging `path` as shown now.
fn unstage_in(dir: &Path, path: &str) -> Write {
    Write::UnstageFile {
        path: path.to_string(),
        shown: shown_in(dir, path).1,
    }
}

fn commit_on(head: &str, message: &str) -> Write {
    Write::Commit {
        message: message.to_string(),
        head: Some(head.to_string()),
    }
}

fn staged_names(dir: &Path) -> String {
    git(dir, &["diff", "--cached", "--name-status"])
}

#[test]
// Verifies: ADR-0033, G-4
fn whole_files_are_staged_and_unstaged_as_git_add_and_restore_do() {
    let scratch = with_file("stage");
    write_file(&scratch.0, "f", "changed\n");
    write_file(&scratch.0, "dir/new", "new\n");
    write_file(&scratch.0, "gone", "x\n");
    git(&scratch.0, &["add", "gone"]);
    git(&scratch.0, &["commit", "--quiet", "-m", "gone"]);
    std::fs::remove_file(scratch.0.join("gone")).unwrap();

    for path in ["f", "dir/new", "gone"] {
        GitWriter
            .write(&scratch.0, &stage_in(&scratch.0, path))
            .unwrap();
    }
    assert_eq!(staged_names(&scratch.0), "A\tdir/new\nM\tf\nD\tgone");
    for path in ["f", "dir/new", "gone"] {
        GitWriter
            .write(&scratch.0, &unstage_in(&scratch.0, path))
            .unwrap();
    }
    assert_eq!(staged_names(&scratch.0), "");
    assert_eq!(
        std::fs::read_to_string(scratch.0.join("f")).unwrap(),
        "changed\n"
    );
}

#[test]
fn a_file_without_a_change_to_stage_or_unstage_is_refused() {
    let scratch = with_file("stage-refused");
    refused(&scratch.0, &stage_in(&scratch.0, "f"), "no change to stage");
    refused(&scratch.0, &unstage_in(&scratch.0, "f"), "not staged");
}

#[test]
// Verifies: ADR-0033, G-4
fn the_index_is_committed_by_the_configured_identity_on_the_head_shown() {
    let scratch = with_file("commit");
    let head = git(&scratch.0, &["rev-parse", "HEAD"]);
    write_file(&scratch.0, "f", "changed\n");
    write_file(&scratch.0, "other", "not staged\n");
    GitWriter
        .write(&scratch.0, &stage_in(&scratch.0, "f"))
        .unwrap();
    let made = GitWriter
        .write(
            &scratch.0,
            &commit_on(&head, "Change f\n\n# kept, as git commit -m keeps it"),
        )
        .unwrap();
    assert_eq!(made, Some(git(&scratch.0, &["rev-parse", "HEAD"])));
    assert_eq!(git(&scratch.0, &["rev-parse", "HEAD^"]), head);
    assert_eq!(
        git(&scratch.0, &["log", "-1", "--format=%an %s"]),
        "Ada Change f"
    );
    let expected = git_command(std::env::temp_dir())
        .args(["stripspace"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write as _;
            child
                .stdin
                .take()
                .unwrap()
                .write_all(b"Change f\n\n# kept, as git commit -m keeps it")?;
            child.wait_with_output()
        })
        .unwrap();
    assert_eq!(
        git(&scratch.0, &["log", "-1", "--format=%B"]) + "\n",
        String::from_utf8(expected.stdout).unwrap()
    );
    assert_eq!(status(&scratch.0), "?? other", "what was not staged stays");
}

#[test]
// Verifies: ADR-0033, G-4
fn a_commit_is_refused_with_nothing_changed() {
    let scratch = with_file("commit-refused");
    let head = git(&scratch.0, &["rev-parse", "HEAD"]);
    refused(
        &scratch.0,
        &commit_on(&head, "nothing"),
        "nothing is staged",
    );

    write_file(&scratch.0, "f", "changed\n");
    GitWriter
        .write(&scratch.0, &stage_in(&scratch.0, "f"))
        .unwrap();
    let moved = "0123456789abcdef0123456789abcdef01234567";
    for (write, why) in [
        (commit_on(moved, "msg"), "no longer the one shown"),
        (commit_on(&head, "  \n"), "needs a message"),
    ] {
        refused(&scratch.0, &write, why);
    }
    git(&scratch.0, &["config", "commit.gpgSign", "true"]);
    refused(&scratch.0, &commit_on(&head, "msg"), "commit.gpgSign");
}

#[test]
// Verifies: ADR-0033
fn an_annotated_tag_is_refused_where_tags_are_signed_and_a_lightweight_one_is_not() {
    let scratch = Scratch::new("tag-signed");
    let (one, _) = repository(&scratch.0);
    git(&scratch.0, &["config", "tag.gpgSign", "true"]);
    refused(&scratch.0, &tag("v1", &one, Some("release")), "tag.gpgSign");
    GitWriter
        .write(&scratch.0, &tag("mark", &one, None))
        .unwrap();
}

#[test]
// Verifies: ADR-0033, G-4
fn a_file_changed_since_it_was_shown_is_neither_staged_nor_unstaged() {
    let scratch = with_file("stage-moved");
    write_file(&scratch.0, "f", "shown\n");
    let shown = stage_in(&scratch.0, "f");
    write_file(&scratch.0, "f", "changed after\n");
    refused(&scratch.0, &shown, "changed since it was shown");

    GitWriter
        .write(&scratch.0, &stage_in(&scratch.0, "f"))
        .unwrap();
    let shown = unstage_in(&scratch.0, "f");
    write_file(&scratch.0, "f", "staged again\n");
    git(&scratch.0, &["add", "f"]);
    refused(&scratch.0, &shown, "changed since it was shown");
}

#[test]
// Verifies: ADR-0033, G-4
fn a_commit_is_refused_while_a_merge_is_in_progress() {
    let scratch = with_file("merging");
    git(&scratch.0, &["switch", "--quiet", "-c", "side"]);
    write_file(&scratch.0, "side", "side\n");
    git(&scratch.0, &["add", "side"]);
    git(&scratch.0, &["commit", "--quiet", "-m", "side"]);
    git(&scratch.0, &["switch", "--quiet", "main"]);
    git(
        &scratch.0,
        &["merge", "--quiet", "--no-commit", "--no-ff", "side"],
    );
    let head = git(&scratch.0, &["rev-parse", "HEAD"]);
    refused(
        &scratch.0,
        &commit_on(&head, "merge"),
        "a merge is in progress",
    );
    assert!(scratch.0.join(".git/MERGE_HEAD").exists());
}

#[test]
fn a_commit_cleans_its_message_as_commit_cleanup_says() {
    let scratch = with_file("cleanup");
    git(&scratch.0, &["config", "commit.cleanup", "strip"]);
    let head = git(&scratch.0, &["rev-parse", "HEAD"]);
    write_file(&scratch.0, "f", "changed\n");
    GitWriter
        .write(&scratch.0, &stage_in(&scratch.0, "f"))
        .unwrap();
    GitWriter
        .write(&scratch.0, &commit_on(&head, "# gone\nkept"))
        .unwrap();
    assert_eq!(git(&scratch.0, &["log", "-1", "--format=%B"]), "kept");
}

#[test]
// Verifies: ADR-0033
fn an_annotated_tag_is_refused_where_annotated_tags_are_always_signed() {
    let scratch = Scratch::new("force-sign");
    let (one, _) = repository(&scratch.0);
    git(&scratch.0, &["config", "tag.forceSignAnnotated", "true"]);
    refused(
        &scratch.0,
        &tag("v1", &one, Some("release")),
        "tag.forceSignAnnotated",
    );
}

/// HEAD as `git` reads it: the branch it is on, or the commit it is detached at.
fn head_of(dir: &Path) -> String {
    let out = git_command(dir)
        .args(["symbolic-ref", "--quiet", "--short", "HEAD"])
        .output()
        .unwrap();
    match out.status.success() {
        true => String::from_utf8(out.stdout).unwrap().trim().to_string(),
        false => format!("detached at {}", git(dir, &["rev-parse", "HEAD"])),
    }
}

/// A repository with `f` committed on main, and a branch `side` one commit on, changing `f` and
/// adding `s`; main checked out.
fn with_side(name: &str) -> (Scratch, String, String) {
    let scratch = with_file(name);
    let main = git(&scratch.0, &["rev-parse", "HEAD"]);
    git(&scratch.0, &["switch", "--quiet", "-c", "side"]);
    write_file(&scratch.0, "f", "side\n");
    write_file(&scratch.0, "s", "side only\n");
    git(&scratch.0, &["add", "f", "s"]);
    git(&scratch.0, &["commit", "--quiet", "-m", "side"]);
    let side = git(&scratch.0, &["rev-parse", "HEAD"]);
    git(&scratch.0, &["switch", "--quiet", "main"]);
    (scratch, main, side)
}

#[test]
// Verifies: ADR-0033, G-4
fn a_branch_is_created_at_the_commit_and_switched_to_where_asked() {
    let (scratch, main, side) = with_side("create-branch");
    let create = |name: &str, commit: &str, switch| Write::CreateBranch {
        name: name.into(),
        commit: commit.into(),
        switch,
    };
    write_file(&scratch.0, "mine", "untracked\n");
    GitWriter
        .write(&scratch.0, &create("here", &main, false))
        .unwrap();
    assert_eq!(git(&scratch.0, &["rev-parse", "here"]), main);
    assert_eq!(head_of(&scratch.0), "main");

    GitWriter
        .write(&scratch.0, &create("there", &side, true))
        .unwrap();
    assert_eq!(head_of(&scratch.0), "there");
    assert_eq!(git(&scratch.0, &["rev-parse", "HEAD"]), side);
    assert_eq!(
        status(&scratch.0),
        "?? mine",
        "the working directory is side's, mine kept"
    );

    for (name, why) in [
        ("here", "exists already"),
        ("-x", "not a name"),
        ("HEAD", "not a name"),
        ("a..b", "not a name"),
    ] {
        refused(&scratch.0, &create(name, &main, true), why);
    }
}

#[test]
// Verifies: ADR-0033, G-4
fn a_switch_takes_the_changes_along_or_is_refused_naming_what_is_in_the_way() {
    let (scratch, main, side) = with_side("switch");
    let switch = |name: &str, tip: &str| Write::SwitchBranch {
        name: name.into(),
        tip: tip.into(),
    };
    write_file(&scratch.0, "mine", "untracked\n");
    GitWriter.write(&scratch.0, &switch("side", &side)).unwrap();
    assert_eq!(head_of(&scratch.0), "side");
    assert_eq!(
        std::fs::read_to_string(scratch.0.join("s")).unwrap(),
        "side only\n"
    );
    assert_eq!(status(&scratch.0), "?? mine");

    write_file(&scratch.0, "f", "my change\n");
    refused(
        &scratch.0,
        &switch("main", &main),
        "changes to f are in the way",
    );
    refused(
        &scratch.0,
        &switch("main", &side),
        "no longer where it was shown",
    );

    git(&scratch.0, &["checkout", "--quiet", "--", "f"]);
    GitWriter
        .write(
            &scratch.0,
            &Write::Detach {
                commit: main.clone(),
            },
        )
        .unwrap();
    assert_eq!(head_of(&scratch.0), format!("detached at {main}"));
}

/// A clone of a repository made by [`with_side`], on main, with `origin` pointing there:
/// (origin, clone, side's tip).
fn cloned_with_side(name: &str) -> (Scratch, Scratch, String) {
    let (origin, _, side) = with_side(&format!("{name}-origin"));
    let clone = Scratch::new(&format!("{name}-clone"));
    std::fs::remove_dir_all(&clone.0).unwrap();
    git(
        Path::new("/"),
        &[
            "clone",
            "--quiet",
            origin.0.to_str().unwrap(),
            clone.0.to_str().unwrap(),
        ],
    );
    (origin, clone, side)
}

#[test]
// Verifies: ADR-0033, G-4
fn a_remote_branch_is_switched_to_through_a_local_branch_that_tracks_it() {
    let (_origin, clone, side) = cloned_with_side("remote");
    let track = Write::TrackRemote {
        remote: "origin".into(),
        branch: "side".into(),
        tip: side.clone(),
    };
    GitWriter.write(&clone.0, &track).unwrap();
    assert_eq!(head_of(&clone.0), "side");
    assert_eq!(
        git(&clone.0, &["rev-parse", "--abbrev-ref", "side@{upstream}"]),
        "origin/side"
    );
    refused(&clone.0, &track, "exists already");
}

#[test]
// Verifies: ADR-0033, G-4
fn a_remote_branch_whose_switch_is_refused_leaves_neither_a_branch_nor_its_upstream() {
    let (_origin, clone, side) = cloned_with_side("track-refused");
    write_file(&clone.0, "f", "mine\n");
    let track = Write::TrackRemote {
        remote: "origin".into(),
        branch: "side".into(),
        tip: side,
    };
    // The branch and its upstream are set before the switch finds f in its way.
    refused(&clone.0, &track, "changes to f are in the way");
}

#[test]
// Verifies: ADR-0033, ADR-0034, G-4
fn a_branch_is_deleted_only_as_shown_and_never_while_checked_out() {
    let (scratch, main, side) = with_side("delete-branch");
    git(&scratch.0, &["branch", "merged", &main]);
    let delete = |name: &str, tip: &str, merged| Write::DeleteBranch {
        name: name.into(),
        tip: tip.into(),
        merged,
    };
    for (write, why) in [
        (
            delete("side", &side, true),
            "whether 'side' is merged changed",
        ),
        (delete("side", &main, false), "no longer where it was shown"),
        (delete("main", &main, true), "is checked out"),
    ] {
        refused(&scratch.0, &write, why);
    }
    GitWriter
        .write(&scratch.0, &delete("merged", &main, true))
        .unwrap();
    GitWriter
        .write(&scratch.0, &delete("side", &side, false))
        .unwrap();
    assert_eq!(
        git(
            &scratch.0,
            &["branch", "--list", "--format=%(refname:short)"]
        ),
        "main"
    );
}

#[test]
// Verifies: ADR-0033, G-4
fn a_switch_carries_what_the_target_leaves_alone_and_refuses_staged_changes_it_touches() {
    let (scratch, _, side) = with_side("switch-carry");
    let switch = Write::SwitchBranch {
        name: "side".into(),
        tip: side.clone(),
    };

    // Staged where the target changes f, the working directory as HEAD has it.
    write_file(&scratch.0, "f", "staged\n");
    git(&scratch.0, &["add", "f"]);
    write_file(&scratch.0, "f", "base\n");
    refused(&scratch.0, &switch, "staged changes to f are in the way");
    git(&scratch.0, &["reset", "--quiet", "--", "f"]);

    // A staged file the target leaves alone goes along; a deleted f the target changes comes
    // back, as git switch has it.
    write_file(&scratch.0, "h", "new\n");
    git(&scratch.0, &["add", "h"]);
    std::fs::remove_file(scratch.0.join("f")).unwrap();
    GitWriter.write(&scratch.0, &switch).unwrap();
    assert_eq!(head_of(&scratch.0), "side");
    assert_eq!(status(&scratch.0), "A  h");
    assert_eq!(
        std::fs::read_to_string(scratch.0.join("f")).unwrap(),
        "side\n"
    );
}

#[test]
// Verifies: ADR-0033, G-4
fn a_branch_checked_out_in_another_working_tree_is_neither_switched_to_nor_deleted() {
    let (scratch, _, side) = with_side("worktree");
    let elsewhere = Scratch::new("worktree-other");
    std::fs::remove_dir_all(&elsewhere.0).unwrap();
    git(
        &scratch.0,
        &[
            "worktree",
            "add",
            "--quiet",
            elsewhere.0.to_str().unwrap(),
            "side",
        ],
    );
    for write in [
        Write::SwitchBranch {
            name: "side".into(),
            tip: side.clone(),
        },
        Write::DeleteBranch {
            name: "side".into(),
            tip: side.clone(),
            merged: false,
        },
    ] {
        refused(&scratch.0, &write, "checked out in the working tree");
    }
}

#[test]
// Verifies: ADR-0033, G-4
fn a_branch_whose_name_clashes_with_another_is_refused_with_nothing_changed() {
    let (scratch, main, side) = with_side("clash");
    git(&scratch.0, &["branch", "feature/x", &main]);
    let create = Write::CreateBranch {
        name: "feature".into(),
        commit: side.clone(),
        switch: true,
    };
    refused(&scratch.0, &create, "cannot be created here");

    // A switch refused after the branch was created takes the branch away again.
    write_file(&scratch.0, "f", "mine\n");
    let create = Write::CreateBranch {
        name: "topic".into(),
        commit: side.clone(),
        switch: true,
    };
    refused(&scratch.0, &create, "in the way");
}

/// A bare remote made with `git` from a repository with `f`, a clone of it with `origin` pointing
/// there, and a commit pushed to the remote after the clone: (remote, clone, the commit).
fn with_remote(name: &str) -> (Scratch, Scratch, String) {
    let work = with_file(&format!("{name}-work"));
    let remote = Scratch::new(&format!("{name}-remote"));
    std::fs::remove_dir_all(&remote.0).unwrap();
    git(
        Path::new("/"),
        &[
            "clone",
            "--quiet",
            "--bare",
            work.0.to_str().unwrap(),
            remote.0.to_str().unwrap(),
        ],
    );
    let clone = Scratch::new(&format!("{name}-clone"));
    std::fs::remove_dir_all(&clone.0).unwrap();
    git(
        Path::new("/"),
        &[
            "clone",
            "--quiet",
            remote.0.to_str().unwrap(),
            clone.0.to_str().unwrap(),
        ],
    );
    write_file(&work.0, "f", "later\n");
    git(&work.0, &["commit", "--quiet", "-am", "later"]);
    git(
        &work.0,
        &["push", "--quiet", remote.0.to_str().unwrap(), "main"],
    );
    let later = git(&work.0, &["rev-parse", "HEAD"]);
    (remote, clone, later)
}

#[test]
// Verifies: ADR-0035, G-4
fn a_fetch_moves_the_remote_branches_as_git_fetch_does() {
    let (_remote, clone, later) = with_remote("fetch");
    let before = snapshot(&clone.0);
    let mut told = Vec::new();
    GitWriter
        .write_reporting(
            &clone.0,
            &Write::Fetch {
                remote: "origin".into(),
            },
            &mut |p| {
                told.push(p.to_string());
                true
            },
        )
        .unwrap();
    assert_eq!(git(&clone.0, &["rev-parse", "origin/main"]), later);
    let mut after = snapshot(&clone.0);
    let local = |refs: &str| -> Vec<String> {
        refs.lines()
            .filter(|r| !r.starts_with("refs/remotes/"))
            .map(str::to_string)
            .collect()
    };
    assert_eq!(local(&after.refs), local(&before.refs));
    after.refs = before.refs.clone();
    assert_eq!(after, before, "only the remote branches move");
    let refs = ocelli_git::GitReader.load_refs(&clone.0).unwrap();
    assert_eq!(
        refs.default_remote.as_deref(),
        Some("origin"),
        "main's upstream's remote"
    );
    assert!(!told.is_empty(), "the fetch told how far it came");
    assert!(
        told.iter().all(|t| t.starts_with("Fetching origin")),
        "{told:?}"
    );
}

#[test]
// Verifies: ADR-0035
fn a_fetch_cancelled_or_of_no_remote_changes_nothing() {
    let (_remote, clone, _) = with_remote("fetch-cancel");
    let fetch = |name: &str| Write::Fetch {
        remote: name.into(),
    };
    refused_reporting(&clone.0, &fetch("origin"), "cancelled", false, |_| {});
    refused(&clone.0, &fetch("nowhere"), "no remote named");
}

fn pull_of(dir: &Path, branch: &str) -> Write {
    Write::Pull {
        branch: branch.into(),
        tip: git(dir, &["rev-parse", branch]),
    }
}

#[test]
// Verifies: ADR-0035, G-4
fn a_pull_fast_forwards_the_checked_out_branch_as_git_pull_ff_only_does() {
    let (_remote, clone, later) = with_remote("pull");
    write_file(&clone.0, "mine", "untracked\n");
    GitWriter
        .write(&clone.0, &pull_of(&clone.0, "main"))
        .unwrap();
    assert_eq!(git(&clone.0, &["rev-parse", "HEAD"]), later);
    assert_eq!(head_of(&clone.0), "main");
    assert_eq!(
        std::fs::read_to_string(clone.0.join("f")).unwrap(),
        "later\n"
    );
    assert_eq!(
        status(&clone.0),
        "?? mine",
        "the untracked file is carried along"
    );

    // Up to date: nothing to do, nothing done.
    let before = snapshot(&clone.0);
    GitWriter
        .write(&clone.0, &pull_of(&clone.0, "main"))
        .unwrap();
    assert_eq!(snapshot(&clone.0), before);
}

#[test]
// Verifies: ADR-0035, G-4
fn a_pull_is_refused_where_the_branch_diverged_has_no_upstream_or_is_not_checked_out() {
    let (_remote, clone, _) = with_remote("pull-refused");
    write_file(&clone.0, "f", "mine\n");
    git(&clone.0, &["commit", "--quiet", "-am", "mine"]);
    // A refused pull has fetched, as `git pull` has, which moves the remote branches: fetched
    // before, its refusal leaves everything as it was.
    git(&clone.0, &["fetch", "--quiet", "origin"]);
    refused(&clone.0, &pull_of(&clone.0, "main"), "have diverged");

    git(&clone.0, &["branch", "loose"]);
    git(&clone.0, &["switch", "--quiet", "loose"]);
    refused(&clone.0, &pull_of(&clone.0, "loose"), "no upstream");
    refused(&clone.0, &pull_of(&clone.0, "main"), "not checked out");
}

fn push_of(dir: &Path, branch: &str) -> Write {
    Write::Push {
        branch: branch.into(),
        tip: git(dir, &["rev-parse", branch]),
    }
}

#[test]
// Verifies: ADR-0035, G-4
fn a_push_sends_the_branch_to_its_upstream_or_sets_one_as_git_push_does() {
    let (remote, clone, _) = with_remote("push");
    GitWriter
        .write(&clone.0, &pull_of(&clone.0, "main"))
        .unwrap();
    write_file(&clone.0, "f", "pushed\n");
    git(&clone.0, &["commit", "--quiet", "-am", "pushed"]);
    let pushed = git(&clone.0, &["rev-parse", "HEAD"]);
    GitWriter
        .write(&clone.0, &push_of(&clone.0, "main"))
        .unwrap();
    assert_eq!(git(&remote.0, &["rev-parse", "main"]), pushed);
    assert_eq!(git(&clone.0, &["rev-parse", "origin/main"]), pushed);

    git(&clone.0, &["switch", "--quiet", "-c", "topic"]);
    GitWriter
        .write(&clone.0, &push_of(&clone.0, "topic"))
        .unwrap();
    assert_eq!(git(&remote.0, &["rev-parse", "topic"]), pushed);
    assert_eq!(
        git(&clone.0, &["rev-parse", "--abbrev-ref", "topic@{upstream}"]),
        "origin/topic"
    );
}

#[test]
// Verifies: ADR-0035, G-4
fn a_push_that_is_not_a_fast_forward_is_refused_with_nothing_changed() {
    let (remote, clone, later) = with_remote("push-refused");
    write_file(&clone.0, "f", "mine\n");
    git(&clone.0, &["commit", "--quiet", "-am", "mine"]);
    refused(&clone.0, &push_of(&clone.0, "main"), "pull first");
    assert_eq!(
        git(&remote.0, &["rev-parse", "main"]),
        later,
        "the remote kept its branch"
    );
}

#[test]
// Verifies: ADR-0035, G-4
fn a_commit_made_while_a_pull_fetches_is_neither_lost_nor_checked_out_over() {
    let (_remote, clone, _) = with_remote("pull-race");
    let pull = pull_of(&clone.0, "main");
    // Another program commits while the pull fetches, and fetches too, so that the remote
    // branch the pull's own fetch moves is where it moves it already.
    refused_reporting(
        &clone.0,
        &pull,
        "no longer where it was shown",
        true,
        |dir| {
            write_file(dir, "x", "made meanwhile\n");
            git(dir, &["add", "x"]);
            git(dir, &["commit", "--quiet", "-m", "meanwhile"]);
            git(dir, &["fetch", "--quiet", "origin"]);
        },
    );
    assert_eq!(git(&clone.0, &["log", "-1", "--format=%s"]), "meanwhile");
}

#[test]
// Verifies: ADR-0035
fn a_pull_whose_branch_another_program_holds_locked_changes_nothing() {
    let (_remote, clone, _) = with_remote("pull-locked");
    // Fetched before, as for a pull refused where the branches diverged.
    git(&clone.0, &["fetch", "--quiet", "origin"]);
    let pull = pull_of(&clone.0, "main");
    let lock = clone.0.join(".git/refs/heads/main.lock");
    std::fs::write(&lock, "").unwrap();
    refused(&clone.0, &pull, "locked by another program");
    std::fs::remove_file(&lock).unwrap();
}

#[test]
// Verifies: ADR-0035, G-4
fn a_push_follows_push_default_as_git_push_does() {
    let (remote, clone, _) = with_remote("push-default");
    GitWriter
        .write(&clone.0, &pull_of(&clone.0, "main"))
        .unwrap();
    git(&clone.0, &["switch", "--quiet", "-c", "feat"]);
    write_file(&clone.0, "f", "feat\n");
    git(&clone.0, &["commit", "--quiet", "-am", "feat"]);
    let main_there = git(&remote.0, &["rev-parse", "main"]);

    // `simple`, the default: an upstream named differently is refused.
    git(
        &clone.0,
        &["branch", "--quiet", "--set-upstream-to", "origin/main"],
    );
    refused(&clone.0, &push_of(&clone.0, "feat"), "named differently");
    assert_eq!(git(&remote.0, &["rev-parse", "main"]), main_there);

    // `upstream` pushes there, as git does.
    git(&clone.0, &["config", "push.default", "upstream"]);
    GitWriter
        .write(&clone.0, &push_of(&clone.0, "feat"))
        .unwrap();
    assert_eq!(
        git(&remote.0, &["rev-parse", "main"]),
        git(&clone.0, &["rev-parse", "feat"])
    );

    for (mode, why) in [("nothing", "is nothing"), ("matching", "every branch")] {
        git(&clone.0, &["config", "push.default", mode]);
        refused(&clone.0, &push_of(&clone.0, "feat"), why);
    }

    // An upstream on a local branch is no remote to push to.
    git(&clone.0, &["config", "--unset", "push.default"]);
    git(
        &clone.0,
        &["branch", "--quiet", "--set-upstream-to", "main"],
    );
    refused(&clone.0, &push_of(&clone.0, "feat"), "local branch");
}

#[test]
// Verifies: ADR-0033, G-4

fn a_path_is_unstaged_and_diffed_as_written_never_as_a_pattern() {
    let scratch = with_file("stage-pattern");
    let d = &scratch.0;
    for name in ["x[ab].txt", "xa.txt"] {
        std::fs::write(d.join(name), "base\n").unwrap();
    }
    git(d, &["add", "--all"]);
    git(d, &["commit", "--quiet", "-m", "both"]);
    for name in ["x[ab].txt", "xa.txt"] {
        std::fs::write(d.join(name), "changed\n").unwrap();
    }
    git(d, &["add", "--all"]);
    GitWriter.write(d, &unstage_in(d, "x[ab].txt")).unwrap();
    assert_eq!(staged_names(d), "M\txa.txt");
    let diff = ocelli_git::GitReader
        .load_file_diff(d, ocelli_core::model::WORKDIR_SHA, "x[ab].txt")
        .unwrap();
    let changed = diff
        .iter()
        .filter(|l| l.content.contains("changed"))
        .count();
    assert_eq!(changed, 1, "one file's change, not xa.txt's too: {diff:?}");
}

#[test]
// Verifies: ADR-0033, Q-6
fn a_switch_that_puts_a_file_where_a_directory_holds_untracked_files_is_refused() {
    for (name, kept) in [("untracked", "d/u"), ("ignored", "d/u.log")] {
        let scratch = with_file(&format!("dir-to-file-{name}"));
        let d = &scratch.0;
        std::fs::write(d.join(".gitignore"), "*.log\n").unwrap();
        std::fs::create_dir(d.join("d")).unwrap();
        std::fs::write(d.join("d/x"), "x\n").unwrap();
        git(d, &["add", "--all"]);
        git(d, &["commit", "--quiet", "-m", "d is a directory"]);
        git(d, &["switch", "--quiet", "-c", "t"]);
        git(d, &["rm", "--quiet", "-r", "d"]);
        std::fs::write(d.join("d"), "a file\n").unwrap();
        git(d, &["add", "d"]);
        git(d, &["commit", "--quiet", "-m", "d is a file"]);
        let tip = git(d, &["rev-parse", "t"]);
        git(d, &["switch", "--quiet", "main"]);
        std::fs::write(d.join(kept), "the developer's\n").unwrap();
        let switch = Write::SwitchBranch {
            name: "t".into(),
            tip,
        };
        refused(d, &switch, "holding files the commit does not track");
        assert_eq!(
            std::fs::read_to_string(d.join(kept)).unwrap(),
            "the developer's\n"
        );
    }
}

#[test]
// Verifies: ADR-0033, Q-6
fn a_stash_during_a_stopped_operation_is_refused_with_the_operation_kept() {
    let scratch = with_file("stash-stopped");
    let d = &scratch.0;
    git(d, &["switch", "--quiet", "-c", "side"]);
    std::fs::write(d.join("f"), "theirs\n").unwrap();
    git(d, &["commit", "--quiet", "-am", "theirs"]);
    git(d, &["switch", "--quiet", "main"]);
    std::fs::write(d.join("f"), "ours\n").unwrap();
    git(d, &["commit", "--quiet", "-am", "ours"]);
    let _ = git_command(d).args(["merge", "side"]).output().unwrap();
    std::fs::write(d.join("f"), "resolved\n").unwrap();
    refused(
        d,
        &Write::CreateStash {
            message: None,
            untracked: false,
        },
        "a merge is in progress",
    );
}
