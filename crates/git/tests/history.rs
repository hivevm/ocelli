//! Verifies: ADR-0033, G-5, G-6
//!
//! Merge, rebase, cherry-pick, revert, and reset, and the resolution of the conflicts they stop
//! on, on repositories made with git: each compared with git doing the same on a twin of the
//! repository, and each refusal leaving the repository as it was.

#![cfg(unix)]

use ocelli_core::model::Operation;
use ocelli_core::port::{RepositoryReader, RepositoryWriter, ResetMode, Take, Write};
use ocelli_git::{GitReader, GitWriter};
use std::path::{Path, PathBuf};
use std::process::Command;

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("ocelli-history-{name}-{}", std::process::id()));
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
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=Ada",
            "-c",
            "user.email=ada@example.invalid",
        ])
        .args(["-c", "core.editor=true", "-c", "commit.gpgSign=false"])
        .env("GIT_EDITOR", "true")
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

/// Runs git and says whether it succeeded, for the commands that stop on a conflict.
fn git_may_fail(dir: &Path, args: &[&str]) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=Ada",
            "-c",
            "user.email=ada@example.invalid",
        ])
        .env("GIT_EDITOR", "true")
        .args(args)
        .output()
        .unwrap()
        .status
        .success()
}

fn write_file(dir: &Path, path: &str, content: &str) {
    std::fs::write(dir.join(path), content).unwrap();
}

/// A repository whose `main` and `side` both start from `f` = "base\n"; `main` then writes `f`
/// to `ours`, and `side` adds `s` and writes `f` to `theirs` (a conflict where both differ from
/// base, none where `theirs` equals base). Identity in its configuration.
fn diverged(name: &str, ours: &str, theirs: &str) -> Scratch {
    let s = Scratch::new(name);
    let d = &s.0;
    git(d, &["init", "--quiet", "-b", "main"]);
    git(d, &["config", "user.name", "Ada"]);
    git(d, &["config", "user.email", "ada@example.invalid"]);
    write_file(d, "f", "base\n");
    write_file(d, "g", "g\n");
    git(d, &["add", "f", "g"]);
    git(d, &["commit", "--quiet", "-m", "base"]);
    git(d, &["switch", "--quiet", "-c", "side"]);
    write_file(d, "s", "side\n");
    write_file(d, "f", theirs);
    git(d, &["add", "s", "f"]);
    git(
        d,
        &[
            "commit",
            "--quiet",
            "--author",
            "Bea <bea@example.invalid>",
            "-m",
            "side work",
        ],
    );
    git(d, &["switch", "--quiet", "main"]);
    write_file(d, "f", ours);
    write_file(d, "g", "g on main\n");
    git(d, &["commit", "--quiet", "-am", "main work"]);
    s
}

/// A copy of the repository, for git to do on what Ocelli does on the original.
fn twin(of: &Scratch, name: &str) -> Scratch {
    let t = Scratch::new(name);
    std::fs::remove_dir_all(&t.0).unwrap();
    let status = Command::new("cp")
        .arg("-a")
        .arg(&of.0)
        .arg(&t.0)
        .status()
        .unwrap();
    assert!(status.success());
    t
}

fn head(dir: &Path) -> String {
    git(dir, &["rev-parse", "HEAD"])
}

fn tree(dir: &Path) -> String {
    git(dir, &["rev-parse", "HEAD^{tree}"])
}

/// What a write must leave alone where it is refused.
fn state(dir: &Path) -> (String, String, String, String) {
    (
        head(dir),
        git(dir, &["status", "--porcelain", "--ignored"]),
        git(dir, &["ls-files", "--stage"]),
        git(dir, &["for-each-ref", "--format=%(refname) %(objectname)"]),
    )
}

fn merge(dir: &Path, commit: &str, name: Option<&str>) -> Write {
    Write::Merge {
        branch: "main".into(),
        tip: head(dir),
        commit: git(dir, &["rev-parse", commit]),
        name: name.map(str::to_string),
    }
}

#[test]
// Verifies: ADR-0033, G-5
fn a_merge_makes_what_git_merge_makes() {
    let repo = diverged("merge", "ours\n", "base\n");
    let other = twin(&repo, "merge-twin");
    let made = GitWriter
        .write(&repo.0, &merge(&repo.0, "side", Some("side")))
        .unwrap();
    git(&other.0, &["merge", "--quiet", "--no-edit", "side"]);
    assert_eq!(made, Some(head(&repo.0)));
    assert_eq!(tree(&repo.0), tree(&other.0));
    assert_eq!(
        git(&repo.0, &["log", "-1", "--format=%P%n%B"])
            .lines()
            .count(),
        git(&other.0, &["log", "-1", "--format=%P%n%B"])
            .lines()
            .count()
    );
    assert_eq!(
        git(&repo.0, &["log", "-1", "--format=%s"]),
        "Merge branch 'side'"
    );
    assert_eq!(git(&repo.0, &["status", "--porcelain"]), "");
}

#[test]
// Verifies: ADR-0033, G-5
fn a_merge_fast_forwards_where_it_can_unless_merge_ff_says_otherwise() {
    let repo = diverged("merge-ff", "base\n", "theirs\n");
    git(&repo.0, &["reset", "--quiet", "--hard", "HEAD~1"]);
    let side = git(&repo.0, &["rev-parse", "side"]);
    GitWriter
        .write(&repo.0, &merge(&repo.0, "side", None))
        .unwrap();
    assert_eq!(head(&repo.0), side, "fast-forwarded");

    let repo = diverged("merge-ff-only", "ours\n", "base\n");
    git(&repo.0, &["config", "merge.ff", "only"]);
    let before = state(&repo.0);
    let error = GitWriter
        .write(&repo.0, &merge(&repo.0, "side", None))
        .unwrap_err();
    assert!(error.to_string().contains("merge.ff is only"), "{error}");
    assert_eq!(state(&repo.0), before);
}

#[test]
// Verifies: ADR-0033, G-5
fn a_merge_that_conflicts_stops_where_git_stops() {
    let repo = diverged("merge-conflict", "ours\n", "theirs\n");
    let made = GitWriter
        .write(&repo.0, &merge(&repo.0, "side", Some("side")))
        .unwrap();
    assert_eq!(made, None, "nothing committed");
    assert!(repo.0.join(".git/MERGE_HEAD").exists());
    assert_eq!(
        git(&repo.0, &["diff", "--name-only", "--diff-filter=U"]),
        "f"
    );
    let refs = GitReader.load_refs(&repo.0).unwrap();
    let stopped = refs.in_progress.unwrap();
    assert_eq!(stopped.operation, Operation::Merge);
    assert_eq!(stopped.conflicts.len(), 1);
    assert_eq!(stopped.conflicts[0].path, "f");
}

