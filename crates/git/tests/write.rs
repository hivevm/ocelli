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

mod shown;
mod support;
use ocelli_core::port::{RepositoryReader, RepositoryWriter, Write};
use ocelli_git::GitWriter;
use shown::{kinds_of_write, stage_in, unstage_in};
use std::collections::BTreeSet;
use std::path::Path;
use support::{
    Origin, Scratch, as_twins, assert_twins_alike, git, git_command, head_of, refused,
    refused_reporting, snapshot, twin, write_file,
};

mod programs;

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

/// `git` in `dir` with `args`, its commits made at the second `at`.
fn git_at(dir: &Path, at: u32, args: &[&str]) {
    let date = format!("{at} +0000");
    let out = git_command(dir)
        .env("GIT_AUTHOR_DATE", &date)
        .env("GIT_COMMITTER_DATE", &date)
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
}

#[test]
fn twins_read_alike_where_one_makes_at_the_time_of_the_other_what_the_other_made_before() {
    let scratch = with_file("same-second");
    let origin = Origin::of(&scratch.0);
    let other = twin(&scratch, "same-second-twin");
    // The stash `git` makes in `dir` at the second `at`, of `f` changed to `content`.
    let stash_at = |dir: &Path, at: u32, content: &str, message: &[&str]| {
        write_file(dir, "f", content);
        git_at(dir, at, &[&["stash", "push", "--quiet"], message].concat());
    };
    // Each side stashes twice, the twin a second later: the index commit of this side's second
    // stash is the one of the twin's first, which the twin holds, where the twin's second stash
    // has an index commit of its own, alike but for its time.
    for (dir, start) in [(&scratch.0, 1_700_000_000), (&other.0, 1_700_000_001)] {
        stash_at(dir, start, "changed\n", &["-m", "first"]);
        git(dir, &["stash", "apply", "--quiet"]);
        stash_at(dir, start + 1, "changed\n", &[]);
    }
    assert_twins_alike(&scratch.0, &other.0, &origin, "stashed a second apart");
    // A stash of another change still differs, made at the same second or not.
    stash_at(&scratch.0, 1_700_000_003, "one\n", &[]);
    stash_at(&other.0, 1_700_000_003, "two\n", &[]);
    let (ours, theirs) = as_twins(&scratch.0, &other.0, &origin);
    assert_ne!(ours, theirs);
}

#[test]
fn twins_differ_where_one_rewrote_a_commit_they_parted_with_alike_but_for_its_time() {
    let scratch = with_file("amended");
    let origin = Origin::of(&scratch.0);
    let other = twin(&scratch, "amended-twin");
    git_at(
        &scratch.0,
        1_900_000_000,
        &["commit", "--quiet", "--amend", "--no-edit"],
    );
    let (ours, theirs) = as_twins(&scratch.0, &other.0, &origin);
    assert_ne!(ours, theirs);
}

#[test]
fn twins_differ_where_one_swapped_branches_on_commits_they_parted_with_alike_but_for_their_time() {
    let scratch = with_file("swapped");
    let d = &scratch.0;
    for (branch, at) in [("p", 1_700_000_000), ("q", 1_700_000_001)] {
        git(d, &["switch", "--quiet", "-c", branch, "main"]);
        git_at(
            d,
            at,
            &["commit", "--quiet", "--allow-empty", "-m", "alike"],
        );
    }
    git(d, &["switch", "--quiet", "main"]);
    let origin = Origin::of(d);
    let other = twin(&scratch, "swapped-twin");
    let (p, q) = (git(d, &["rev-parse", "p"]), git(d, &["rev-parse", "q"]));
    git(d, &["branch", "--quiet", "--force", "p", &q]);
    git(d, &["branch", "--quiet", "--force", "q", &p]);
    let (ours, theirs) = as_twins(d, &other.0, &origin);
    assert_ne!(ours, theirs);
}

#[test]
fn twins_read_alike_where_the_messages_of_what_each_made_name_what_each_made_since_they_parted() {
    let scratch = with_file("named-since");
    let origin = Origin::of(&scratch.0);
    let other = twin(&scratch, "named-since-twin");
    // Each side makes the same commits and tags, the twin a second later, so that each id made
    // since they parted differs and each message naming one names it as text.
    for (dir, at) in [(&scratch.0, 1_700_000_000), (&other.0, 1_700_000_001)] {
        let made = |args: &[&str]| git_at(dir, at, args);
        write_file(dir, "three", "3\n");
        made(&["add", "three"]);
        made(&["commit", "--quiet", "-m", "three"]);
        made(&["revert", "--no-edit", "HEAD"]);
        made(&["switch", "--quiet", "-c", "side", "HEAD~2"]);
        write_file(dir, "g", "g\n");
        made(&["add", "g"]);
        made(&["commit", "--quiet", "-m", "g"]);
        made(&["switch", "--quiet", "main"]);
        made(&["cherry-pick", "-x", "side"]);
        let side = git(dir, &["rev-parse", "side"]);
        made(&["merge", "--quiet", "--no-ff", "--no-edit", &side]);
        made(&["tag", "-a", "t", "-m", &format!("from {side}"), "HEAD^2"]);
    }
    assert_twins_alike(&scratch.0, &other.0, &origin, "named a second apart");
}

#[test]
fn twins_differ_where_the_messages_of_what_each_made_name_different_commits() {
    let scratch = with_file("named-apart");
    let d = &scratch.0;
    // Two commits the twins part with, alike but for their time: `p` adds `g` to main, and `q`
    // is `p` made again a second later, which a revert on `p` undoes just the same.
    write_file(d, "g", "g\n");
    git(d, &["add", "g"]);
    git_at(d, 1_700_000_000, &["commit", "--quiet", "-m", "g"]);
    git(d, &["switch", "--quiet", "-c", "q"]);
    git_at(
        d,
        1_700_000_001,
        &["commit", "--quiet", "--amend", "--no-edit"],
    );
    git(d, &["switch", "--quiet", "main"]);
    let (p, q) = (git(d, &["rev-parse", "main"]), git(d, &["rev-parse", "q"]));
    assert_ne!(p, q);
    let origin = Origin::of(d);
    let other = twin(&scratch, "named-apart-twin");
    let reverting_p = twin(&scratch, "named-apart-p");
    // Each reverts `p`, a second apart: alike.
    git_at(d, 1_700_000_002, &["revert", "--no-edit", &p]);
    git_at(&reverting_p.0, 1_700_000_003, &["revert", "--no-edit", &p]);
    assert_twins_alike(d, &reverting_p.0, &origin, "reverted p a second apart");
    // The twin reverts `q` instead, to the same tree on the same parent: only the message, which
    // names another commit, differs.
    git_at(&other.0, 1_700_000_003, &["revert", "--no-edit", &q]);
    let (ours, theirs) = as_twins(d, &other.0, &origin);
    let differences = ours.differences(&theirs, ("here", "on the twin"));
    assert_eq!(differences.len(), 1, "only main differs: {differences:#?}");
    assert_ne!(ours.refs, theirs.refs, "reverted p here and q on the twin");
}

