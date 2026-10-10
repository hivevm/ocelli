//! Verifies: ADR-0033, G-5, G-6
//!
//! Merge, rebase, cherry-pick, revert, and reset, and the resolution of the conflicts they stop
//! on, on repositories made with git: each compared with git doing the same on a twin of the
//! repository, and each refusal leaving the repository as it was.

#![allow(
    clippy::disallowed_methods,
    reason = "a test sets a repository up by hand, the way a developer would"
)]
#![cfg(unix)]

mod support;
use ocelli_core::model::Operation;
use ocelli_core::port::{RepositoryReader, RepositoryWriter, ResetMode, Take, Write};
use ocelli_git::{GitReader, GitWriter};
use std::path::Path;
use support::{
    Origin, Scratch, Snapshot, assert_twins_alike, git, git_command, git_may_fail, refused,
    snapshot, stopped_partway, twin, write_file,
};

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

fn head(dir: &Path) -> String {
    git(dir, &["rev-parse", "HEAD"])
}

fn tree(dir: &Path) -> String {
    git(dir, &["rev-parse", "HEAD^{tree}"])
}

// Where Ocelli is known to differ from git without an ADR, by the files of an operation it
// leaves otherwise; each is asserted to differ still, so that a fix shows up as a failure that
// removes it.

/// Expects the repository at `dir` back where `before` was once an operation stopped and was
/// aborted, but for the files of the operation, which `git` leaves too: a twin compares them with
/// what `git` leaves.
fn gone_back(dir: &Path, before: &Snapshot, what: &str) {
    let mut after = snapshot(dir);
    after.in_progress.clone_from(&before.in_progress);
    assert_eq!(&after, before, "{what}");
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
    refused(&repo.0, &merge(&repo.0, "side", None), "merge.ff is only");
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
    refused(
        &repo.0,
        &merge(&repo.0, "side", None),
        "uncommitted changes to g",
    );

    let repo = diverged("refused-busy", "ours\n", "theirs\n");
    assert!(!git_may_fail(&repo.0, &["merge", "side"]));
    let pick = Write::CherryPick {
        branch: "main".into(),
        tip: head(&repo.0),
        commit: git(&repo.0, &["rev-parse", "side"]),
    };
    refused(&repo.0, &pick, "a merge is in progress");
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
        refused(&repo.0, &write, "merge commit");
    }
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
        let origin = Origin::of(&repo.0);
        let other = twin(&repo, &format!("reset{flag}-twin"));
        let reset = Write::Reset {
            branch: "main".into(),
            tip: head(&repo.0),
            commit: git(&repo.0, &["rev-parse", "HEAD~1"]),
            mode,
        };
        GitWriter.write(&repo.0, &reset).unwrap();
        git(&other.0, &["reset", "--quiet", flag, "HEAD~1"]);
        assert_twins_alike(&repo.0, &other.0, &origin, flag);
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
        let origin = Origin::of(&repo.0);
        let other = twin(&repo, &format!("resolve{side}-twin"));
        GitWriter
            .write(&repo.0, &resolve(&repo.0, "f", take))
            .unwrap();
        git(&other.0, &["checkout", side, "f"]);
        git(&other.0, &["add", "f"]);
        assert_twins_alike(&repo.0, &other.0, &origin, side);

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
    refused(
        &repo.0,
        &resolve(&repo.0, "f", Take::AsEdited),
        "conflict marker",
    );
    let continue_now = Write::Continue {
        operation: Operation::Merge,
        head: head(&repo.0),
    };
    refused(&repo.0, &continue_now, "still conflict");

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
        let before = snapshot(&repo.0);
        let origin = Origin::of(&repo.0);
        let other = twin(&repo, &format!("abort-{operation}-twin"));
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
        gone_back(&repo.0, &before, &format!("{operation} --abort"));
        assert!(!git_may_fail(&other.0, &[operation, "side"]));
        git(&other.0, &[operation, "--abort"]);
        assert_twins_alike(&repo.0, &other.0, &origin, operation);
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
    let before = snapshot(&repo.0);
    let origin = Origin::of(&repo.0);
    let other = twin(&repo, "rebase-abort-twin");
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
    gone_back(&repo.0, &before, "aborted");
    assert!(!git_may_fail(&other.0, &["rebase", "side"]));
    git(&other.0, &["rebase", "--abort"]);
    assert_twins_alike(&repo.0, &other.0, &origin, "aborted");
}