#[test]
// Verifies: ADR-0033
fn an_operation_over_uncommitted_changes_or_another_in_progress_is_refused() {
    let repo = diverged("refused", "ours\n", "base\n");
    write_file(&repo.0, "g", "mine\n");
    let before = state(&repo.0);
    let error = GitWriter
        .write(&repo.0, &merge(&repo.0, "side", None))
        .unwrap_err();
    assert!(
        error.to_string().contains("uncommitted changes to g"),
        "{error}"
    );
    assert_eq!(state(&repo.0), before);

    let repo = diverged("refused-busy", "ours\n", "theirs\n");
    assert!(!git_may_fail(&repo.0, &["merge", "side"]));
    let before = state(&repo.0);
    let pick = Write::CherryPick {
        branch: "main".into(),
        tip: head(&repo.0),
        commit: git(&repo.0, &["rev-parse", "side"]),
    };
    let error = GitWriter.write(&repo.0, &pick).unwrap_err();
    assert!(
        error.to_string().contains("a merge is in progress"),
        "{error}"
    );
    assert_eq!(state(&repo.0), before);
}

#[test]
// Verifies: ADR-0033, G-5
fn a_cherry_pick_and_a_revert_commit_as_git_does() {
    let repo = diverged("pick", "ours\n", "base\n");
    let other = twin(&repo, "pick-twin");
    let side = git(&repo.0, &["rev-parse", "side"]);
    let pick = Write::CherryPick {
        branch: "main".into(),
        tip: head(&repo.0),
        commit: side.clone(),
    };
    GitWriter.write(&repo.0, &pick).unwrap();
    git(&other.0, &["cherry-pick", "side"]);
    assert_eq!(tree(&repo.0), tree(&other.0));
    let said = |d: &Path| git(d, &["log", "-1", "--format=%an %s"]);
    assert_eq!(said(&repo.0), said(&other.0));
    assert_eq!(said(&repo.0), "Bea side work");

    let revert = Write::Revert {
        branch: "main".into(),
        tip: head(&repo.0),
        commit: head(&repo.0),
    };
    let picked = head(&other.0);
    GitWriter.write(&repo.0, &revert).unwrap();
    git(&other.0, &["revert", "--no-edit", &picked]);
    assert_eq!(tree(&repo.0), tree(&other.0));
    let body = |d: &Path| git(d, &["log", "-1", "--format=%B"]);
    assert_eq!(
        body(&repo.0).lines().next(),
        body(&other.0).lines().next(),
        "Revert \"side work\""
    );
    assert!(
        body(&repo.0).contains("This reverts commit"),
        "{}",
        body(&repo.0)
    );
}

#[test]
// Verifies: ADR-0033
fn a_merge_commit_is_neither_picked_nor_reverted() {
    let repo = diverged("pick-merge", "ours\n", "base\n");
    git(&repo.0, &["merge", "--quiet", "--no-edit", "side"]);
    git(&repo.0, &["reset", "--quiet", "--hard", "HEAD~1"]);
    let merged = git(&repo.0, &["rev-parse", "HEAD@{1}"]);
    let before = state(&repo.0);
    for write in [
        Write::CherryPick {
            branch: "main".into(),
            tip: head(&repo.0),
            commit: merged.clone(),
        },
        Write::Revert {
            branch: "main".into(),
            tip: head(&repo.0),
            commit: merged.clone(),
        },
    ] {
        let error = GitWriter.write(&repo.0, &write).unwrap_err();
        assert!(error.to_string().contains("merge commit"), "{error}");
    }
    assert_eq!(state(&repo.0), before);
}

#[test]
// Verifies: ADR-0033, G-5
fn a_rebase_replays_the_branch_as_git_rebase_does_and_stops_on_a_conflict() {
    let repo = diverged("rebase", "ours\n", "base\n");
    let other = twin(&repo, "rebase-twin");
    let rebase = Write::Rebase {
        branch: "main".into(),
        tip: head(&repo.0),
        onto: git(&repo.0, &["rev-parse", "side"]),
    };
    GitWriter.write(&repo.0, &rebase).unwrap();
    git(&other.0, &["rebase", "--quiet", "side"]);
    assert_eq!(tree(&repo.0), tree(&other.0));
    let subjects = |d: &Path| git(d, &["log", "--format=%s"]);
    assert_eq!(subjects(&repo.0), subjects(&other.0));
    assert_eq!(git(&repo.0, &["rev-parse", "--abbrev-ref", "HEAD"]), "main");

    let repo = diverged("rebase-conflict", "ours\n", "theirs\n");
    let rebase = Write::Rebase {
        branch: "main".into(),
        tip: head(&repo.0),
        onto: git(&repo.0, &["rev-parse", "side"]),
    };
    assert_eq!(GitWriter.write(&repo.0, &rebase).unwrap(), None);
    assert!(
        git(&repo.0, &["status"]).contains("rebas"),
        "git status reads a rebase"
    );
    let stopped = GitReader.load_refs(&repo.0).unwrap().in_progress.unwrap();
    assert_eq!(stopped.operation, Operation::Rebase);
}

#[test]
// Verifies: ADR-0033, G-5
fn a_reset_moves_the_branch_as_git_reset_does_in_each_mode() {
    for (mode, flag) in [
        (ResetMode::Soft, "--soft"),
        (ResetMode::Mixed, "--mixed"),
        (ResetMode::Hard, "--hard"),
    ] {
        let repo = diverged(&format!("reset{flag}"), "ours\n", "base\n");
        write_file(&repo.0, "g", "mine\n");
        let other = twin(&repo, &format!("reset{flag}-twin"));
        let reset = Write::Reset {
            branch: "main".into(),
            tip: head(&repo.0),
            commit: git(&repo.0, &["rev-parse", "HEAD~1"]),
            mode,
        };
        GitWriter.write(&repo.0, &reset).unwrap();
        git(&other.0, &["reset", "--quiet", flag, "HEAD~1"]);
        assert_eq!(state(&repo.0), state(&other.0), "{flag}");
    }
}

fn conflict_of(dir: &Path, path: &str) -> ocelli_core::model::Conflict {
    GitReader
        .load_refs(dir)
        .unwrap()
        .in_progress
        .unwrap()
        .conflicts
        .into_iter()
        .find(|c| c.path == path)
        .unwrap()
}

fn resolve(dir: &Path, path: &str, take: Take) -> Write {
    Write::Resolve {
        path: path.into(),
        conflict: conflict_of(dir, path),
        take,
    }
}

