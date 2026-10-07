//! Verifies: ADR-0033, G-7
//!
//! The writes of the repository port on repositories made with git: each does what it says, and
//! one whose expected state the repository does not hold is refused with nothing changed. Writing
//! runs none of the programs the repository names (Q-4).

#![cfg(unix)]

use ocelli_core::port::{RepositoryWriter, Write};
use ocelli_git::GitWriter;
use std::path::{Path, PathBuf};
use std::process::Command;

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
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=F", "-c", "user.email=f@example.invalid"])
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
    let before = tags(&scratch.0);
    let error = GitWriter
        .write(&scratch.0, &tag("v1", &two, None))
        .unwrap_err();
    assert!(error.to_string().contains("exists already"), "{error}");
    assert_eq!(tags(&scratch.0), before);
}

#[test]
fn a_tag_on_a_commit_no_longer_there_or_with_a_bad_name_is_refused() {
    let scratch = Scratch::new("refused");
    let (one, _) = repository(&scratch.0);
    let gone = "0123456789abcdef0123456789abcdef01234567";
    let error = GitWriter
        .write(&scratch.0, &tag("v1", gone, None))
        .unwrap_err();
    assert!(
        error.to_string().contains("no longer in the repository"),
        "{error}"
    );
    for bad in ["a..b", "-v1"] {
        let error = GitWriter
            .write(&scratch.0, &tag(bad, &one, None))
            .unwrap_err();
        assert!(error.to_string().contains("not a name"), "{error}");
    }
    assert_eq!(tags(&scratch.0), "");
}

#[test]
// Verifies: ADR-0033, Q-4
fn writing_a_repository_runs_none_of_its_programs() {
    let scratch = with_file("programs");
    let one = git(&scratch.0, &["rev-parse", "HEAD"]);
    // Outside the repository, so that stashing the untracked files leaves them be.
    let tools = Scratch::new("programs-ran");
    let markers = programs::set_everywhere(&scratch.0, &tools.0);

    GitWriter
        .write(&scratch.0, &tag("v1", &one, Some("release")))
        .unwrap();
    GitWriter.write(&scratch.0, &tag("v2", &one, None)).unwrap();
    std::fs::write(scratch.0.join("f"), "more\n").unwrap();
    std::fs::write(scratch.0.join("new"), "untracked\n").unwrap();
    GitWriter
        .write(&scratch.0, &create_stash(None, true))
        .unwrap();
    let mut stash = String::new();
    git2::Repository::open(&scratch.0)
        .unwrap()
        .stash_foreach(|_, _, oid| {
            stash = oid.to_string();
            true
        })
        .unwrap();
    GitWriter
        .write(&scratch.0, &apply(0, &stash, true))
        .unwrap();
    let ran = markers.ran();
    assert!(ran.is_empty(), "programs ran: {ran:?}");
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
    let before = tags(&scratch.0);
    for (write, why) in [
        (delete("v1", &one, false), "no longer the one shown"),
        (delete("v1", &two, true), "no longer the one shown"),
        (delete("gone", &two, false), "no tag named 'gone'"),
    ] {
        let error = GitWriter.write(&scratch.0, &write).unwrap_err();
        assert!(error.to_string().contains(why), "{error}");
    }
    assert_eq!(tags(&scratch.0), before);
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
    let error = GitWriter
        .write(&scratch.0, &create_stash(None, false))
        .unwrap_err();
    assert!(error.to_string().contains("nothing to stash"), "{error}");
    assert_eq!(stashes(&scratch.0), "");
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

    let error = GitWriter
        .write(&scratch.0, &apply(0, &stash, true))
        .unwrap_err();
    assert!(error.to_string().contains("conflicts"), "{error}");
    assert_eq!(
        status(&scratch.0),
        "",
        "not even the untracked file came back"
    );
    assert_eq!(stashes(&scratch.0).lines().count(), 1);
}

#[test]
fn a_stash_over_changes_of_the_working_directory_is_refused() {
    let scratch = with_file("overwrite");
    std::fs::write(scratch.0.join("f"), "stashed\n").unwrap();
    git(&scratch.0, &["stash", "push", "--quiet"]);
    let stash = stash_at(&scratch.0, 0);
    std::fs::write(scratch.0.join("f"), "mine\n").unwrap();

    let error = GitWriter
        .write(&scratch.0, &apply(0, &stash, false))
        .unwrap_err();
    assert!(
        error.to_string().contains("f has changes of its own"),
        "{error}"
    );
    assert_eq!(
        std::fs::read_to_string(scratch.0.join("f")).unwrap(),
        "mine\n"
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
    let before = stashes(&scratch.0);

    for write in [
        apply(0, &seen, false),
        Write::DropStash {
            index: 0,
            commit: seen.clone(),
        },
    ] {
        let error = GitWriter.write(&scratch.0, &write).unwrap_err();
        assert!(
            error.to_string().contains("no longer the stash shown"),
            "{error}"
        );
    }
    assert_eq!(stashes(&scratch.0), before);
    assert_eq!(status(&scratch.0), "");

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
/// and expects applying and popping the stash to be refused with `why` and nothing changed.
fn refused_after(name: &str, set_up: impl Fn(&Path), then: impl Fn(&Path), why: &str) {
    let scratch = with_file(name);
    set_up(&scratch.0);
    git(
        &scratch.0,
        &["stash", "push", "--quiet", "--include-untracked"],
    );
    let stash = stash_at(&scratch.0, 0);
    then(&scratch.0);
    let before = (status(&scratch.0), stashes(&scratch.0));
    let ignored = git(&scratch.0, &["status", "--porcelain", "--ignored"]);
    for drop in [false, true] {
        let error = GitWriter
            .write(&scratch.0, &apply(0, &stash, drop))
            .unwrap_err();
        assert!(error.to_string().contains(why), "{name}: {error}");
    }
    assert_eq!((status(&scratch.0), stashes(&scratch.0)), before, "{name}");
    assert_eq!(
        git(&scratch.0, &["status", "--porcelain", "--ignored"]),
        ignored,
        "{name}"
    );
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
    refused_after(
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
    refused_after(
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
    refused_after(
        "ignored-file",
        |d| write_file(d, ".env", "stashed\n"),
        |d| {
            write_file(d, ".git/info/exclude", ".env\n");
            write_file(d, ".env", "PRECIOUS\n");
        },
        ".env is not tracked",
    );
    // An untracked file the stash took along, and an ignored directory of that name.
    refused_after(
        "ignored-dir",
        |d| write_file(d, "build", "stashed\n"),
        |d| {
            write_file(d, ".git/info/exclude", "build/\n");
            write_file(d, "build/out.o", "object\n");
        },
        "build is a directory",
    );
    // An untracked file the stash took along, tracked by now.
    refused_after(
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
        refused_after(
            "case",
            |d| write_file(d, "Readme", "stashed\n"),
            |d| write_file(d, "README", "mine\n"),
            "Readme is not tracked",
        );
    } else {
        eprintln!("this file system tells Readme from README; the case-folding case is skipped");
    }
    // Changes staged in the index.
    refused_after(
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
    let made = Command::new("git")
        .arg("-C")
        .arg(dir)
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
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
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