#[test]
// Verifies: ADR-0033
fn a_rebase_either_side_started_is_aborted_by_the_other_and_continued_where_it_started() {
    // git starts it: Ocelli refuses to continue it and aborts it as git would.
    let repo = diverged("git-rebase", "ours\n", "theirs\n");
    let before = snapshot(&repo.0);
    let origin = Origin::of(&repo.0);
    let other = twin(&repo, "git-rebase-twin");
    assert!(!git_may_fail(&repo.0, &["rebase", "side"]));
    GitWriter
        .write(&repo.0, &resolve(&repo.0, "f", Take::Ours))
        .unwrap();
    let carry_on = Write::Continue {
        operation: Operation::Rebase,
        head: head(&repo.0),
    };
    refused(&repo.0, &carry_on, "started by git");
    let abort = Write::Abort {
        operation: Operation::Rebase,
        head: head(&repo.0),
    };
    GitWriter.write(&repo.0, &abort).unwrap();
    gone_back(&repo.0, &before, "aborted by Ocelli");
    assert!(!git_may_fail(&other.0, &["rebase", "side"]));
    git(&other.0, &["checkout", "--ours", "f"]);
    git(&other.0, &["add", "f"]);
    git(&other.0, &["rebase", "--abort"]);
    assert_twins_alike(&repo.0, &other.0, &origin, "aborted by Ocelli");

    // Ocelli starts it: git aborts it.
    let repo = diverged("ocelli-rebase", "ours\n", "theirs\n");
    let before = snapshot(&repo.0);
    let origin = Origin::of(&repo.0);
    let other = twin(&repo, "ocelli-rebase-twin");
    let rebase = Write::Rebase {
        branch: "main".into(),
        tip: head(&repo.0),
        onto: git(&repo.0, &["rev-parse", "side"]),
    };
    assert_eq!(GitWriter.write(&repo.0, &rebase).unwrap(), None);
    git(&repo.0, &["rebase", "--abort"]);
    gone_back(&repo.0, &before, "aborted by git");
    assert!(!git_may_fail(&other.0, &["rebase", "side"]));
    git(&other.0, &["rebase", "--abort"]);
    assert_twins_alike(&repo.0, &other.0, &origin, "aborted by git");
}

#[test]
// Verifies: ADR-0033, G-6
fn a_stopped_revert_is_aborted_as_git_revert_abort_does() {
    let repo = diverged("revert-abort", "ours\n", "base\n");
    let reverted = head(&repo.0);
    write_file(&repo.0, "f", "again\n");
    git(&repo.0, &["commit", "--quiet", "-am", "again"]);
    write_file(&repo.0, "untracked", "mine\n");
    let before = snapshot(&repo.0);
    let origin = Origin::of(&repo.0);
    let other = twin(&repo, "revert-abort-twin");
    let revert = Write::Revert {
        branch: "main".into(),
        tip: head(&repo.0),
        commit: reverted.clone(),
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
    gone_back(&repo.0, &before, "aborted");
    assert!(!git_may_fail(&other.0, &["revert", "--no-edit", &reverted]));
    git(&other.0, &["revert", "--abort"]);
    assert_twins_alike(&repo.0, &other.0, &origin, "aborted");
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
    let origin = Origin::of(&s.0);
    let other = twin(&s, "link-twin");
    GitWriter.write(d, &resolve(d, "f", Take::Theirs)).unwrap();
    git(&other.0, &["checkout", "--theirs", "f"]);
    git(&other.0, &["add", "f"]);
    assert_twins_alike(d, &other.0, &origin, "resolved");
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
    assert_twins_alike(d, &other.0, &origin, "aborted");
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
        refused(&repo.0, &write, "sequence git started");
    }

    let repo = diverged("autostash", "ours\n", "theirs\n");
    write_file(&repo.0, "g", "precious work\n");
    assert!(!git_may_fail(&repo.0, &["rebase", "--autostash", "side"]));
    let abort = Write::Abort {
        operation: Operation::Rebase,
        head: head(&repo.0),
    };
    refused(&repo.0, &abort, "autostash");
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
    let rebase = Write::Rebase {
        branch: "main".into(),
        tip: head(&repo.0),
        onto: git(&repo.0, &["rev-parse", "side"]),
    };
    refused(&repo.0, &rebase, "in the way");
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
    let pick = Write::CherryPick {
        branch: "main".into(),
        tip: head(&repo.0),
        commit: git(&repo.0, &["rev-parse", "side"]),
    };
    refused(&repo.0, &pick, "already");
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
    for write in [
        resolve(d, "dir/f", Take::Theirs),
        abort(d, Operation::Merge),
    ] {
        refused(d, &write, "dir is a link");
    }
    assert!(!outside.0.join("f").exists());

    let repo = conflicted_at("below-file", "dir/f", &[]);
    let d = &repo.0;
    std::fs::remove_dir_all(d.join("dir")).unwrap();
    write_file(d, "dir", "a file of the developer's\n");
    refused(d, &resolve(d, "dir/f", Take::Theirs), "dir is a file");
}