#[test]
// Verifies: ADR-0033, G-6
fn a_conflict_git_left_is_resolved_as_git_checkout_and_git_add_resolve_it() {
    for (take, side) in [(Take::Ours, "--ours"), (Take::Theirs, "--theirs")] {
        let repo = diverged(&format!("resolve{side}"), "ours\n", "theirs\n");
        assert!(!git_may_fail(&repo.0, &["merge", "side"]));
        let other = twin(&repo, &format!("resolve{side}-twin"));
        GitWriter
            .write(&repo.0, &resolve(&repo.0, "f", take))
            .unwrap();
        git(&other.0, &["checkout", side, "f"]);
        git(&other.0, &["add", "f"]);
        assert_eq!(state(&repo.0), state(&other.0), "{side}");

        let at = head(&repo.0);
        let carry_on = Write::Continue {
            operation: Operation::Merge,
            head: at,
        };
        GitWriter.write(&repo.0, &carry_on).unwrap();
        git(&other.0, &["commit", "--quiet", "--no-edit"]);
        assert_eq!(tree(&repo.0), tree(&other.0));
        assert_eq!(
            git(&repo.0, &["log", "-1", "--format=%P"])
                .split(' ')
                .count(),
            2
        );
        assert!(!repo.0.join(".git/MERGE_HEAD").exists());
    }
}

#[test]
// Verifies: ADR-0033
fn a_file_resolved_as_edited_is_refused_while_a_marker_remains() {
    let repo = diverged("as-edited", "ours\n", "theirs\n");
    assert!(!git_may_fail(&repo.0, &["merge", "side"]));
    let before = state(&repo.0);
    let error = GitWriter
        .write(&repo.0, &resolve(&repo.0, "f", Take::AsEdited))
        .unwrap_err();
    assert!(error.to_string().contains("conflict marker"), "{error}");
    let continue_now = Write::Continue {
        operation: Operation::Merge,
        head: head(&repo.0),
    };
    let error = GitWriter.write(&repo.0, &continue_now).unwrap_err();
    assert!(error.to_string().contains("still conflict"), "{error}");
    assert_eq!(state(&repo.0), before);

    write_file(&repo.0, "f", "both\n");
    GitWriter
        .write(&repo.0, &resolve(&repo.0, "f", Take::AsEdited))
        .unwrap();
    assert_eq!(
        git(&repo.0, &["diff", "--name-only", "--diff-filter=U"]),
        ""
    );
}

#[test]
// Verifies: ADR-0033, G-6
fn a_stopped_cherry_pick_continues_with_its_author_and_an_abort_goes_back() {
    let repo = diverged("pick-continue", "ours\n", "theirs\n");
    let pick = Write::CherryPick {
        branch: "main".into(),
        tip: head(&repo.0),
        commit: git(&repo.0, &["rev-parse", "side"]),
    };
    assert_eq!(GitWriter.write(&repo.0, &pick).unwrap(), None);
    GitWriter
        .write(&repo.0, &resolve(&repo.0, "f", Take::Theirs))
        .unwrap();
    let carry_on = Write::Continue {
        operation: Operation::CherryPick,
        head: head(&repo.0),
    };
    GitWriter.write(&repo.0, &carry_on).unwrap();
    assert_eq!(
        git(&repo.0, &["log", "-1", "--format=%an %s"]),
        "Bea side work"
    );
    assert_eq!(git(&repo.0, &["status", "--porcelain"]), "");

    for operation in ["merge", "cherry-pick"] {
        let repo = diverged(&format!("abort-{operation}"), "ours\n", "theirs\n");
        write_file(&repo.0, "untracked", "mine\n");
        let before = state(&repo.0);
        assert!(!git_may_fail(&repo.0, &[operation, "side"]));
        let op = match operation {
            "merge" => Operation::Merge,
            _ => Operation::CherryPick,
        };
        let abort = Write::Abort {
            operation: op,
            head: head(&repo.0),
        };
        GitWriter.write(&repo.0, &abort).unwrap();
        assert_eq!(state(&repo.0), before, "{operation} --abort");
    }
}

#[test]
// Verifies: ADR-0033, G-6
fn a_stopped_rebase_continues_to_its_end_and_an_abort_restores_the_branch() {
    let repo = diverged("rebase-continue", "ours\n", "theirs\n");
    let other = twin(&repo, "rebase-continue-twin");
    let tip = head(&repo.0);
    let rebase = Write::Rebase {
        branch: "main".into(),
        tip: tip.clone(),
        onto: git(&repo.0, &["rev-parse", "side"]),
    };
    assert_eq!(GitWriter.write(&repo.0, &rebase).unwrap(), None);
    GitWriter
        .write(&repo.0, &resolve(&repo.0, "f", Take::Ours))
        .unwrap();
    let carry_on = Write::Continue {
        operation: Operation::Rebase,
        head: head(&repo.0),
    };
    GitWriter.write(&repo.0, &carry_on).unwrap();
    assert!(!git_may_fail(&other.0, &["rebase", "side"]));
    git(&other.0, &["checkout", "--ours", "f"]);
    git(&other.0, &["add", "f"]);
    git(&other.0, &["rebase", "--continue"]);
    assert_eq!(
        tree(&repo.0),
        tree(&other.0),
        "ours is the new base's side, as in git"
    );
    assert_eq!(git(&repo.0, &["rev-parse", "--abbrev-ref", "HEAD"]), "main");
    assert_eq!(GitReader.load_refs(&repo.0).unwrap().in_progress, None);

    let repo = diverged("rebase-abort", "ours\n", "theirs\n");
    let before = state(&repo.0);
    let rebase = Write::Rebase {
        branch: "main".into(),
        tip: head(&repo.0),
        onto: git(&repo.0, &["rev-parse", "side"]),
    };
    assert_eq!(GitWriter.write(&repo.0, &rebase).unwrap(), None);
    let abort = Write::Abort {
        operation: Operation::Rebase,
        head: head(&repo.0),
    };
    GitWriter.write(&repo.0, &abort).unwrap();
    assert_eq!(state(&repo.0), before);
}

#[test]
// Verifies: ADR-0033
fn a_rebase_either_side_started_is_aborted_by_the_other_and_continued_where_it_started() {
    // git starts it: Ocelli refuses to continue it and aborts it as git would.
    let repo = diverged("git-rebase", "ours\n", "theirs\n");
    let before = state(&repo.0);
    assert!(!git_may_fail(&repo.0, &["rebase", "side"]));
    GitWriter
        .write(&repo.0, &resolve(&repo.0, "f", Take::Ours))
        .unwrap();
    let carry_on = Write::Continue {
        operation: Operation::Rebase,
        head: head(&repo.0),
    };
    let error = GitWriter.write(&repo.0, &carry_on).unwrap_err();
    assert!(error.to_string().contains("started by git"), "{error}");
    let abort = Write::Abort {
        operation: Operation::Rebase,
        head: head(&repo.0),
    };
    GitWriter.write(&repo.0, &abort).unwrap();
    assert_eq!(state(&repo.0), before);

    // Ocelli starts it: git aborts it.
    let repo = diverged("ocelli-rebase", "ours\n", "theirs\n");
    let before = state(&repo.0);
    let rebase = Write::Rebase {
        branch: "main".into(),
        tip: head(&repo.0),
        onto: git(&repo.0, &["rev-parse", "side"]),
    };
    assert_eq!(GitWriter.write(&repo.0, &rebase).unwrap(), None);
    git(&repo.0, &["rebase", "--abort"]);
    assert_eq!(state(&repo.0), before);
}