#[test]
fn twins_differ_where_the_messages_of_what_each_made_name_different_commits_made_since() {
    let scratch = with_file("named-other");
    let origin = Origin::of(&scratch.0);
    let other = twin(&scratch, "named-other-twin");
    // Each makes the branches `one` and `two` alike, a second apart, and then a commit whose
    // message names `one` here and `two` on the twin.
    for (dir, at, named) in [
        (&scratch.0, 1_700_000_000, "one"),
        (&other.0, 1_700_000_001, "two"),
    ] {
        for branch in ["one", "two"] {
            git(dir, &["switch", "--quiet", "-c", branch, "main"]);
            git_at(
                dir,
                at,
                &["commit", "--quiet", "--allow-empty", "-m", branch],
            );
        }
        git(dir, &["switch", "--quiet", "main"]);
        let id = git(dir, &["rev-parse", named]);
        git_at(dir, at, &["commit", "--quiet", "--allow-empty", "-m", &id]);
    }
    let (ours, theirs) = as_twins(&scratch.0, &other.0, &origin);
    let differences = ours.differences(&theirs, ("here", "on the twin"));
    assert_eq!(differences.len(), 1, "only main differs: {differences:#?}");
    assert_ne!(ours.refs, theirs.refs, "main differs");
}

#[test]
fn twins_differ_where_they_name_different_blobs_made_since_or_hex_that_is_no_object() {
    // Each side does `make` with its own text, the twin a second later; the twins then differ in
    // `part` alone.
    let differ = |name: &str, texts: [&str; 2], make: &dyn Fn(&Path, u32, &str), part: &str| {
        let scratch = with_file(name);
        let origin = Origin::of(&scratch.0);
        let other = twin(&scratch, &format!("{name}-twin"));
        make(&scratch.0, 1_700_000_000, texts[0]);
        make(&other.0, 1_700_000_001, texts[1]);
        let (ours, theirs) = as_twins(&scratch.0, &other.0, &origin);
        let differences = ours.differences(&theirs, ("here", "on the twin"));
        assert_eq!(differences.len(), 1, "{name}: {differences:#?}");
        assert!(
            differences[0].trim_start().starts_with(part),
            "{name}: {differences:#?}"
        );
    };
    // A commit's message names a blob it wrote, of another content on each side.
    let names_blob = |dir: &Path, at: u32, content: &str| {
        write_file(dir, "blob", content);
        let blob = git(dir, &["hash-object", "-w", "blob"]);
        std::fs::remove_file(dir.join("blob")).unwrap();
        git_at(
            dir,
            at,
            &["commit", "--quiet", "--allow-empty", "-m", &blob],
        );
    };
    differ("named-blob", ["one\n", "two\n"], &names_blob, "refs");
    // The index holds another content of `f` on each side, the working directory the same.
    let stages = |dir: &Path, _: u32, content: &str| {
        write_file(dir, "f", content);
        git(dir, &["add", "f"]);
        write_file(dir, "f", "worked\n");
    };
    differ("staged-blob", ["one\n", "two\n"], &stages, "index");
    // A commit's message holds 40 hexadecimal digits that are no object, other ones on each side.
    let names_hex = |dir: &Path, at: u32, hex: &str| {
        git_at(dir, at, &["commit", "--quiet", "--allow-empty", "-m", hex]);
    };
    let (a, b) = ("a".repeat(40), "b".repeat(40));
    differ("named-hex", [&a, &b], &names_hex, "refs");
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

fn commit_on(head: &str, message: &str) -> Write {
    Write::Commit {
        message: message.to_string(),
        head: Some(head.to_string()),
    }
}

fn staged_names(dir: &Path) -> String {
    git(dir, &["diff", "--cached", "--name-status"])
}

/// The paths a caller may send that name no file of the working directory: each made git2 panic
/// where it read the index, or leads into the repository's own files.
const NO_FILE: &[&str] = &[
    "",
    ".",
    "./f",
    "../x",
    "dir/../f",
    "/abs",
    "f\0g",
    ".git/config",
    ".GIT/config",
    ".git/HEAD",
];

#[test]
// Verifies: ADR-0033, ADR-0039
fn a_path_that_names_no_file_of_the_working_directory_is_refused_by_every_write_and_read() {
    use ocelli_core::model::Conflict;
    use ocelli_core::port::Take;
    let scratch = with_file("no-file");
    let d = &scratch.0;
    write_file(d, "f", "changed\n");
    let head = git(d, &["rev-parse", "HEAD"]);
    for path in NO_FILE {
        let read_why = "names no file of the working directory";
        // A write refuses a NUL in any string it is given first, before it reads the path; the
        // read takes the path's own guard.
        let why = if path.contains('\0') {
            "holds a NUL character"
        } else {
            read_why
        };
        for shown in [None, Some(head.clone())] {
            let stage = Write::StageFile {
                path: path.to_string(),
                shown: shown.clone(),
            };
            refused(d, &stage, why);
            let unstage = Write::UnstageFile {
                path: path.to_string(),
                shown,
            };
            refused(d, &unstage, why);
        }
        let resolve = Write::Resolve {
            path: path.to_string(),
            conflict: Conflict {
                path: path.to_string(),
                ancestor: None,
                ours: None,
                theirs: None,
            },
            take: Take::AsEdited,
        };
        refused(d, &resolve, why);
        for sha in [head.as_str(), ocelli_core::model::WORKDIR_SHA] {
            let read = ocelli_git::GitReader.load_file_diff(d, sha, path);
            assert!(
                read.is_err_and(|e| e.to_string().contains(read_why)),
                "{path:?} at {sha}"
            );
        }
    }
    // A file of the working directory is taken.
    GitWriter.write(d, &stage_in(d, "f")).unwrap();
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

    // An upstream merge that names no branch on the remote, a tag here, is nothing to push to.
    git(&clone.0, &["config", "branch.feat.merge", "refs/tags/v1"]);
    refused(&clone.0, &push_of(&clone.0, "feat"), "no upstream branch");
    git(
        &clone.0,
        &["config", "branch.feat.merge", "refs/heads/main"],
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

#[test]
// Verifies: ADR-0033, Q-6
fn a_stash_libgit2_would_make_by_writing_past_it_is_refused() {
    let stash = |untracked| Write::CreateStash {
        message: None,
        untracked,
    };
    // A link where the index has a file, to a file outside the repository.
    let outside = Scratch::new("stash-outside");
    std::fs::write(outside.0.join("secret"), "SECRET\n").unwrap();
    let scratch = with_file("stash-link");
    let d = &scratch.0;
    std::fs::remove_file(d.join("f")).unwrap();
    std::os::unix::fs::symlink(outside.0.join("secret"), d.join("f")).unwrap();
    refused(d, &stash(false), "f is a link where the index has a file");
    assert_eq!(
        std::fs::read_to_string(outside.0.join("secret")).unwrap(),
        "SECRET\n"
    );

    // An entry marked skip-worktree, whose content a stash would overwrite.
    let scratch = with_file("stash-skip");
    let d = &scratch.0;
    git(d, &["update-index", "--skip-worktree", "f"]);
    std::fs::write(d.join("f"), "the developer's\n").unwrap();
    std::fs::write(d.join("g"), "g\n").unwrap();
    git(d, &["add", "g"]);
    refused(d, &stash(false), "f is marked skip-worktree");

    // An ignored file in a directory only untracked files make, which -u would delete.
    let scratch = with_file("stash-ignored");
    let d = &scratch.0;
    std::fs::write(d.join(".gitignore"), "*.log\n").unwrap();
    git(d, &["add", ".gitignore"]);
    git(d, &["commit", "--quiet", "-m", "ignore logs"]);
    std::fs::create_dir(d.join("ud")).unwrap();
    std::fs::write(d.join("ud/u"), "untracked\n").unwrap();
    std::fs::write(d.join("ud/l.log"), "kept\n").unwrap();
    refused(d, &stash(true), "ud/l.log is ignored in ud");
    assert_eq!(
        std::fs::read_to_string(d.join("ud/l.log")).unwrap(),
        "kept\n"
    );
}

/// A repository made by [`with_file`], and beside it a directory outside it whose `f` holds a
/// secret.
fn beside_outside(name: &str) -> (Scratch, Scratch) {
    let outside = Scratch::new(&format!("{name}-outside"));
    std::fs::write(outside.0.join("f"), "SECRET\n").unwrap();
    (with_file(name), outside)
}

/// Commits everything in `dir` as `message`, and the commit made.
fn commit_all(dir: &Path, message: &str) -> String {
    git(dir, &["add", "--all"]);
    git(dir, &["commit", "--quiet", "-m", message]);
    git(dir, &["rev-parse", "HEAD"])
}

/// Puts a link at `at` to `to`.
fn link(to: &Path, at: &Path) {
    std::os::unix::fs::symlink(to, at).unwrap();
}

/// Asserts that nothing outside the repository changed: the secret as it was, and nothing new.
fn kept(outside: &Scratch) {
    assert_eq!(
        std::fs::read_to_string(outside.0.join("f")).unwrap(),
        "SECRET\n"
    );
    let found: Vec<_> = std::fs::read_dir(&outside.0)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(found.len(), 1, "written outside: {found:?}");
}

/// What only the guard of every write of the working directory says of a link in the way.
const LINK_IN_THE_WAY: &str = "is a link, which writing";

/// A repository beside a directory outside it, with `side` adding `d/f`, `main` ahead of where
/// they parted, and an untracked link at `d` to the directory outside: (repository, outside,
/// main, side).
fn link_where_side_adds_a_directory(name: &str) -> (Scratch, Scratch, String, String) {
    let (scratch, outside) = beside_outside(name);
    let d = &scratch.0;
    git(d, &["switch", "--quiet", "-c", "side"]);
    write_file(d, "d/f", "side\n");
    let side = commit_all(d, "side");
    git(d, &["switch", "--quiet", "main"]);
    write_file(d, "g", "main\n");
    let main = commit_all(d, "main");
    link(&outside.0, &d.join("d"));
    (scratch, outside, main, side)
}

#[test]
// Verifies: ADR-0033, ADR-0039, Q-6
fn a_switch_goes_through_no_link_the_index_does_not_hold() {
    let (scratch, outside, _, side) = link_where_side_adds_a_directory("link-switch");
    let switch = Write::SwitchBranch {
        name: "side".into(),
        tip: side,
    };
    refused(&scratch.0, &switch, LINK_IN_THE_WAY);
    kept(&outside);
}

#[test]
// Verifies: ADR-0033, ADR-0039, Q-6
fn a_merge_goes_through_no_link_the_index_does_not_hold() {
    let (scratch, outside, main, side) = link_where_side_adds_a_directory("link-merge");
    let merge = Write::Merge {
        branch: "main".into(),
        tip: main,
        commit: side,
        name: None,
    };
    refused(&scratch.0, &merge, LINK_IN_THE_WAY);
    kept(&outside);
}

#[test]
// Verifies: ADR-0033, ADR-0039, Q-6
fn a_cherry_pick_goes_through_no_link_the_index_does_not_hold() {
    let (scratch, outside, main, side) = link_where_side_adds_a_directory("link-pick");
    let pick = Write::CherryPick {
        branch: "main".into(),
        tip: main,
        commit: side,
    };
    refused(&scratch.0, &pick, LINK_IN_THE_WAY);
    kept(&outside);
}

#[test]
// Verifies: ADR-0033, ADR-0039, Q-6
fn a_revert_goes_through_no_link_the_index_does_not_hold() {
    let (scratch, outside) = beside_outside("link-revert");
    let d = &scratch.0;
    write_file(d, "d/f", "tracked\n");
    commit_all(d, "d");
    git(d, &["rm", "-r", "--quiet", "d"]);
    let removed = commit_all(d, "no d");
    link(&outside.0, &d.join("d"));
    let revert = Write::Revert {
        branch: "main".into(),
        tip: removed.clone(),
        commit: removed,
    };
    refused(d, &revert, LINK_IN_THE_WAY);
    kept(&outside);
}

#[test]
// Verifies: ADR-0033, ADR-0039, Q-6
fn a_rebase_goes_through_no_link_the_index_does_not_hold_as_it_starts_or_steps() {
    // Its start checks out what it rebases onto.
    let (scratch, outside, main, side) = link_where_side_adds_a_directory("link-rebase-start");
    let rebase = Write::Rebase {
        branch: "main".into(),
        tip: main,
        onto: side,
    };
    refused(&scratch.0, &rebase, LINK_IN_THE_WAY);
    kept(&outside);

    // A step continued after a stop writes what its commit adds.
    let (scratch, outside) = beside_outside("link-rebase-step");
    let d = &scratch.0;
    git(d, &["switch", "--quiet", "-c", "side"]);
    write_file(d, "f", "side\n");
    let side = commit_all(d, "side");
    git(d, &["switch", "--quiet", "main"]);
    write_file(d, "f", "main\n");
    commit_all(d, "main");
    write_file(d, "d/f", "main\n");
    let main = commit_all(d, "d");
    git(d, &["rm", "-r", "--quiet", "d"]);
    let main = {
        let _ = main;
        commit_all(d, "no d")
    };
    let stopped = GitWriter
        .write(
            d,
            &Write::Rebase {
                branch: "main".into(),
                tip: main,
                onto: side,
            },
        )
        .unwrap();
    assert_eq!(stopped, None, "the rebase stopped on f");
    git(d, &["checkout", "--quiet", "--theirs", "--", "f"]);
    git(d, &["add", "f"]);
    link(&outside.0, &d.join("d"));
    let next = Write::Continue {
        operation: ocelli_core::model::Operation::Rebase,
        head: git(d, &["rev-parse", "HEAD"]),
    };
    // The step resolved is committed first, as `git rebase --continue` commits it; the next one
    // is refused before it writes anything.
    support::stopped_partway(d, &next, LINK_IN_THE_WAY);
    assert!(
        d.join(".git/rebase-merge").exists(),
        "the rebase is still in progress"
    );
    kept(&outside);
}

#[test]
// Verifies: ADR-0033, ADR-0039, Q-6
fn a_rebase_s_abort_goes_through_no_link_the_index_does_not_hold() {
    let (scratch, outside) = beside_outside("link-rebase-abort");
    let d = &scratch.0;
    git(d, &["switch", "--quiet", "-c", "side"]);
    write_file(d, "f", "side\n");
    let side = commit_all(d, "side");
    git(d, &["switch", "--quiet", "main"]);
    write_file(d, "f", "main\n");
    write_file(d, "d/f", "main\n");
    let main = commit_all(d, "main");
    let stopped = GitWriter
        .write(
            d,
            &Write::Rebase {
                branch: "main".into(),
                tip: main,
                onto: side,
            },
        )
        .unwrap();
    assert_eq!(stopped, None, "the rebase stopped on f");
    std::fs::remove_dir_all(d.join("d")).unwrap();
    link(&outside.0, &d.join("d"));
    let abort = Write::Abort {
        operation: ocelli_core::model::Operation::Rebase,
        head: git(d, &["rev-parse", "HEAD"]),
    };
    refused(d, &abort, LINK_IN_THE_WAY);
    kept(&outside);
}

#[test]
// Verifies: ADR-0033, ADR-0039, Q-6
fn a_hard_reset_goes_through_no_link_the_index_does_not_hold() {
    let reset_hard = |d: &Path| {
        let head = git(d, &["rev-parse", "HEAD"]);
        Write::Reset {
            branch: "main".into(),
            tip: head.clone(),
            commit: head,
            mode: ocelli_core::port::ResetMode::Hard,
        }
    };
    // The link where the index has a directory, and where it has a file.
    let (scratch, outside) = beside_outside("link-reset");
    let d = &scratch.0;
    write_file(d, "d/f", "tracked\n");
    commit_all(d, "d");
    std::fs::remove_dir_all(d.join("d")).unwrap();
    link(&outside.0, &d.join("d"));
    refused(
        d,
        &reset_hard(d),
        "d is a link, which writing d/f would go through",
    );
    kept(&outside);
    std::fs::remove_file(d.join("d")).unwrap();
    git(d, &["checkout", "--quiet", "--", "d"]);
    std::fs::remove_file(d.join("f")).unwrap();
    link(&outside.0.join("f"), &d.join("f"));
    refused(
        d,
        &reset_hard(d),
        "f is a link, which writing f would go through",
    );
    kept(&outside);
}

#[test]
// Verifies: ADR-0033, ADR-0039, Q-6
fn a_stash_goes_through_no_link_the_index_does_not_hold() {
    // Only the checked-out commit still has the directory the link stands in.
    let (scratch, outside) = beside_outside("link-stash");
    let d = &scratch.0;
    write_file(d, "d/f", "tracked\n");
    commit_all(d, "d");
    git(d, &["rm", "-r", "--cached", "--quiet", "d"]);
    std::fs::remove_dir_all(d.join("d")).unwrap();
    link(&outside.0, &d.join("d"));
    refused(d, &create_stash(None, false), LINK_IN_THE_WAY);
    kept(&outside);

    // The untracked files, an untracked link among them: stashed as the link it is.
    let (scratch, outside) = beside_outside("link-stash-untracked");
    let d = &scratch.0;
    link(&outside.0, &d.join("d"));
    GitWriter.write(d, &create_stash(None, true)).unwrap();
    assert!(!d.join("d").exists(), "the link is stashed");
    kept(&outside);

    // Applied where a link stands in a directory the stash's untracked files go to.
    let (scratch, outside) = beside_outside("link-stash-apply");
    let d = &scratch.0;
    write_file(d, "d/f", "untracked\n");
    GitWriter.write(d, &create_stash(None, true)).unwrap();
    link(&outside.0, &d.join("d"));
    let stash = git(d, &["rev-parse", "refs/stash"]);
    refused(d, &apply(0, &stash, false), LINK_IN_THE_WAY);
    kept(&outside);
}

/// A repository beside a directory outside it, stopped in a cherry-pick of `side` that
/// conflicts on `f`, with `f` then put as a link to the secret outside: (repository, outside).
fn conflict_on_a_link(name: &str) -> (Scratch, Scratch) {
    let (scratch, outside) = beside_outside(name);
    let d = &scratch.0;
    git(d, &["switch", "--quiet", "-c", "side"]);
    write_file(d, "f", "side\n");
    commit_all(d, "side");
    git(d, &["switch", "--quiet", "main"]);
    write_file(d, "f", "main\n");
    commit_all(d, "main");
    assert!(!support::git_may_fail(d, &["cherry-pick", "side"]));
    std::fs::remove_file(d.join("f")).unwrap();
    link(&outside.0.join("f"), &d.join("f"));
    (scratch, outside)
}

#[test]
// Verifies: ADR-0033, ADR-0039, Q-6
fn an_abort_and_a_resolve_go_through_no_link_the_index_does_not_hold() {
    let (scratch, outside) = conflict_on_a_link("link-abort");
    let d = &scratch.0;
    let abort = Write::Abort {
        operation: ocelli_core::model::Operation::CherryPick,
        head: git(d, &["rev-parse", "HEAD"]),
    };
    refused(d, &abort, LINK_IN_THE_WAY);
    kept(&outside);

    let (scratch, outside) = conflict_on_a_link("link-resolve");
    let d = &scratch.0;
    let conflict = ocelli_git::GitReader
        .load_refs(d)
        .unwrap()
        .in_progress
        .unwrap()
        .conflicts
        .remove(0);
    let resolve = Write::Resolve {
        path: "f".into(),
        conflict,
        take: ocelli_core::port::Take::Theirs,
    };
    refused(d, &resolve, LINK_IN_THE_WAY);
    kept(&outside);
}

/// A repository beside a directory outside it where `main` adds a file `x` and `side` a
/// directory `x`, and a link where libgit2 would write the file's side of their conflict, under
/// `x~HEAD`, to a file outside that is not there yet: (repository, outside, main, side).
fn file_and_directory_conflict(name: &str) -> (Scratch, Scratch, String, String) {
    let (scratch, outside) = beside_outside(name);
    let d = &scratch.0;
    git(d, &["switch", "--quiet", "-c", "side"]);
    write_file(d, "x/y", "side\n");
    let side = commit_all(d, "side");
    git(d, &["switch", "--quiet", "main"]);
    write_file(d, "x", "planted\n");
    let main = commit_all(d, "main");
    link(&outside.0.join("planted"), &d.join("x~HEAD"));
    (scratch, outside, main, side)
}

/// What the guard says of a conflict between a file and a directory.
const FILE_AND_DIRECTORY: &str = "a file and a directory named x conflict";

#[test]
// Verifies: ADR-0033, ADR-0039, Q-6
fn a_conflict_between_a_file_and_a_directory_is_refused_by_every_write_that_merges() {
    let (scratch, outside, main, side) = file_and_directory_conflict("df-merge");
    let merge = Write::Merge {
        branch: "main".into(),
        tip: main,
        commit: side,
        name: None,
    };
    refused(&scratch.0, &merge, FILE_AND_DIRECTORY);
    kept(&outside);

    let (scratch, outside, main, side) = file_and_directory_conflict("df-pick");
    let pick = Write::CherryPick {
        branch: "main".into(),
        tip: main,
        commit: side,
    };
    refused(&scratch.0, &pick, FILE_AND_DIRECTORY);
    kept(&outside);

    // A revert bringing back the directory the file now stands in.
    let (scratch, outside) = beside_outside("df-revert");
    let d = &scratch.0;
    write_file(d, "x/y", "tracked\n");
    commit_all(d, "x/y");
    git(d, &["rm", "-r", "--quiet", "x"]);
    let removed = commit_all(d, "no x");
    write_file(d, "x", "planted\n");
    let main = commit_all(d, "x");
    link(&outside.0.join("planted"), &d.join("x~HEAD"));
    let revert = Write::Revert {
        branch: "main".into(),
        tip: main,
        commit: removed,
    };
    refused(d, &revert, FILE_AND_DIRECTORY);
    kept(&outside);

    // A rebase whose step adds the file onto the directory.
    let (scratch, outside, main, side) = file_and_directory_conflict("df-rebase");
    let rebase = Write::Rebase {
        branch: "main".into(),
        tip: main,
        onto: side,
    };
    refused(&scratch.0, &rebase, FILE_AND_DIRECTORY);
    kept(&outside);

    // A stash whose untracked file stands where the checked-out commit now has a directory.
    let (scratch, outside) = beside_outside("df-stash");
    let d = &scratch.0;
    write_file(d, "x", "planted\n");
    GitWriter.write(d, &create_stash(None, true)).unwrap();
    write_file(d, "x/y", "tracked\n");
    commit_all(d, "x/y");
    link(&outside.0.join("planted"), &d.join("x~Stashed changes"));
    let stash = git(d, &["rev-parse", "refs/stash"]);
    // Refused before the guard: a stash that would overwrite a tracked path is never applied.
    refused(d, &apply(0, &stash, false), "x is tracked by now");
    kept(&outside);
}

#[test]
// Verifies: ADR-0033, ADR-0039, Q-6
fn a_conflict_libgit2_writes_through_a_link_or_beside_its_path_is_refused() {
    // Two files renamed to one name, whose sides libgit2 writes as c~HEAD and c~<commit>.
    let (scratch, outside) = beside_outside("rename-collision");
    let d = &scratch.0;
    write_file(d, "a", &"alpha line\n".repeat(20));
    write_file(d, "b", &"beta line\n".repeat(20));
    commit_all(d, "a and b");
    git(d, &["switch", "--quiet", "-c", "side"]);
    git(d, &["mv", "b", "c"]);
    let side = commit_all(d, "b to c");
    git(d, &["switch", "--quiet", "main"]);
    git(d, &["mv", "a", "c"]);
    let main = commit_all(d, "a to c");
    link(&outside.0.join("planted"), &d.join("c~HEAD"));
    let merge = Write::Merge {
        branch: "main".into(),
        tip: main,
        commit: side,
        name: None,
    };
    refused(
        d,
        &merge,
        "c~HEAD is in the way of a side of the conflict on c",
    );
    kept(&outside);

    // A link the checked-out commit tracks, which the other side changes as a file.
    let (scratch, outside) = beside_outside("link-file-conflict");
    let d = &scratch.0;
    git(d, &["switch", "--quiet", "-c", "side"]);
    write_file(d, "f", "side\n");
    let side = commit_all(d, "f changed");
    git(d, &["switch", "--quiet", "main"]);
    std::fs::remove_file(d.join("f")).unwrap();
    link(&outside.0.join("planted"), &d.join("f"));
    let main = commit_all(d, "f a link");
    let merge = Write::Merge {
        branch: "main".into(),
        tip: main,
        commit: side,
        name: None,
    };
    refused(
        d,
        &merge,
        "f is a link, which libgit2 would write a side of its conflict through",
    );
    kept(&outside);
}

/// What `git` prints in `dir` for `args`, `input` on its standard input, trimmed.
fn git_fed(dir: &Path, args: &[&str], input: &[u8]) -> String {
    use std::io::Write as _;
    let mut child = git_command(dir)
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "git {args:?}");
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

#[test]
// Verifies: ADR-0033, Q-6
fn a_tree_naming_one_path_both_a_file_and_a_directory_is_never_written() {
    let scratch = with_file("tree-twice");
    let d = &scratch.0;
    let raw = |id: &str| -> Vec<u8> {
        (0..id.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&id[i..i + 2], 16).unwrap())
            .collect()
    };
    let blob = git_fed(d, &["hash-object", "-w", "--stdin"], b"file\n");
    let inner = git_fed(
        d,
        &["mktree"],
        format!("100644 blob {blob}\ty\n").as_bytes(),
    );
    // `x` as a file and as a directory in one tree, which only a damaged or forged repository
    // holds: `git fsck` reports it.
    let mut tree = b"100644 x\0".to_vec();
    tree.extend(raw(&blob));
    tree.extend_from_slice(b"40000 x\0");
    tree.extend(raw(&inner));
    let twice = git_fed(
        d,
        &["hash-object", "-t", "tree", "--literally", "-w", "--stdin"],
        &tree,
    );
    let head = git(d, &["rev-parse", "HEAD"]);
    let commit = git(d, &["commit-tree", &twice, "-p", &head, "-m", "x twice"]);
    git(d, &["branch", "twice", &commit]);
    let switch = Write::SwitchBranch {
        name: "twice".into(),
        tip: commit,
    };
    refused(d, &switch, "a tree names x twice");
}

/// The tree whose entries are `entries`, (mode, name, id), as written, a name twice included,
/// which only `git hash-object --literally` makes.
fn raw_tree(dir: &Path, entries: &[(&str, &str, &str)]) -> String {
    let mut bytes = Vec::new();
    for (mode, name, id) in entries {
        bytes.extend_from_slice(format!("{mode} {name}\0").as_bytes());
        for i in (0..id.len()).step_by(2) {
            bytes.push(u8::from_str_radix(&id[i..i + 2], 16).unwrap());
        }
    }
    git_fed(
        dir,
        &["hash-object", "-t", "tree", "--literally", "-w", "--stdin"],
        &bytes,
    )
}

/// The id `dir`'s tree at `rev` gives `name`.
fn entry_id(dir: &Path, rev: &str, name: &str) -> String {
    git(dir, &["rev-parse", &format!("{rev}:{name}")])
}

/// Every write that checks a commit out or merges one, of `side` onto `main` in `d`: (what it is,
/// the write).
fn writes_of(main: &str, side: &str) -> Vec<(&'static str, Write)> {
    vec![
        (
            "switch",
            Write::SwitchBranch {
                name: "side".into(),
                tip: side.into(),
            },
        ),
        (
            "reset",
            Write::Reset {
                branch: "main".into(),
                tip: main.into(),
                commit: side.into(),
                mode: ocelli_core::port::ResetMode::Hard,
            },
        ),
        (
            "merge",
            Write::Merge {
                branch: "main".into(),
                tip: main.into(),
                commit: side.into(),
                name: None,
            },
        ),
        (
            "cherry-pick",
            Write::CherryPick {
                branch: "main".into(),
                tip: main.into(),
                commit: side.into(),
            },
        ),
        (
            "rebase",
            Write::Rebase {
                branch: "main".into(),
                tip: main.into(),
                onto: side.into(),
            },
        ),
    ]
}

#[test]
// Verifies: ADR-0033, Q-6
fn a_tree_naming_a_plain_file_twice_is_refused_by_every_write_that_reads_it() {
    // No link and no conflict with the checked-out commit: only reading the target's own tree
    // finds the duplicate, so every write that reads a tree must read this one.
    for what in ["switch", "reset", "merge", "cherry-pick", "rebase"] {
        let scratch = with_file(&format!("plain-twice-{what}"));
        let d = &scratch.0;
        let main = git(d, &["rev-parse", "HEAD"]);
        let one = git_fed(d, &["hash-object", "-w", "--stdin"], b"one\n");
        let two = git_fed(d, &["hash-object", "-w", "--stdin"], b"two\n");
        let f = entry_id(d, &main, "f");
        let tree = raw_tree(
            d,
            &[
                ("100644", "b", &one),
                ("100644", "b", &two),
                ("100644", "f", &f),
            ],
        );
        let side = git(d, &["commit-tree", &tree, "-p", &main, "-m", "b twice"]);
        git(d, &["branch", "side", &side]);
        let write = writes_of(&main, &side)
            .into_iter()
            .find(|(w, _)| *w == what)
            .map(|(_, w)| w)
            .unwrap();
        refused(d, &write, "a tree names b twice");
    }
}

/// A repository beside a directory outside it whose `main` commits a link `a` to that directory,
/// or to the secret where `to_secret` says so, and whose `side` names `a` twice in one tree, as
/// the link and as `also`, a directory holding `x` or a file: (repository, outside, main, side).
fn named_twice(
    name: &str,
    to_secret: bool,
    also: &str,
    link_first: bool,
) -> (Scratch, Scratch, String, String) {
    let (scratch, outside) = beside_outside(name);
    let d = &scratch.0;
    let target = if to_secret {
        outside.0.join("f")
    } else {
        outside.0.clone()
    };
    link(&target, &d.join("a"));
    let main = commit_all(d, "link a");
    let pwned = git_fed(d, &["hash-object", "-w", "--stdin"], b"PWNED\n");
    let other = if also == "directory" {
        ("40000", raw_tree(d, &[("100644", "x", &pwned)]))
    } else {
        ("100644", pwned)
    };
    let a = entry_id(d, &main, "a");
    let f = entry_id(d, &main, "f");
    let mut entries = vec![
        ("120000", "a", a.as_str()),
        (other.0, "a", other.1.as_str()),
    ];
    if !link_first {
        entries.reverse();
    }
    entries.push(("100644", "f", &f));
    let tree = raw_tree(d, &entries);
    let side = git(d, &["commit-tree", &tree, "-p", &main, "-m", "a twice"]);
    git(d, &["branch", "side", &side]);
    (scratch, outside, main, side)
}

#[test]
// Verifies: ADR-0033, ADR-0039, Q-6
fn a_tree_naming_a_link_twice_is_never_written_through() {
    for (also, link_first, to_secret) in [
        ("directory", true, false),
        ("file", true, true),
        ("file", false, true),
    ] {
        for what in ["switch", "reset", "merge", "cherry-pick", "rebase"] {
            let name = format!("twice-{also}-{link_first}-{what}");
            let (scratch, outside, main, side) = named_twice(&name, to_secret, also, link_first);
            let write = writes_of(&main, &side)
                .into_iter()
                .find(|(w, _)| *w == what)
                .map(|(_, w)| w)
                .unwrap();
            refused(&scratch.0, &write, "a tree names a twice");
            kept(&outside);
        }
    }
}

#[test]
// Verifies: ADR-0033, ADR-0039, Q-6
fn a_tree_naming_a_link_and_a_file_one_name_is_never_written_by_a_pull() {
    let (scratch, outside) = beside_outside("twice-pull");
    let d = &scratch.0;
    let main = git(d, &["rev-parse", "HEAD"]);
    let pwned = git_fed(d, &["hash-object", "-w", "--stdin"], b"PWNED\n");
    let target = outside.0.join("f");
    let to = git_fed(
        d,
        &["hash-object", "-w", "--stdin"],
        target.to_str().unwrap().as_bytes(),
    );
    let f = entry_id(d, &main, "f");
    let tree = raw_tree(
        d,
        &[
            ("120000", "a", &to),
            ("100644", "a", &pwned),
            ("100644", "f", &f),
        ],
    );
    let twice = git(d, &["commit-tree", &tree, "-p", &main, "-m", "a twice"]);
    let clone = Scratch::new("twice-pull-clone");
    std::fs::remove_dir_all(&clone.0).unwrap();
    git(
        Path::new("/"),
        &[
            "clone",
            "--quiet",
            d.to_str().unwrap(),
            clone.0.to_str().unwrap(),
        ],
    );
    git(d, &["update-ref", "refs/heads/main", &twice]);
    // Fetched first, which a refused pull has done too.
    git(&clone.0, &["fetch", "--quiet", "origin"]);
    let pull = Write::Pull {
        branch: "main".into(),
        tip: main,
    };
    refused(&clone.0, &pull, "a tree names a twice");
    kept(&outside);
}

#[test]
// Verifies: ADR-0033, ADR-0039, Q-6
fn a_link_only_staged_is_never_written_through() {
    // A switch to a commit that adds a/x, the link staged where a stands.
    let (scratch, outside) = beside_outside("staged-switch");
    let d = &scratch.0;
    git(d, &["switch", "--quiet", "-c", "side"]);
    write_file(d, "a/x", "PWNED\n");
    let side = commit_all(d, "a/x");
    git(d, &["switch", "--quiet", "main"]);
    link(&outside.0, &d.join("a"));
    git(d, &["add", "a"]);
    let switch = Write::SwitchBranch {
        name: "side".into(),
        tip: side,
    };
    refused(d, &switch, LINK_IN_THE_WAY);
    kept(&outside);

    // A hard reset and a stash, where the commit has the directory and the index the link.
    for what in ["reset", "stash"] {
        let (scratch, outside) = beside_outside(&format!("staged-{what}"));
        let d = &scratch.0;
        write_file(d, "a/x", "PWNED\n");
        let head = commit_all(d, "a/x");
        git(d, &["rm", "-r", "--quiet", "--cached", "a"]);
        std::fs::remove_dir_all(d.join("a")).unwrap();
        link(&outside.0, &d.join("a"));
        git(d, &["add", "a"]);
        let write = if what == "reset" {
            Write::Reset {
                branch: "main".into(),
                tip: head.clone(),
                commit: head,
                mode: ocelli_core::port::ResetMode::Hard,
            }
        } else {
            create_stash(None, false)
        };
        refused(d, &write, LINK_IN_THE_WAY);
        kept(&outside);
    }
}

#[test]
// Verifies: ADR-0033, ADR-0039
fn a_committed_link_a_write_turns_into_a_directory_is_replaced_not_written_through() {
    for what in ["switch", "reset", "cherry-pick"] {
        let (scratch, outside) = beside_outside(&format!("typechange-{what}"));
        let d = &scratch.0;
        link(&outside.0, &d.join("a"));
        let main = commit_all(d, "link a");
        git(d, &["switch", "--quiet", "-c", "side"]);
        std::fs::remove_file(d.join("a")).unwrap();
        write_file(d, "a/x", "side\n");
        let side = commit_all(d, "a/x");
        git(d, &["switch", "--quiet", "main"]);
        let write = writes_of(&main, &side)
            .into_iter()
            .find(|(w, _)| *w == what)
            .map(|(_, w)| w)
            .unwrap();
        GitWriter.write(d, &write).unwrap();
        assert!(d.join("a").symlink_metadata().unwrap().is_dir(), "{what}");
        assert_eq!(std::fs::read_to_string(d.join("a/x")).unwrap(), "side\n");
        kept(&outside);
    }
}

#[test]
// Verifies: ADR-0033, ADR-0039
fn names_one_where_case_is_not_told_apart_are_refused_where_the_repository_says_so() {
    let scratch = with_file("ignorecase");
    let d = &scratch.0;
    let head = git(d, &["rev-parse", "HEAD"]);
    // The tree is built by hand: a working tree on a case-insensitive file system, APFS on macOS
    // among them, cannot hold both `a` and `A`, so `git add` would stage only one of them.
    let lower = git_fed(d, &["hash-object", "-w", "--stdin"], b"lower\n");
    let upper = git_fed(d, &["hash-object", "-w", "--stdin"], b"upper\n");
    let f = entry_id(d, &head, "f");
    let tree = raw_tree(
        d,
        &[
            ("100644", "A", &upper),
            ("100644", "a", &lower),
            ("100644", "f", &f),
        ],
    );
    let side = git(d, &["commit-tree", &tree, "-p", &head, "-m", "a and A"]);
    git(d, &["branch", "side", &side]);
    let switch = Write::SwitchBranch {
        name: "side".into(),
        tip: side.clone(),
    };
    git(d, &["config", "core.ignorecase", "true"]);
    refused(d, &switch, "one name where case is not told apart");

    // Where the file system tells case apart and the repository does not fold it, both are
    // written. On a file system that does not, the guard folds regardless, so the write stays
    // refused; and the two could not both land anyway.
    git(d, &["config", "core.ignorecase", "false"]);
    let probe = scratch.0.join("probe-CF");
    std::fs::write(&probe, "x").unwrap();
    let case_sensitive = !scratch.0.join("probe-cf").exists();
    std::fs::remove_file(&probe).unwrap();
    if case_sensitive && cfg!(not(any(windows, target_os = "macos"))) {
        GitWriter.write(d, &switch).unwrap();
        assert_eq!(std::fs::read_to_string(d.join("A")).unwrap(), "upper\n");
        assert_eq!(std::fs::read_to_string(d.join("a")).unwrap(), "lower\n");
    } else {
        refused(d, &switch, "one name where case is not told apart");
    }
}

#[test]
// Verifies: ADR-0033, ADR-0040, ADR-0039, Q-6
fn a_conflict_side_is_written_under_the_commit_id_not_its_subject() {
    // libgit2 names the side of a rename collision `<path>~<label>`, and left to itself takes
    // the label from the commit subject, which can hold `/` and `..`: a crafted subject would
    // write a new file anywhere writable. The label is the abbreviated commit id instead, as
    // `git` writes it without the subject (ADR-0040).
    let (scratch, outside) = beside_outside("label-traversal");
    let d = &scratch.0;
    write_file(d, "a", &"alpha line\n".repeat(20));
    write_file(d, "b", &"beta line\n".repeat(20));
    commit_all(d, "a and b");
    git(d, &["switch", "--quiet", "-c", "side"]);
    git(d, &["mv", "b", "c"]);
    // The subject is a path that climbs out of the repository into the directory beside it.
    let traversal = format!(
        "../{}/pwned",
        outside.0.file_name().unwrap().to_str().unwrap()
    );
    git(d, &["commit", "--quiet", "-m", &traversal]);
    let side = git(d, &["rev-parse", "HEAD"]);
    let short = git(d, &["rev-parse", "--short", "HEAD"]);
    git(d, &["switch", "--quiet", "main"]);
    git(d, &["mv", "a", "c"]);
    let main = commit_all(d, "a to c");
    GitWriter
        .write(
            d,
            &Write::CherryPick {
                branch: "main".into(),
                tip: main,
                commit: side,
            },
        )
        .unwrap();
    // Nothing climbed out, and the side stands beside its path, under the commit's id.
    assert!(!outside.0.join("pwned").exists(), "written outside");
    assert_eq!(
        std::fs::read_dir(&outside.0).unwrap().count(),
        1,
        "only the secret is outside"
    );
    assert!(
        d.join(format!("c~{short}")).exists(),
        "the side under the id"
    );
    assert!(d.join("c~HEAD").exists(), "ours under HEAD");
}

#[test]
// Verifies: ADR-0033, ADR-0039
fn a_rebase_replaying_a_root_commit_applies_it_without_a_false_refusal() {
    // A root commit has no parent: libgit2's step merges the empty tree, HEAD and the commit,
    // which the guard reads the same way, rather than a plain diff that would miss a conflict.
    let (scratch, outside) = beside_outside("root-rebase");
    let d = &scratch.0;
    write_file(d, "keep", "base\n");
    commit_all(d, "base-main");
    let main = git(d, &["rev-parse", "HEAD"]);
    git(d, &["switch", "--quiet", "--orphan", "other"]);
    link(&outside.0, &d.join("a"));
    git(d, &["add", "a"]);
    git(d, &["commit", "--quiet", "-m", "root link a"]);
    let other = git(d, &["rev-parse", "HEAD"]);
    GitWriter
        .write(
            d,
            &Write::Rebase {
                branch: "other".into(),
                tip: other,
                onto: main,
            },
        )
        .unwrap();
    assert_eq!(git(d, &["status", "--porcelain"]), "", "applied cleanly");
    assert_eq!(
        std::fs::read_to_string(outside.0.join("f")).unwrap(),
        "SECRET\n",
        "nothing written through the committed link",
    );
    assert!(
        d.join("a")
            .symlink_metadata()
            .unwrap()
            .file_type()
            .is_symlink(),
        "the commit's own link is written as a link",
    );
}

#[test]
// Verifies: ADR-0033, ADR-0040, Q-6
fn a_merge_writing_a_conflict_through_a_lock_link_is_refused() {
    // libgit2 creates `<path>.lock` exclusively to write a conflicted file's merged content: a
    // link there is not followed, but stops the merge half done, so it is refused first.
    let (scratch, outside) = beside_outside("lock-link");
    let d = &scratch.0;
    write_file(d, "x", "base\n");
    commit_all(d, "x base");
    git(d, &["switch", "--quiet", "-c", "side"]);
    write_file(d, "x", "side\n");
    let side = commit_all(d, "x side");
    git(d, &["switch", "--quiet", "main"]);
    write_file(d, "x", "main\n");
    let main = commit_all(d, "x main");
    link(&outside.0.join("planted"), &d.join("x.lock"));
    let merge = Write::Merge {
        branch: "main".into(),
        tip: main,
        commit: side,
        name: None,
    };
    refused(
        d,
        &merge,
        "x.lock is in the way of a side of the conflict on x",
    );
    kept(&outside);
}

#[test]
// Verifies: ADR-0033, ADR-0040
fn a_rebase_labels_a_stopped_step_head_and_its_own_commit() {
    // libgit2 reads a rebase's labels again at each step, after the call that opened it
    // returned: the options must outlive the rebase, or the markers hold freed memory. Each step
    // is labelled `HEAD` and its own abbreviated commit, as `git` writes them.
    let scratch = with_file("rebase-markers");
    let d = &scratch.0;
    git(d, &["switch", "--quiet", "-c", "side"]);
    write_file(d, "f", "side\n");
    let side = commit_all(d, "side");
    git(d, &["switch", "--quiet", "main"]);
    write_file(d, "f", "main\n");
    let main = commit_all(d, "main");
    let short = git(d, &["rev-parse", "--short", &main]);
    let stopped = GitWriter
        .write(
            d,
            &Write::Rebase {
                branch: "main".into(),
                tip: main,
                onto: side,
            },
        )
        .unwrap();
    assert_eq!(stopped, None, "the rebase stopped on f");
    let f = std::fs::read_to_string(d.join("f")).unwrap();
    assert!(f.contains("<<<<<<< HEAD\n"), "{f:?}");
    assert!(f.contains(&format!(">>>>>>> {short}\n")), "{f:?}");
}

/// A repository beside a directory outside it where `main` and `side` each rename another file
/// to `c`, so that a merge of `side` writes each side of the collision beside `c`, under `c~`
/// and a label: (repository, outside, main, side).
fn rename_collision(name: &str) -> (Scratch, Scratch, String, String) {
    let (scratch, outside) = beside_outside(name);
    let d = &scratch.0;
    write_file(d, "a", &"alpha line\n".repeat(20));
    write_file(d, "b", &"beta line\n".repeat(20));
    commit_all(d, "a and b");
    git(d, &["switch", "--quiet", "-c", "side"]);
    git(d, &["mv", "b", "c"]);
    let side = commit_all(d, "b to c");
    git(d, &["switch", "--quiet", "main"]);
    git(d, &["mv", "a", "c"]);
    let main = commit_all(d, "a to c");
    (scratch, outside, main, side)
}

#[test]
// Verifies: ADR-0033, ADR-0040, ADR-0039, Q-6
fn a_link_beside_a_conflict_is_found_as_the_file_system_folds_its_name() {
    // Where case is not told apart, `C~HEAD` is the `c~HEAD` libgit2 writes a side of `c` to.
    let (scratch, outside, main, side) = rename_collision("folded-sibling");
    let d = &scratch.0;
    git(d, &["config", "core.ignorecase", "true"]);
    link(&outside.0.join("planted"), &d.join("C~HEAD"));
    let merge = Write::Merge {
        branch: "main".into(),
        tip: main,
        commit: side,
        name: None,
    };
    refused(
        d,
        &merge,
        "C~HEAD is in the way of a side of the conflict on c",
    );
    kept(&outside);
}

#[test]
// Verifies: ADR-0033, ADR-0040, Q-6
fn anything_at_a_conflicted_files_lock_refuses_the_write_before_it_stops_half_done() {
    // libgit2 creates `<path>.lock` exclusively to write the merged side: a file already there
    // fails it with the merge half done, so a plain file there refuses the write too.
    let scratch = with_file("lock-file");
    let d = &scratch.0;
    write_file(d, "x", "base\n");
    commit_all(d, "x base");
    git(d, &["switch", "--quiet", "-c", "side"]);
    write_file(d, "x", "side\n");
    let side = commit_all(d, "x side");
    git(d, &["switch", "--quiet", "main"]);
    write_file(d, "x", "main\n");
    let main = commit_all(d, "x main");
    write_file(d, "x.lock", "another program's\n");
    let merge = Write::Merge {
        branch: "main".into(),
        tip: main,
        commit: side,
        name: None,
    };
    refused(
        d,
        &merge,
        "x.lock is in the way of a side of the conflict on x",
    );
}

#[test]
// Verifies: ADR-0033, ADR-0040, G-5
fn a_criss_cross_merge_with_two_merge_bases_merges_as_git_merges_it() {
    // Two branches each merged into the other leave two merge bases; the writer reads every one,
    // and merges the tips as `git` does.
    let scratch = with_file("criss-cross");
    let d = &scratch.0;
    git(d, &["switch", "--quiet", "-c", "side"]);
    write_file(d, "s", "side\n");
    let side_one = commit_all(d, "side one");
    git(d, &["switch", "--quiet", "main"]);
    write_file(d, "m", "main\n");
    let main_one = commit_all(d, "main one");
    git(d, &["merge", "--quiet", "--no-edit", &side_one]);
    git(d, &["switch", "--quiet", "side"]);
    git(d, &["merge", "--quiet", "--no-edit", &main_one]);
    write_file(d, "s2", "side two\n");
    commit_all(d, "side two");
    git(d, &["switch", "--quiet", "main"]);
    write_file(d, "m2", "main two\n");
    commit_all(d, "main two");
    let bases = git(d, &["merge-base", "--all", "main", "side"]);
    assert_eq!(bases.lines().count(), 2, "a criss-cross: {bases}");
    let other = twin(&scratch, "criss-cross-git");
    let merge = Write::Merge {
        branch: "main".into(),
        tip: git(d, &["rev-parse", "main"]),
        commit: git(d, &["rev-parse", "side"]),
        name: Some("side".into()),
    };
    GitWriter.write(d, &merge).unwrap();
    git(&other.0, &["merge", "--quiet", "--no-edit", "side"]);
    assert_eq!(
        git(d, &["rev-parse", "HEAD^{tree}"]),
        git(&other.0, &["rev-parse", "HEAD^{tree}"]),
        "the merge's tree is git's"
    );
}