#[test]
// Verifies: ADR-0033, Q-6
fn an_abort_keeps_an_edit_made_while_stopped_as_git_does() {
    let repo = diverged("abort-edit", "ours\n", "theirs\n");
    let d = &repo.0;
    assert!(!git_may_fail(d, &["merge", "side"]));
    // `s` came in cleanly; the developer edits it while the merge is stopped.
    write_file(d, "s", "an edit of the developer's\n");
    let origin = Origin::of(&repo.0);
    let other = twin(&repo, "abort-edit-twin");
    refused(d, &abort(d, Operation::Merge), "s was changed");
    assert!(!git_may_fail(&other.0, &["merge", "--abort"]));
    assert_twins_alike(d, &other.0, &origin, "refused alike");
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
    refused(d, &abort(d, Operation::Merge), "gone was changed");
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
        let origin = Origin::of(&repo.0);
        let other = twin(&repo, &format!("{name}-twin"));
        git_may_fail(&other.0, &["update-index", "--refresh"]);
        GitWriter.write(d, &abort(d, Operation::Merge)).unwrap();
        git(&other.0, &["merge", "--abort"]);
        assert_twins_alike(d, &other.0, &origin, name);
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
    stopped_partway(d, &go_on(), "in the way");
    std::fs::remove_file(d.join("n")).unwrap();
    refused(d, &go_on(), "could not be applied");
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
    let origin = Origin::of(&repo.0);
    let other = twin(&repo, "swap-link-twin");
    git_may_fail(&other.0, &["update-index", "--refresh"]);
    GitWriter.write(d, &abort(d, Operation::Merge)).unwrap();
    assert_eq!(
        std::fs::read_to_string(outside.0.join("x")).unwrap(),
        "outside\n"
    );
    git(&other.0, &["merge", "--abort"]);
    assert_twins_alike(d, &other.0, &origin, "aborted");
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
    refused(d, &abort(d, Operation::Merge), "d was changed");
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
    let origin = Origin::of(&repo.0);
    let other = twin(&repo, "sub-bump-twin");
    git_may_fail(&other.0, &["update-index", "--refresh"]);
    GitWriter.write(d, &abort(d, Operation::Merge)).unwrap();
    git(&other.0, &["merge", "--abort"]);
    assert_twins_alike(d, &other.0, &origin, "aborted");

    // Theirs puts a file in its place: the abort puts the submodule back, as git's does.
    let (repo, _sub) = with_submodule("sub-file", &|d, _| {
        git(d, &["rm", "--quiet", "--cached", "m"]);
        std::fs::remove_dir_all(d.join("m")).unwrap();
        write_file(d, "m", "a file\n");
    });
    let d = &repo.0;
    assert!(!git_may_fail(d, &["merge", "side"]));
    let origin = Origin::of(&repo.0);
    let other = twin(&repo, "sub-file-twin");
    git_may_fail(&other.0, &["update-index", "--refresh"]);
    GitWriter.write(d, &abort(d, Operation::Merge)).unwrap();
    git(&other.0, &["merge", "--abort"]);
    assert_twins_alike(
        d,
        &other.0,
        &origin,
        "aborted over a file in the submodule's place",
    );
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
    refused(d, &abort(d, Operation::Merge), "f was changed");
    refused(d, &resolve(d, "f", Take::Theirs), "f is a directory");
    assert!(!git_may_fail(&other.0, &["merge", "--abort"]));
    assert_eq!(
        std::fs::read_to_string(d.join("f/mine")).unwrap(),
        "the developer's\n"
    );
}