#[test]
// Verifies: ADR-0033, G-6
fn a_stopped_revert_is_aborted_as_git_revert_abort_does() {
    let repo = diverged("revert-abort", "ours\n", "base\n");
    let reverted = head(&repo.0);
    write_file(&repo.0, "f", "again\n");
    git(&repo.0, &["commit", "--quiet", "-am", "again"]);
    write_file(&repo.0, "untracked", "mine\n");
    let before = state(&repo.0);
    let revert = Write::Revert {
        branch: "main".into(),
        tip: head(&repo.0),
        commit: reverted,
    };
    assert_eq!(GitWriter.write(&repo.0, &revert).unwrap(), None, "stopped");
    assert_eq!(
        GitReader
            .load_refs(&repo.0)
            .unwrap()
            .in_progress
            .unwrap()
            .operation,
        Operation::Revert
    );
    let abort = Write::Abort {
        operation: Operation::Revert,
        head: head(&repo.0),
    };
    GitWriter.write(&repo.0, &abort).unwrap();
    assert_eq!(state(&repo.0), before);
}

mod programs;

#[test]
// Verifies: ADR-0033, Q-4
fn history_operations_run_none_of_the_repositorys_programs() {
    let repo = diverged("programs", "ours\n", "theirs\n");
    let tools = Scratch::new("programs-ran");
    let markers = programs::set_everywhere(&repo.0, &tools.0);
    let mut config = git2::Repository::open(&repo.0).unwrap().config().unwrap();
    for (key, _) in programs::SWITCHES {
        config.remove(key).unwrap();
    }
    let rev = |r: &str| {
        let repository = git2::Repository::open(&repo.0).unwrap();
        repository.revparse_single(r).unwrap().id().to_string()
    };
    let at = || rev("HEAD");

    // A merge that stops, resolved and continued: pre-merge-commit, post-merge, commit-msg.
    let merge = Write::Merge {
        branch: "main".into(),
        tip: at(),
        commit: rev("side"),
        name: Some("side".into()),
    };
    assert_eq!(GitWriter.write(&repo.0, &merge).unwrap(), None);
    let conflict = GitReader
        .load_refs(&repo.0)
        .unwrap()
        .in_progress
        .unwrap()
        .conflicts[0]
        .clone();
    let resolve = Write::Resolve {
        path: "f".into(),
        conflict,
        take: Take::Ours,
    };
    GitWriter.write(&repo.0, &resolve).unwrap();
    let carry_on = Write::Continue {
        operation: Operation::Merge,
        head: at(),
    };
    GitWriter.write(&repo.0, &carry_on).unwrap();
    // A reset back, then a rebase: pre-rebase, post-rewrite, post-checkout.
    let reset = Write::Reset {
        branch: "main".into(),
        tip: at(),
        commit: rev("HEAD~1"),
        mode: ResetMode::Hard,
    };
    GitWriter.write(&repo.0, &reset).unwrap();
    let rebase = Write::Rebase {
        branch: "main".into(),
        tip: at(),
        onto: rev("side"),
    };
    assert_eq!(GitWriter.write(&repo.0, &rebase).unwrap(), None);
    let abort = Write::Abort {
        operation: Operation::Rebase,
        head: at(),
    };
    GitWriter.write(&repo.0, &abort).unwrap();
    // A cherry-pick, which stops on the same conflict.
    let pick = Write::CherryPick {
        branch: "main".into(),
        tip: at(),
        commit: rev("side~0"),
    };
    GitWriter.write(&repo.0, &pick).unwrap();
    let ran = markers.ran();
    assert!(ran.is_empty(), "programs ran: {ran:?}");
}

#[test]
// Verifies: ADR-0033
fn a_conflicted_link_is_resolved_as_git_resolves_it_and_nothing_outside_is_written() {
    let s = Scratch::new("link");
    let d = &s.0;
    let outside = Scratch::new("link-outside");
    write_file(&outside.0, "kept", "precious\n");
    git(d, &["init", "--quiet", "-b", "main"]);
    git(d, &["config", "user.name", "Ada"]);
    git(d, &["config", "user.email", "ada@example.invalid"]);
    write_file(d, "f", "base\n");
    git(d, &["add", "f"]);
    git(d, &["commit", "--quiet", "-m", "base"]);
    write_file(&outside.0, "other", "theirs\n");
    git(d, &["switch", "--quiet", "-c", "side"]);
    std::fs::remove_file(d.join("f")).unwrap();
    std::os::unix::fs::symlink(outside.0.join("other"), d.join("f")).unwrap();
    git(d, &["add", "f"]);
    git(d, &["commit", "--quiet", "-m", "theirs, a link"]);
    git(d, &["switch", "--quiet", "main"]);
    std::fs::remove_file(d.join("f")).unwrap();
    std::os::unix::fs::symlink(outside.0.join("kept"), d.join("f")).unwrap();
    git(d, &["add", "f"]);
    git(d, &["commit", "--quiet", "-m", "ours, a link"]);
    assert!(!git_may_fail(d, &["merge", "side"]));
    let other = twin(&s, "link-twin");
    GitWriter.write(d, &resolve(d, "f", Take::Theirs)).unwrap();
    git(&other.0, &["checkout", "--theirs", "f"]);
    git(&other.0, &["add", "f"]);
    assert_eq!(state(d), state(&other.0));
    assert_eq!(
        std::fs::read_to_string(outside.0.join("kept")).unwrap(),
        "precious\n",
        "the link's target is untouched"
    );
    assert_eq!(
        std::fs::read_link(d.join("f")).unwrap(),
        outside.0.join("other"),
        "theirs, a link, written as a link"
    );

    // The abort puts ours back, again a link.
    let abort = Write::Abort {
        operation: Operation::Merge,
        head: head(d),
    };
    GitWriter.write(d, &abort).unwrap();
    git(&other.0, &["merge", "--abort"]);
    assert_eq!(state(d), state(&other.0));
    assert!(
        d.join("f")
            .symlink_metadata()
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        std::fs::read_to_string(outside.0.join("kept")).unwrap(),
        "precious\n"
    );
}

#[test]
// Verifies: ADR-0033
fn a_sequence_or_an_autostash_git_holds_is_left_to_git() {
    let repo = diverged("sequence", "ours\n", "theirs\n");
    git(&repo.0, &["switch", "--quiet", "side"]);
    write_file(&repo.0, "s", "more\n");
    git(&repo.0, &["commit", "--quiet", "-am", "more"]);
    git(&repo.0, &["switch", "--quiet", "main"]);
    assert!(!git_may_fail(&repo.0, &["cherry-pick", "side~1", "side"]));
    GitWriter
        .write(&repo.0, &resolve(&repo.0, "f", Take::Theirs))
        .unwrap();
    let before = state(&repo.0);
    for write in [
        Write::Continue {
            operation: Operation::CherryPick,
            head: head(&repo.0),
        },
        Write::Abort {
            operation: Operation::CherryPick,
            head: head(&repo.0),
        },
    ] {
        let error = GitWriter.write(&repo.0, &write).unwrap_err();
        assert!(
            error.to_string().contains("sequence git started"),
            "{error}"
        );
    }
    assert_eq!(state(&repo.0), before);

    let repo = diverged("autostash", "ours\n", "theirs\n");
    write_file(&repo.0, "g", "precious work\n");
    assert!(!git_may_fail(&repo.0, &["rebase", "--autostash", "side"]));
    let abort = Write::Abort {
        operation: Operation::Rebase,
        head: head(&repo.0),
    };
    let error = GitWriter.write(&repo.0, &abort).unwrap_err();
    assert!(error.to_string().contains("autostash"), "{error}");
    git(&repo.0, &["rebase", "--abort"]);
    assert_eq!(
        std::fs::read_to_string(repo.0.join("g")).unwrap(),
        "precious work\n"
    );
}

#[test]
// Verifies: ADR-0033
fn a_rebase_whose_step_meets_a_file_in_its_way_is_undone_whole() {
    let repo = diverged("rebase-in-the-way", "ours\n", "base\n");
    write_file(&repo.0, "new", "on main\n");
    git(&repo.0, &["add", "new"]);
    git(&repo.0, &["commit", "--quiet", "-m", "add new"]);
    git(&repo.0, &["switch", "--quiet", "side"]);
    git(&repo.0, &["rm", "--quiet", "s"]);
    git(&repo.0, &["commit", "--quiet", "-m", "drop s"]);
    git(&repo.0, &["switch", "--quiet", "main"]);
    // Untracked, and in the way of the step that adds it on the new base.
    git(&repo.0, &["rm", "--quiet", "--cached", "new"]);
    git(&repo.0, &["commit", "--quiet", "-m", "untrack new"]);
    let before = state(&repo.0);
    let rebase = Write::Rebase {
        branch: "main".into(),
        tip: head(&repo.0),
        onto: git(&repo.0, &["rev-parse", "side"]),
    };
    match GitWriter.write(&repo.0, &rebase) {
        Ok(Some(_)) => {} // libgit2 found nothing in the way: the rebase went through.
        Ok(None) => panic!("a step neither applied nor refused"),
        Err(error) => {
            assert!(error.to_string().contains("in the way"), "{error}");
            assert_eq!(state(&repo.0), before, "undone whole");
        }
    }
}

#[test]
// Verifies: ADR-0033, G-5
fn messages_and_merge_ff_follow_git() {
    // A cherry-pick keeps a line that starts with '#'.
    let repo = diverged("hash-line", "ours\n", "base\n");
    git(&repo.0, &["switch", "--quiet", "side"]);
    write_file(&repo.0, "t", "issue\n");
    git(&repo.0, &["add", "t"]);
    git(
        &repo.0,
        &["commit", "--quiet", "-m", "fix it\n\n#123 was the issue"],
    );
    git(&repo.0, &["switch", "--quiet", "main"]);
    let pick = Write::CherryPick {
        branch: "main".into(),
        tip: head(&repo.0),
        commit: git(&repo.0, &["rev-parse", "side"]),
    };
    GitWriter.write(&repo.0, &pick).unwrap();
    assert!(git(&repo.0, &["log", "-1", "--format=%B"]).contains("#123 was the issue"));

    // merge.ff = no makes a merge commit where a fast-forward was possible.
    let repo = diverged("merge-ff-no", "base\n", "theirs\n");
    git(&repo.0, &["reset", "--quiet", "--hard", "HEAD~1"]);
    git(&repo.0, &["config", "merge.ff", "no"]);
    GitWriter
        .write(&repo.0, &merge(&repo.0, "side", Some("side")))
        .unwrap();
    assert_eq!(
        git(&repo.0, &["log", "-1", "--format=%P"])
            .split(' ')
            .count(),
        2
    );

    // A remote-tracking branch is named as git names it.
    let repo = diverged("merge-remote", "ours\n", "base\n");
    git(&repo.0, &["update-ref", "refs/remotes/origin/side", "side"]);
    GitWriter
        .write(&repo.0, &merge(&repo.0, "origin/side", Some("origin/side")))
        .unwrap();
    assert_eq!(
        git(&repo.0, &["log", "-1", "--format=%s"]),
        "Merge remote-tracking branch 'origin/side'"
    );
}

#[test]
// Verifies: ADR-0033
fn a_cherry_pick_already_on_the_branch_is_refused_with_nothing_changed() {
    let repo = diverged("empty-pick", "ours\n", "base\n");
    git(&repo.0, &["cherry-pick", "side"]);
    let before = state(&repo.0);
    let pick = Write::CherryPick {
        branch: "main".into(),
        tip: head(&repo.0),
        commit: git(&repo.0, &["rev-parse", "side"]),
    };
    let error = GitWriter.write(&repo.0, &pick).unwrap_err();
    assert!(error.to_string().contains("already"), "{error}");
    assert_eq!(state(&repo.0), before);
    assert_eq!(GitReader.load_refs(&repo.0).unwrap().in_progress, None);
}

/// A repository stopped in a merge whose only conflict is on `path`; `extra` files, the same on
/// both sides, come along.
fn conflicted_at(name: &str, path: &str, extra: &[&str]) -> Scratch {
    let s = Scratch::new(name);
    let d = &s.0;
    git(d, &["init", "--quiet", "-b", "main"]);
    git(d, &["config", "user.name", "Ada"]);
    git(d, &["config", "user.email", "ada@example.invalid"]);
    if let Some(parent) = Path::new(path).parent() {
        std::fs::create_dir_all(d.join(parent)).unwrap();
    }
    write_file(d, path, "base\n");
    for e in extra {
        write_file(d, e, "kept\n");
    }
    git(d, &["add", "."]);
    git(d, &["commit", "--quiet", "-m", "base"]);
    git(d, &["switch", "--quiet", "-c", "side"]);
    write_file(d, path, "theirs\n");
    git(d, &["commit", "--quiet", "-am", "theirs"]);
    git(d, &["switch", "--quiet", "main"]);
    write_file(d, path, "ours\n");
    git(d, &["commit", "--quiet", "-am", "ours"]);
    assert!(!git_may_fail(d, &["merge", "side"]));
    s
}