#[test]
// Verifies: ADR-0033
fn a_message_is_cleaned_up_as_git_cleans_it_up_for_the_way_it_is_made() {
    let message = "fix it\n\n#123 was the issue\n  trailing   \n";
    // Picked without a conflict, in every mode, and continued after one, where git strips.
    for (mode, conflict) in [
        ("", false),
        ("strip", false),
        ("whitespace", false),
        ("verbatim", false),
        ("", true),
        ("verbatim", true),
        ("whitespace", true),
    ] {
        let repo = diverged("cleanup", "ours\n", "base\n");
        let d = &repo.0;
        if !mode.is_empty() {
            git(d, &["config", "commit.cleanup", mode]);
        }
        git(d, &["switch", "--quiet", "side"]);
        write_file(d, if conflict { "f" } else { "t" }, "picked\n");
        git(d, &["add", "--all"]);
        git(
            d,
            &["commit", "--quiet", "--cleanup=verbatim", "-m", message],
        );
        git(d, &["switch", "--quiet", "main"]);
        let other = twin(&repo, "cleanup-twin");
        let pick = Write::CherryPick {
            branch: "main".into(),
            tip: head(d),
            commit: git(d, &["rev-parse", "side"]),
        };
        GitWriter.write(d, &pick).unwrap();
        let picked = git_may_fail(&other.0, &["cherry-pick", "side"]);
        assert_eq!(picked, !conflict);
        if conflict {
            for dir in [d, &other.0] {
                write_file(dir, "f", "resolved\n");
                git(dir, &["add", "f"]);
            }
            let go_on = Write::Continue {
                operation: Operation::CherryPick,
                head: head(d),
            };
            GitWriter.write(d, &go_on).unwrap();
            git(&other.0, &["cherry-pick", "--continue"]);
        }
        let body = |dir: &Path| {
            let out = git_command(dir)
                .args(["log", "-1", "--format=%B"])
                .output()
                .unwrap();
            String::from_utf8(out.stdout).unwrap()
        };
        assert_eq!(body(d), body(&other.0), "{mode:?} conflict {conflict}");
    }
    // A merge continued, on the same stop for Ocelli and git: comment lines stripped by default,
    // cut at the scissors line under scissors.
    for mode in ["", "strip", "scissors"] {
        let repo = diverged("cleanup-merge", "ours\n", "theirs\n");
        let d = &repo.0;
        if !mode.is_empty() {
            git(d, &["config", "commit.cleanup", mode]);
        }
        GitWriter.write(d, &merge(d, "side", Some("side"))).unwrap();
        let other = twin(&repo, "cleanup-merge-twin");
        for dir in [d, &other.0] {
            write_file(dir, "f", "resolved\n");
            git(dir, &["add", "f"]);
        }
        let go_on = Write::Continue {
            operation: Operation::Merge,
            head: head(d),
        };
        GitWriter.write(d, &go_on).unwrap();
        git(&other.0, &["merge", "--continue"]);
        assert_eq!(raw_body(d), raw_body(&other.0), "{mode:?}");
    }
    // A merge without a conflict, in every mode.
    for mode in ["", "strip", "whitespace", "verbatim", "scissors"] {
        let repo = diverged("cleanup-clean-merge", "ours\n", "base\n");
        let d = &repo.0;
        if !mode.is_empty() {
            git(d, &["config", "commit.cleanup", mode]);
        }
        let other = twin(&repo, "cleanup-clean-merge-twin");
        GitWriter.write(d, &merge(d, "side", Some("side"))).unwrap();
        git(&other.0, &["merge", "--no-edit", "side"]);
        assert_eq!(raw_body(d), raw_body(&other.0), "{mode:?}");
    }
}