fn abort(dir: &Path, operation: Operation) -> Write {
    Write::Abort {
        operation,
        head: head(dir),
    }
}

#[test]
// Verifies: ADR-0033, G-6
fn a_path_is_taken_as_it_is_written_never_as_a_pattern() {
    // `x[ab].txt` as a pattern matches `xa.txt`: neither its edit nor an untracked copy is touched.
    let repo = conflicted_at("pattern-resolve", "x[ab].txt", &["xa.txt"]);
    let d = &repo.0;
    write_file(d, "xa.txt", "an edit of the developer's\n");
    GitWriter
        .write(d, &resolve(d, "x[ab].txt", Take::Ours))
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(d.join("xa.txt")).unwrap(),
        "an edit of the developer's\n"
    );
    assert_eq!(
        std::fs::read_to_string(d.join("x[ab].txt")).unwrap(),
        "ours\n"
    );

    let repo = conflicted_at("pattern-abort", "x[ab].txt", &[]);
    let d = &repo.0;
    write_file(d, "xb.txt", "untracked, the developer's\n");
    GitWriter.write(d, &abort(d, Operation::Merge)).unwrap();
    assert_eq!(
        std::fs::read_to_string(d.join("xb.txt")).unwrap(),
        "untracked, the developer's\n"
    );
}

#[test]
// Verifies: ADR-0033, G-6
fn a_path_below_a_link_or_a_file_is_refused_and_nothing_outside_is_written() {
    let outside = Scratch::new("below-outside");
    let repo = conflicted_at("below-link", "dir/f", &[]);
    let d = &repo.0;
    std::fs::remove_dir_all(d.join("dir")).unwrap();
    std::os::unix::fs::symlink(&outside.0, d.join("dir")).unwrap();
    let before = state(d);
    for write in [
        resolve(d, "dir/f", Take::Theirs),
        abort(d, Operation::Merge),
    ] {
        let refused = GitWriter.write(d, &write).unwrap_err().to_string();
        assert!(refused.contains("dir is a link"), "{refused}");
        assert_eq!(state(d), before);
    }
    assert!(!outside.0.join("f").exists());

    let repo = conflicted_at("below-file", "dir/f", &[]);
    let d = &repo.0;
    std::fs::remove_dir_all(d.join("dir")).unwrap();
    write_file(d, "dir", "a file of the developer's\n");
    let before = state(d);
    let refused = GitWriter
        .write(d, &resolve(d, "dir/f", Take::Theirs))
        .unwrap_err()
        .to_string();
    assert!(refused.contains("dir is a file"), "{refused}");
    assert_eq!(state(d), before);
}

#[test]
// Verifies: ADR-0033, Q-6
fn an_abort_keeps_an_edit_made_while_stopped_as_git_does() {
    let repo = diverged("abort-edit", "ours\n", "theirs\n");
    let d = &repo.0;
    assert!(!git_may_fail(d, &["merge", "side"]));
    // `s` came in cleanly; the developer edits it while the merge is stopped.
    write_file(d, "s", "an edit of the developer's\n");
    let other = twin(&repo, "abort-edit-twin");
    let refused = GitWriter
        .write(d, &abort(d, Operation::Merge))
        .unwrap_err()
        .to_string();
    assert!(refused.contains("s was changed"), "{refused}");
    assert!(!git_may_fail(&other.0, &["merge", "--abort"]));
    assert_eq!(state(d), state(&other.0));
}

#[test]
// Verifies: ADR-0033, G-6
fn a_rebase_step_resolved_to_nothing_is_dropped_as_git_drops_it() {
    let s = Scratch::new("rebase-empty");
    let d = &s.0;
    git(d, &["init", "--quiet", "-b", "main"]);
    git(d, &["config", "user.name", "Ada"]);
    git(d, &["config", "user.email", "ada@example.invalid"]);
    write_file(d, "f", "base\n");
    git(d, &["add", "."]);
    git(d, &["commit", "--quiet", "-m", "base"]);
    git(d, &["switch", "--quiet", "-c", "side"]);
    write_file(d, "f", "theirs\n");
    git(d, &["commit", "--quiet", "-am", "theirs"]);
    git(d, &["switch", "--quiet", "main"]);
    write_file(d, "f", "ours\n");
    git(d, &["commit", "--quiet", "-am", "ours"]);
    write_file(d, "h", "h\n");
    git(d, &["add", "h"]);
    git(d, &["commit", "--quiet", "-m", "more"]);
    let other = twin(&s, "rebase-empty-twin");
    let rebase = Write::Rebase {
        branch: "main".into(),
        tip: head(d),
        onto: git(d, &["rev-parse", "side"]),
    };
    GitWriter.write(d, &rebase).unwrap();
    // In a rebase the upstream is ours: taking it leaves the step nothing to change.
    GitWriter.write(d, &resolve(d, "f", Take::Ours)).unwrap();
    let go_on = Write::Continue {
        operation: Operation::Rebase,
        head: head(d),
    };
    GitWriter.write(d, &go_on).unwrap();
    assert!(!git_may_fail(&other.0, &["rebase", "side"]));
    git(&other.0, &["checkout", "--ours", "f"]);
    git(&other.0, &["add", "f"]);
    assert!(git_may_fail(
        &other.0,
        &["-c", "core.editor=true", "rebase", "--continue"]
    ));
    let subjects = |dir: &Path| git(dir, &["log", "--format=%s"]);
    assert_eq!(subjects(d), subjects(&other.0));
    assert_eq!(tree(d), tree(&other.0));
    assert!(GitReader.load_refs(d).unwrap().in_progress.is_none());
}

#[test]
// Verifies: ADR-0033, G-5
fn a_merge_into_another_branch_is_named_as_git_names_it() {
    for suppress in [None, Some("fe*")] {
        let repo = diverged("merge-into", "ours\n", "base\n");
        let d = &repo.0;
        git(d, &["switch", "--quiet", "-c", "feat"]);
        if let Some(pattern) = suppress {
            git(d, &["config", "merge.suppressDest", pattern]);
        }
        let other = twin(&repo, "merge-into-twin");
        let write = Write::Merge {
            branch: "feat".into(),
            tip: head(d),
            commit: git(d, &["rev-parse", "side"]),
            name: Some("side".into()),
        };
        GitWriter.write(d, &write).unwrap();
        git(&other.0, &["merge", "--quiet", "--no-edit", "side"]);
        let subject = |dir: &Path| git(dir, &["log", "-1", "--format=%s"]);
        assert_eq!(subject(d), subject(&other.0), "{suppress:?}");
    }
}

/// A repository with `main` and `side` from one base: `base`, `ours`, and `theirs` each run on
/// the repository at that point, each step followed by a commit of everything.
fn branched(
    name: &str,
    base: &dyn Fn(&Path),
    ours: &dyn Fn(&Path),
    theirs: &dyn Fn(&Path),
) -> Scratch {
    let s = Scratch::new(name);
    let d = &s.0;
    git(d, &["init", "--quiet", "-b", "main"]);
    git(d, &["config", "user.name", "Ada"]);
    git(d, &["config", "user.email", "ada@example.invalid"]);
    base(d);
    git(d, &["add", "--all"]);
    git(d, &["commit", "--quiet", "-m", "base"]);
    git(d, &["switch", "--quiet", "-c", "side"]);
    theirs(d);
    git(d, &["add", "--all"]);
    git(d, &["commit", "--quiet", "-m", "theirs"]);
    git(d, &["switch", "--quiet", "main"]);
    ours(d);
    git(d, &["add", "--all"]);
    git(d, &["commit", "--quiet", "-m", "ours"]);
    s
}

#[test]
// Verifies: ADR-0033, Q-6
fn an_abort_keeps_a_file_made_where_the_operation_removed_one_as_git_does() {
    let repo = branched(
        "abort-made",
        &|d| {
            write_file(d, "f", "base\n");
            write_file(d, "gone", "base\n");
        },
        &|d| write_file(d, "f", "ours\n"),
        &|d| {
            write_file(d, "f", "theirs\n");
            std::fs::remove_file(d.join("gone")).unwrap();
        },
    );
    let d = &repo.0;
    assert!(!git_may_fail(d, &["merge", "side"]));
    write_file(d, "gone", "made by the developer\n");
    let other = twin(&repo, "abort-made-twin");
    git_may_fail(&other.0, &["update-index", "--refresh"]);
    let refused = GitWriter
        .write(d, &abort(d, Operation::Merge))
        .unwrap_err()
        .to_string();
    assert!(refused.contains("gone was changed"), "{refused}");
    assert!(!git_may_fail(&other.0, &["merge", "--abort"]));
    assert_eq!(
        std::fs::read_to_string(d.join("gone")).unwrap(),
        "made by the developer\n"
    );
}

#[test]
// Verifies: ADR-0033, G-6
fn an_abort_puts_back_a_file_and_a_directory_the_operation_swapped_as_git_does() {
    let file_to_dir = |d: &Path| {
        std::fs::remove_file(d.join("d")).unwrap();
        std::fs::create_dir(d.join("d")).unwrap();
        write_file(d, "d/x", "x\n");
    };
    let dir_to_file = |d: &Path| {
        std::fs::remove_dir_all(d.join("d")).unwrap();
        write_file(d, "d", "a file\n");
    };
    for (name, base, swap) in [
        ("swap-file", true, &file_to_dir as &dyn Fn(&Path)),
        ("swap-dir", false, &dir_to_file as &dyn Fn(&Path)),
    ] {
        let repo = branched(
            name,
            &|d| {
                write_file(d, "f", "base\n");
                if base {
                    write_file(d, "d", "a file\n");
                } else {
                    std::fs::create_dir(d.join("d")).unwrap();
                    write_file(d, "d/x", "x\n");
                }
            },
            &|d| write_file(d, "f", "ours\n"),
            &|d| {
                write_file(d, "f", "theirs\n");
                swap(d);
            },
        );
        let d = &repo.0;
        assert!(!git_may_fail(d, &["merge", "side"]));
        let other = twin(&repo, &format!("{name}-twin"));
        git_may_fail(&other.0, &["update-index", "--refresh"]);
        GitWriter.write(d, &abort(d, Operation::Merge)).unwrap();
        git(&other.0, &["merge", "--abort"]);
        assert_eq!(state(d), state(&other.0), "{name}");
    }
}

#[test]
// Verifies: ADR-0033, Q-6
fn a_rebase_step_that_could_not_be_applied_is_never_dropped() {
    let repo = branched(
        "rebase-unapplied",
        &|d| write_file(d, "f", "base\n"),
        &|d| write_file(d, "f", "ours\n"),
        &|d| write_file(d, "f", "theirs\n"),
    );
    let d = &repo.0;
    write_file(d, "n", "n\n");
    git(d, &["add", "n"]);
    git(d, &["commit", "--quiet", "-m", "add n"]);
    let before = head(d);
    let rebase = Write::Rebase {
        branch: "main".into(),
        tip: head(d),
        onto: git(d, &["rev-parse", "side"]),
    };
    GitWriter.write(d, &rebase).unwrap();
    GitWriter.write(d, &resolve(d, "f", Take::Theirs)).unwrap();
    // An untracked file stands where the next step adds one.
    write_file(d, "n", "the developer's\n");
    let go_on = || Write::Continue {
        operation: Operation::Rebase,
        head: head(d),
    };
    assert!(GitWriter.write(d, &go_on()).is_err());
    std::fs::remove_file(d.join("n")).unwrap();
    let refused = GitWriter.write(d, &go_on()).unwrap_err().to_string();
    assert!(refused.contains("could not be applied"), "{refused}");
    GitWriter.write(d, &abort(d, Operation::Rebase)).unwrap();
    assert_eq!(head(d), before);
    assert_eq!(git(d, &["log", "-1", "--format=%s"]), "add n");
}

#[test]
// Verifies: ADR-0033, G-5
fn merge_suppress_dest_matches_as_git_matches_it() {
    for (branch, pattern) in [
        ("fe/x", "fe*"),
        ("fe/x", "fe**"),
        ("fe/x", "fe/**"),
        ("rel-1", "rel-[0-9]"),
        ("ab", "a[b]"),
    ] {
        let repo = diverged("suppress", "ours\n", "base\n");
        let d = &repo.0;
        git(d, &["switch", "--quiet", "-c", branch]);
        git(d, &["config", "merge.suppressDest", pattern]);
        let other = twin(&repo, "suppress-twin");
        let write = Write::Merge {
            branch: branch.into(),
            tip: head(d),
            commit: git(d, &["rev-parse", "side"]),
            name: Some("side".into()),
        };
        GitWriter.write(d, &write).unwrap();
        git(&other.0, &["merge", "--quiet", "--no-edit", "side"]);
        let subject = |dir: &Path| git(dir, &["log", "-1", "--format=%s"]);
        assert_eq!(subject(d), subject(&other.0), "{branch} {pattern}");
    }
}