/// The last commit's message as `git` stores it, its last line break included.
fn raw_body(dir: &Path) -> String {
    let out = git_command(dir)
        .args(["cat-file", "commit", "HEAD"])
        .output()
        .unwrap();
    let object = String::from_utf8(out.stdout).unwrap();
    object.split_once("\n\n").map_or("", |(_, m)| m).to_string()
}

#[test]
// Verifies: ADR-0033
fn what_only_git_finishes_is_named_alike_by_every_refusal() {
    let repo = diverged("am-in-progress", "ours\n", "theirs\n");
    let d = &repo.0;
    // `git am` stopped: the state git leaves for it.
    std::fs::create_dir(d.join(".git/rebase-apply")).unwrap();
    write_file(d, ".git/rebase-apply/applying", "");
    let refusals = [
        Write::Commit {
            message: "m".into(),
            head: Some(head(d)),
        },
        Write::Detach {
            commit: git(d, &["rev-parse", "side"]),
        },
        merge(d, "side", Some("side")),
        Write::Reset {
            branch: "main".into(),
            tip: head(d),
            commit: git(d, &["rev-parse", "HEAD~1"]),
            mode: ResetMode::Soft,
        },
    ];
    for write in &refusals {
        assert_eq!(
            refused(d, write, "git am"),
            "a git am is in progress: finish it with git first",
            "{write:?}"
        );
    }
    assert_eq!(GitReader.load_refs(d).unwrap().in_progress, None);
}

/// `git`'s conflicted file with the difference ADR-0040 decides taken out of its markers' labels:
/// the subject `git` appends to a commit's label, ` (<subject>)`, dropped, and a `/` in a branch's
/// name flattened to `_`, as the labels the writer gives libgit2 are path-safe. The side's label
/// and the base's, which `git`'s `diff3` and `zdiff3` styles write, alike.
fn without_the_label_departure(text: &str) -> String {
    text.lines()
        .map(|line| {
            for marker in [">>>>>>> ", "||||||| "] {
                if let Some(label) = line.strip_prefix(marker) {
                    let label = match label.find(" (") {
                        Some(at) if label.ends_with(')') => &label[..at],
                        _ => label,
                    };
                    return format!("{marker}{}", label.replace('/', "_"));
                }
            }
            line.to_string()
        })
        .map(|line| format!("{line}\n"))
        .collect()
}

/// Asserts that the conflicted `path` reads in `ours`, stopped by the writer, as in `theirs`,
/// stopped by `git`, but for the label departure; returns the writer's file. libgit2 labels a
/// `diff3` base line only where the file has a base (`merge_file.c`): a file both sides added
/// has a bare `|||||||` line where `git` labels it, another departure of ADR-0040, taken out
/// here only where the writer's base line is bare and `base_unlabelled` says the file has none.
fn markers_as_git_with(
    ours: &Path,
    theirs: &Path,
    path: &str,
    what: &str,
    base_unlabelled: bool,
) -> String {
    let by_ocelli = std::fs::read_to_string(ours.join(path)).unwrap();
    let by_git = std::fs::read_to_string(theirs.join(path)).unwrap();
    assert!(
        by_ocelli.contains("<<<<<<< HEAD\n"),
        "{what}: {by_ocelli:?}"
    );
    let mut expected = without_the_label_departure(&by_git);
    if base_unlabelled {
        expected = expected
            .lines()
            .map(|l| match l.starts_with("||||||| ") {
                true => "|||||||\n".to_string(),
                false => format!("{l}\n"),
            })
            .collect();
    }
    assert_eq!(
        by_ocelli, expected,
        "{what}: the writer's markers against git's {by_git:?}"
    );
    by_ocelli
}

/// [`markers_as_git_with`] for a file with a base, every label compared.
fn markers_as_git(ours: &Path, theirs: &Path, path: &str, what: &str) -> String {
    markers_as_git_with(ours, theirs, path, what, false)
}

/// Sets `merge.conflictStyle` to `style` in `dir`, where one is given.
fn style_of(dir: &Path, style: Option<&str>) {
    if let Some(style) = style {
        git(dir, &["config", "merge.conflictStyle", style]);
    }
}

/// The label the writer's markers give the side applied, read off its conflicted file.
fn their_label(markers: &str) -> &str {
    markers
        .lines()
        .find_map(|l| l.strip_prefix(">>>>>>> "))
        .unwrap()
}