#[test]
// Verifies: ADR-0033, Q-6
fn an_abort_never_writes_through_a_link_the_operation_put_in_a_directorys_place() {
    let outside = Scratch::new("swap-link-outside");
    write_file(&outside.0, "x", "outside\n");
    let target = outside.0.clone();
    let repo = branched(
        "swap-link",
        &|d| {
            write_file(d, "f", "base\n");
            std::fs::create_dir(d.join("d")).unwrap();
            write_file(d, "d/x", "x\n");
        },
        &|d| write_file(d, "f", "ours\n"),
        &|d| {
            write_file(d, "f", "theirs\n");
            std::fs::remove_dir_all(d.join("d")).unwrap();
            std::os::unix::fs::symlink(&target, d.join("d")).unwrap();
        },
    );
    let d = &repo.0;
    assert!(!git_may_fail(d, &["merge", "side"]));
    let other = twin(&repo, "swap-link-twin");
    git_may_fail(&other.0, &["update-index", "--refresh"]);
    GitWriter.write(d, &abort(d, Operation::Merge)).unwrap();
    assert_eq!(
        std::fs::read_to_string(outside.0.join("x")).unwrap(),
        "outside\n"
    );
    git(&other.0, &["merge", "--abort"]);
    assert_eq!(state(d), state(&other.0));
}

#[test]
// Verifies: ADR-0033, Q-6
fn an_abort_keeps_a_directory_made_where_the_operation_removed_a_file_as_git_does() {
    let repo = branched(
        "abort-dir",
        &|d| {
            write_file(d, "f", "base\n");
            write_file(d, "d", "a file\n");
        },
        &|d| write_file(d, "f", "ours\n"),
        &|d| {
            write_file(d, "f", "theirs\n");
            std::fs::remove_file(d.join("d")).unwrap();
        },
    );
    let d = &repo.0;
    assert!(!git_may_fail(d, &["merge", "side"]));
    std::fs::create_dir(d.join("d")).unwrap();
    write_file(d, "d/new", "the developer's\n");
    let other = twin(&repo, "abort-dir-twin");
    git_may_fail(&other.0, &["update-index", "--refresh"]);
    let refused = GitWriter
        .write(d, &abort(d, Operation::Merge))
        .unwrap_err()
        .to_string();
    assert!(refused.contains("d was changed"), "{refused}");
    assert!(!git_may_fail(&other.0, &["merge", "--abort"]));
    assert_eq!(
        std::fs::read_to_string(d.join("d/new")).unwrap(),
        "the developer's\n"
    );
}

/// A repository whose `m` is a submodule, `main` and `side` from one base; `theirs` runs on
/// `side` before its commit.
fn with_submodule(name: &str, theirs: &dyn Fn(&Path, &Path)) -> (Scratch, Scratch) {
    let sub = Scratch::new(&format!("{name}-sub"));
    git(&sub.0, &["init", "--quiet", "-b", "main"]);
    git(&sub.0, &["config", "user.name", "Ada"]);
    git(&sub.0, &["config", "user.email", "ada@example.invalid"]);
    write_file(&sub.0, "s", "one\n");
    git(&sub.0, &["add", "s"]);
    git(&sub.0, &["commit", "--quiet", "-m", "one"]);
    write_file(&sub.0, "s", "two\n");
    git(&sub.0, &["commit", "--quiet", "-am", "two"]);
    let s = Scratch::new(name);
    let d = &s.0;
    git(d, &["init", "--quiet", "-b", "main"]);
    git(d, &["config", "user.name", "Ada"]);
    git(d, &["config", "user.email", "ada@example.invalid"]);
    write_file(d, "f", "base\n");
    git(
        d,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "--quiet",
            sub.0.to_str().unwrap(),
            "m",
        ],
    );
    git(&d.join("m"), &["checkout", "--quiet", "HEAD~1"]);
    git(d, &["add", "--all"]);
    git(d, &["commit", "--quiet", "-m", "base"]);
    git(d, &["switch", "--quiet", "-c", "side"]);
    write_file(d, "f", "theirs\n");
    theirs(d, &sub.0);
    git(d, &["add", "--all"]);
    git(d, &["commit", "--quiet", "-m", "theirs"]);
    git(d, &["switch", "--quiet", "main"]);
    git(d, &["submodule", "--quiet", "update", "--init"]);
    write_file(d, "f", "ours\n");
    git(d, &["commit", "--quiet", "-am", "ours"]);
    (s, sub)
}

#[test]
// Verifies: ADR-0033, G-6
fn an_abort_over_a_submodule_moved_or_replaced_ends_as_git_does_or_keeps_its_conflicts() {
    // Theirs moves the submodule on: the abort pins it back, as git's does.
    let (repo, _sub) = with_submodule("sub-bump", &|d, _| {
        git(&d.join("m"), &["checkout", "--quiet", "main"]);
    });
    let d = &repo.0;
    assert!(!git_may_fail(d, &["merge", "side"]));
    let other = twin(&repo, "sub-bump-twin");
    git_may_fail(&other.0, &["update-index", "--refresh"]);
    GitWriter.write(d, &abort(d, Operation::Merge)).unwrap();
    git(&other.0, &["merge", "--abort"]);
    assert_eq!(state(d), state(&other.0));

    // Theirs puts a file in its place: an abort that cannot finish keeps the merge stopped.
    let (repo, _sub) = with_submodule("sub-file", &|d, _| {
        git(d, &["rm", "--quiet", "--cached", "m"]);
        std::fs::remove_dir_all(d.join("m")).unwrap();
        write_file(d, "m", "a file\n");
    });
    let d = &repo.0;
    assert!(!git_may_fail(d, &["merge", "side"]));
    let before = git(d, &["ls-files", "--stage"]);
    if let Err(refused) = GitWriter.write(d, &abort(d, Operation::Merge)) {
        assert!(refused.to_string().contains("conflicts kept"), "{refused}");
        assert_eq!(git(d, &["ls-files", "--stage"]), before);
        assert!(GitReader.load_refs(d).unwrap().in_progress.is_some());
    }
}

#[test]
// Verifies: ADR-0033, Q-6
fn a_directory_made_in_a_conflicted_files_place_is_neither_aborted_nor_resolved_away() {
    let repo = conflicted_at("conflict-dir", "f", &[]);
    let d = &repo.0;
    std::fs::remove_file(d.join("f")).unwrap();
    std::fs::create_dir(d.join("f")).unwrap();
    write_file(d, "f/mine", "the developer's\n");
    let other = twin(&repo, "conflict-dir-twin");
    git_may_fail(&other.0, &["update-index", "--refresh"]);
    let before = state(d);
    for write in [abort(d, Operation::Merge), resolve(d, "f", Take::Theirs)] {
        assert!(GitWriter.write(d, &write).is_err());
        assert_eq!(state(d), before);
    }
    assert!(!git_may_fail(&other.0, &["merge", "--abort"]));
    assert_eq!(
        std::fs::read_to_string(d.join("f/mine")).unwrap(),
        "the developer's\n"
    );
}