/// The merge, the cherry-pick, the revert, and the rebase, as it starts and on a continue, each
/// stopped by the writer and by `git` on a twin, in the conflict style `style`, `git`'s default
/// where none is given; and the merge's label against the name `git` gives a side's file.
fn stopped_operations_mark_as_git(style: Option<&str>) {
    let tag = style.unwrap_or("merge");
    // A merge of a branch whose name holds a `/`.
    let repo = diverged(&format!("markers-merge-{tag}"), "ours\n", "theirs\n");
    style_of(&repo.0, style);
    git(&repo.0, &["branch", "-m", "side", "feature/x"]);
    let other = twin(&repo, &format!("markers-merge-{tag}-git"));
    let made = GitWriter
        .write(&repo.0, &merge(&repo.0, "feature/x", Some("feature/x")))
        .unwrap();
    assert_eq!(made, None, "the merge stopped");
    assert!(!git_may_fail(
        &other.0,
        &["merge", "--no-edit", "feature/x"]
    ));
    let markers = markers_as_git(&repo.0, &other.0, "f", &format!("merge, {tag}"));
    // Where `git` writes a side to a file of its own, a file meeting a directory, it names it as
    // the writer labels the side: `x~` and the branch, flattened. The writer refuses that
    // conflict before writing anything, a departure of ADR-0033.
    let df = Scratch::new(&format!("markers-df-{tag}"));
    let d = &df.0;
    git(d, &["init", "--quiet", "-b", "main"]);
    git(d, &["config", "user.name", "Ada"]);
    git(d, &["config", "user.email", "ada@example.invalid"]);
    write_file(d, "keep", "base\n");
    git(d, &["add", "keep"]);
    git(d, &["commit", "--quiet", "-m", "base"]);
    git(d, &["switch", "--quiet", "-c", "feature/x"]);
    write_file(d, "x", "file\n");
    git(d, &["add", "x"]);
    git(d, &["commit", "--quiet", "-m", "x a file"]);
    git(d, &["switch", "--quiet", "main"]);
    write_file(d, "x/y", "dir\n");
    git(d, &["add", "x"]);
    git(d, &["commit", "--quiet", "-m", "x a directory"]);
    assert!(!git_may_fail(d, &["merge", "--no-edit", "feature/x"]));
    let side = format!("x~{}", their_label(&markers));
    assert!(d.join(&side).exists(), "git names the side's file {side}");

    // A cherry-pick, its side labelled by the commit, the base `parent of` it.
    let repo = diverged(&format!("markers-pick-{tag}"), "ours\n", "theirs\n");
    style_of(&repo.0, style);
    let other = twin(&repo, &format!("markers-pick-{tag}-git"));
    let pick = Write::CherryPick {
        branch: "main".into(),
        tip: head(&repo.0),
        commit: git(&repo.0, &["rev-parse", "side"]),
    };
    assert_eq!(GitWriter.write(&repo.0, &pick).unwrap(), None);
    assert!(!git_may_fail(&other.0, &["cherry-pick", "side"]));
    markers_as_git(&repo.0, &other.0, "f", &format!("cherry-pick, {tag}"));

    // A revert, its side labelled `parent of` the commit, the base the commit.
    let repo = diverged(&format!("markers-revert-{tag}"), "ours\n", "base\n");
    style_of(&repo.0, style);
    write_file(&repo.0, "f", "again\n");
    git(&repo.0, &["commit", "--quiet", "-am", "again"]);
    let other = twin(&repo, &format!("markers-revert-{tag}-git"));
    let revert = Write::Revert {
        branch: "main".into(),
        tip: head(&repo.0),
        commit: git(&repo.0, &["rev-parse", "HEAD~1"]),
    };
    assert_eq!(GitWriter.write(&repo.0, &revert).unwrap(), None);
    assert!(!git_may_fail(&other.0, &["revert", "--no-edit", "HEAD~1"]));
    markers_as_git(&repo.0, &other.0, "f", &format!("revert, {tag}"));

    // A rebase, stopped as it starts, then again on a continue: each step labelled by its own
    // commit, the rebase opened again with its options for the second.
    let repo = diverged(&format!("markers-rebase-{tag}"), "ours\n", "theirs\n");
    style_of(&repo.0, style);
    write_file(&repo.0, "s", "main's own\n");
    git(&repo.0, &["add", "s"]);
    git(&repo.0, &["commit", "--quiet", "-m", "main adds s"]);
    let other = twin(&repo, &format!("markers-rebase-{tag}-git"));
    let rebase = Write::Rebase {
        branch: "main".into(),
        tip: head(&repo.0),
        onto: git(&repo.0, &["rev-parse", "side"]),
    };
    assert_eq!(GitWriter.write(&repo.0, &rebase).unwrap(), None);
    assert!(!git_may_fail(&other.0, &["rebase", "side"]));
    markers_as_git(
        &repo.0,
        &other.0,
        "f",
        &format!("rebase as it starts, {tag}"),
    );
    for dir in [&repo.0, &other.0] {
        write_file(dir, "f", "resolved\n");
        git(dir, &["add", "f"]);
    }
    let next = Write::Continue {
        operation: Operation::Rebase,
        head: head(&repo.0),
    };
    assert_eq!(
        GitWriter.write(&repo.0, &next).unwrap(),
        None,
        "stopped again"
    );
    assert!(!git_may_fail(&other.0, &["rebase", "--continue"]));
    // `s` both sides added: a file with no base, whose base line libgit2 leaves bare.
    markers_as_git_with(
        &repo.0,
        &other.0,
        "s",
        &format!("rebase continued, {tag}"),
        style.is_some(),
    );
}

#[test]
// Verifies: ADR-0033, ADR-0040
fn a_stopped_operation_writes_the_markers_git_writes_but_for_the_listed_label_departure() {
    stopped_operations_mark_as_git(None);
}

#[test]
// Verifies: ADR-0033, ADR-0040
fn in_the_diff3_styles_the_base_is_labelled_as_git_labels_it() {
    stopped_operations_mark_as_git(Some("diff3"));
    stopped_operations_mark_as_git(Some("zdiff3"));
}

#[test]
// Verifies: ADR-0033, ADR-0040
fn a_merge_labels_its_base_as_git_does_with_several_bases_or_none() {
    // Two branches each merged into the other: two merge bases, `merged common ancestors`.
    let s = Scratch::new("markers-criss-cross");
    let d = &s.0;
    git(d, &["init", "--quiet", "-b", "main"]);
    git(d, &["config", "user.name", "Ada"]);
    git(d, &["config", "user.email", "ada@example.invalid"]);
    git(d, &["config", "merge.conflictStyle", "diff3"]);
    write_file(d, "f", "base\n");
    git(d, &["add", "f"]);
    git(d, &["commit", "--quiet", "-m", "base"]);
    git(d, &["switch", "--quiet", "-c", "side"]);
    write_file(d, "s", "s\n");
    git(d, &["add", "s"]);
    git(d, &["commit", "--quiet", "-m", "s"]);
    git(d, &["switch", "--quiet", "main"]);
    write_file(d, "m", "m\n");
    git(d, &["add", "m"]);
    git(d, &["commit", "--quiet", "-m", "m"]);
    git(d, &["merge", "--quiet", "--no-edit", "side"]);
    git(d, &["switch", "--quiet", "side"]);
    git(d, &["merge", "--quiet", "--no-edit", "main~1"]);
    write_file(d, "f", "side\n");
    git(d, &["commit", "--quiet", "-am", "side f"]);
    git(d, &["switch", "--quiet", "main"]);
    write_file(d, "f", "main\n");
    git(d, &["commit", "--quiet", "-am", "main f"]);
    assert_eq!(
        git(d, &["merge-base", "--all", "main", "side"])
            .lines()
            .count(),
        2
    );
    let other = twin(&s, "markers-criss-cross-git");
    assert_eq!(
        GitWriter.write(d, &merge(d, "side", Some("side"))).unwrap(),
        None
    );
    assert!(!git_may_fail(&other.0, &["merge", "--no-edit", "side"]));
    let markers = markers_as_git(d, &other.0, "f", "criss-cross");
    assert!(
        markers.contains("||||||| merged common ancestors\n"),
        "{markers:?}"
    );

    // Two histories with no base at all: `empty tree`.
    let s = Scratch::new("markers-no-base");
    let d = &s.0;
    git(d, &["init", "--quiet", "-b", "main"]);
    git(d, &["config", "user.name", "Ada"]);
    git(d, &["config", "user.email", "ada@example.invalid"]);
    git(d, &["config", "merge.conflictStyle", "diff3"]);
    write_file(d, "f", "main\n");
    git(d, &["add", "f"]);
    git(d, &["commit", "--quiet", "-m", "main"]);
    git(d, &["switch", "--quiet", "--orphan", "other"]);
    write_file(d, "f", "other\n");
    git(d, &["add", "f"]);
    git(d, &["commit", "--quiet", "-m", "other"]);
    git(d, &["switch", "--quiet", "main"]);
    let other = twin(&s, "markers-no-base-git");
    assert_eq!(
        GitWriter
            .write(d, &merge(d, "other", Some("other")))
            .unwrap(),
        None
    );
    assert!(!git_may_fail(
        &other.0,
        &["merge", "--no-edit", "--allow-unrelated-histories", "other"]
    ));
    // With no base, no file has one: `git` writes `||||||| empty tree`, libgit2 a bare line.
    let markers = markers_as_git_with(d, &other.0, "f", "no base", true);
    assert!(markers.contains("|||||||\n"), "{markers:?}");
}

#[test]
// Verifies: ADR-0033, ADR-0040
fn a_numeric_core_abbrev_sets_the_length_of_a_labelled_commit_as_git_sets_it() {
    let repo = diverged("markers-abbrev", "ours\n", "theirs\n");
    git(&repo.0, &["config", "core.abbrev", "12"]);
    let other = twin(&repo, "markers-abbrev-git");
    let pick = Write::CherryPick {
        branch: "main".into(),
        tip: head(&repo.0),
        commit: git(&repo.0, &["rev-parse", "side"]),
    };
    assert_eq!(GitWriter.write(&repo.0, &pick).unwrap(), None);
    assert!(!git_may_fail(&other.0, &["cherry-pick", "side"]));
    let markers = markers_as_git(&repo.0, &other.0, "f", "core.abbrev 12");
    assert_eq!(their_label(&markers).len(), 12, "{markers:?}");
}

#[test]
// Verifies: ADR-0033, ADR-0040
fn a_merge_labels_its_side_by_the_branch_only_where_the_branch_is_at_the_commit() {
    // The name given is a branch elsewhere: the side is labelled by the commit, as for a commit.
    let repo = diverged("markers-moved-branch", "ours\n", "theirs\n");
    let side = git(&repo.0, &["rev-parse", "side"]);
    git(&repo.0, &["branch", "elsewhere", "main~1"]);
    let short = git(&repo.0, &["rev-parse", "--short", &side]);
    let named = Write::Merge {
        branch: "main".into(),
        tip: head(&repo.0),
        commit: side,
        name: Some("elsewhere".into()),
    };
    assert_eq!(GitWriter.write(&repo.0, &named).unwrap(), None);
    let markers = std::fs::read_to_string(repo.0.join("f")).unwrap();
    assert_eq!(their_label(&markers), short, "{markers:?}");
}

#[test]
// Verifies: ADR-0033, ADR-0040
fn a_branch_name_windows_refuses_in_a_file_name_labels_the_side_by_the_commit_instead() {
    // `git` takes `<` in a branch's name, which Windows refuses in a file's: labelled by it, the
    // side's file `<path>~<label>` would stop the write half done there, so the commit labels it.
    let repo = diverged("markers-windows-name", "ours\n", "theirs\n");
    git(&repo.0, &["branch", "-m", "side", "a<b"]);
    let side = git(&repo.0, &["rev-parse", "a<b"]);
    let short = git(&repo.0, &["rev-parse", "--short", &side]);
    let named = Write::Merge {
        branch: "main".into(),
        tip: head(&repo.0),
        commit: side,
        name: Some("a<b".into()),
    };
    assert_eq!(GitWriter.write(&repo.0, &named).unwrap(), None);
    let markers = std::fs::read_to_string(repo.0.join("f")).unwrap();
    assert_eq!(their_label(&markers), short, "{markers:?}");
}
