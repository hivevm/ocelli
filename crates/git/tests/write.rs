//! Verifies: ADR-0033, G-7
//!
//! The writes of the repository port on repositories made with git: each does what it says, and
//! one whose expected state the repository does not hold is refused with nothing changed. Writing
//! runs none of the programs the repository names (Q-4).

#![cfg(unix)]

use ocelli_core::port::{RepositoryReader, RepositoryWriter, Write};
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
    let remote = Scratch::new("programs-remote");
    std::fs::remove_dir_all(&remote.0).unwrap();
    git(
        Path::new("/"),
        &[
            "clone",
            "--quiet",
            "--bare",
            scratch.0.to_str().unwrap(),
            remote.0.to_str().unwrap(),
        ],
    );
    git(
        &scratch.0,
        &["remote", "add", "origin", remote.0.to_str().unwrap()],
    );
    // Outside the repository, so that stashing the untracked files leaves them be.
    let tools = Scratch::new("programs-ran");
    let markers = programs::set_everywhere(&scratch.0, &tools.0);
    // Ocelli refuses what these would have git sign (ADR-0033); the writes here go through.
    let mut config = git2::Repository::open(&scratch.0)
        .unwrap()
        .config()
        .unwrap();
    for (key, _) in programs::SWITCHES {
        config.remove(key).unwrap();
    }

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
    GitWriter
        .write(&scratch.0, &stage_in(&scratch.0, "f"))
        .unwrap();
    GitWriter
        .write(&scratch.0, &unstage_in(&scratch.0, "f"))
        .unwrap();
    GitWriter
        .write(&scratch.0, &stage_in(&scratch.0, "f"))
        .unwrap();
    let made = GitWriter
        .write(&scratch.0, &commit_on(&one, "commit"))
        .unwrap()
        .unwrap();
    // A switch is where git runs post-checkout.
    let branch = Write::CreateBranch {
        name: "b".into(),
        commit: one.clone(),
        switch: true,
    };
    GitWriter.write(&scratch.0, &branch).unwrap();
    let back = Write::SwitchBranch {
        name: "main".into(),
        tip: made.clone(),
    };
    GitWriter.write(&scratch.0, &back).unwrap();
    GitWriter
        .write(
            &scratch.0,
            &Write::Detach {
                commit: one.clone(),
            },
        )
        .unwrap();
    let delete = Write::DeleteBranch {
        name: "b".into(),
        tip: one.clone(),
        merged: true,
    };
    GitWriter.write(&scratch.0, &delete).unwrap();
    let fetch = Write::Fetch {
        remote: "origin".into(),
    };
    GitWriter.write(&scratch.0, &fetch).unwrap();
    // A push is where git runs pre-push.
    let back = Write::SwitchBranch {
        name: "main".into(),
        tip: made.clone(),
    };
    GitWriter.write(&scratch.0, &back).unwrap();
    let push = Write::Push {
        branch: "main".into(),
        tip: made.clone(),
    };
    GitWriter.write(&scratch.0, &push).unwrap();
    // The push set main's upstream, so a pull fetches it and finds nothing to move.
    let pull = Write::Pull {
        branch: "main".into(),
        tip: made.clone(),
    };
    GitWriter.write(&scratch.0, &pull).unwrap();
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
    let error = GitWriter
        .write(&scratch.0, &stage_in(&scratch.0, "f"))
        .unwrap_err();
    assert!(error.to_string().contains("no change to stage"), "{error}");
    let error = GitWriter
        .write(&scratch.0, &unstage_in(&scratch.0, "f"))
        .unwrap_err();
    assert!(error.to_string().contains("not staged"), "{error}");
    assert_eq!(status(&scratch.0), "");
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
    let expected = Command::new("git")
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
    let error = GitWriter
        .write(&scratch.0, &commit_on(&head, "nothing"))
        .unwrap_err();
    assert!(error.to_string().contains("nothing is staged"), "{error}");

    write_file(&scratch.0, "f", "changed\n");
    GitWriter
        .write(&scratch.0, &stage_in(&scratch.0, "f"))
        .unwrap();
    let state = |d: &Path| {
        let ignored = git(d, &["status", "--porcelain", "--ignored"]);
        (staged_names(d), git(d, &["rev-parse", "HEAD"]), ignored)
    };
    let before = state(&scratch.0);
    let moved = "0123456789abcdef0123456789abcdef01234567";
    for (write, why) in [
        (commit_on(moved, "msg"), "no longer the one shown"),
        (commit_on(&head, "  \n"), "needs a message"),
    ] {
        let error = GitWriter.write(&scratch.0, &write).unwrap_err();
        assert!(error.to_string().contains(why), "{error}");
    }
    git(&scratch.0, &["config", "commit.gpgSign", "true"]);
    let error = GitWriter
        .write(&scratch.0, &commit_on(&head, "msg"))
        .unwrap_err();
    assert!(error.to_string().contains("commit.gpgSign"), "{error}");
    assert_eq!(state(&scratch.0), before);
}

#[test]
// Verifies: ADR-0033
fn an_annotated_tag_is_refused_where_tags_are_signed_and_a_lightweight_one_is_not() {
    let scratch = Scratch::new("tag-signed");
    let (one, _) = repository(&scratch.0);
    git(&scratch.0, &["config", "tag.gpgSign", "true"]);
    let error = GitWriter
        .write(&scratch.0, &tag("v1", &one, Some("release")))
        .unwrap_err();
    assert!(error.to_string().contains("tag.gpgSign"), "{error}");
    assert_eq!(tags(&scratch.0), "");
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
    let error = GitWriter.write(&scratch.0, &shown).unwrap_err();
    assert!(
        error.to_string().contains("changed since it was shown"),
        "{error}"
    );
    assert_eq!(staged_names(&scratch.0), "");

    GitWriter
        .write(&scratch.0, &stage_in(&scratch.0, "f"))
        .unwrap();
    let shown = unstage_in(&scratch.0, "f");
    write_file(&scratch.0, "f", "staged again\n");
    git(&scratch.0, &["add", "f"]);
    let error = GitWriter.write(&scratch.0, &shown).unwrap_err();
    assert!(
        error.to_string().contains("changed since it was shown"),
        "{error}"
    );
    assert_eq!(staged_names(&scratch.0), "M\tf");
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
    let before = git(&scratch.0, &["status", "--porcelain", "--ignored"]);
    let error = GitWriter
        .write(&scratch.0, &commit_on(&head, "merge"))
        .unwrap_err();
    assert!(
        error.to_string().contains("a merge is in progress"),
        "{error}"
    );
    assert_eq!(git(&scratch.0, &["rev-parse", "HEAD"]), head);
    assert!(scratch.0.join(".git/MERGE_HEAD").exists());
    assert_eq!(
        git(&scratch.0, &["status", "--porcelain", "--ignored"]),
        before
    );
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
    let error = GitWriter
        .write(&scratch.0, &tag("v1", &one, Some("release")))
        .unwrap_err();
    assert!(
        error.to_string().contains("tag.forceSignAnnotated"),
        "{error}"
    );
    assert_eq!(tags(&scratch.0), "");
}

/// HEAD as `git` reads it: the branch it is on, or the commit it is detached at.
fn head_of(dir: &Path) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
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

fn everything(dir: &Path) -> (String, String, String) {
    let status = git(dir, &["status", "--porcelain", "--ignored"]);
    (
        head_of(dir),
        status,
        git(
            dir,
            &[
                "branch",
                "--list",
                "--format=%(refname:short) %(objectname)",
            ],
        ),
    )
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

    let before = everything(&scratch.0);
    for (name, why) in [
        ("here", "exists already"),
        ("-x", "not a name"),
        ("HEAD", "not a name"),
        ("a..b", "not a name"),
    ] {
        let error = GitWriter
            .write(&scratch.0, &create(name, &main, true))
            .unwrap_err();
        assert!(error.to_string().contains(why), "{name}: {error}");
    }
    assert_eq!(everything(&scratch.0), before);
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
    let before = everything(&scratch.0);
    let error = GitWriter
        .write(&scratch.0, &switch("main", &main))
        .unwrap_err();
    assert!(
        error.to_string().contains("changes to f are in the way"),
        "{error}"
    );
    let error = GitWriter
        .write(&scratch.0, &switch("main", &side))
        .unwrap_err();
    assert!(
        error.to_string().contains("no longer where it was shown"),
        "{error}"
    );
    assert_eq!(everything(&scratch.0), before);
    assert_eq!(
        std::fs::read_to_string(scratch.0.join("f")).unwrap(),
        "my change\n"
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

#[test]
// Verifies: ADR-0033, G-4
fn a_remote_branch_is_switched_to_through_a_local_branch_that_tracks_it() {
    let (origin, _, side) = with_side("remote-origin");
    let clone = Scratch::new("remote-clone");
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
    let error = GitWriter.write(&clone.0, &track).unwrap_err();
    assert!(error.to_string().contains("exists already"), "{error}");
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
    let before = everything(&scratch.0);
    for (write, why) in [
        (
            delete("side", &side, true),
            "whether 'side' is merged changed",
        ),
        (delete("side", &main, false), "no longer where it was shown"),
        (delete("main", &main, true), "is checked out"),
    ] {
        let error = GitWriter.write(&scratch.0, &write).unwrap_err();
        assert!(error.to_string().contains(why), "{error}");
    }
    assert_eq!(everything(&scratch.0), before);
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

/// What a refused write must leave as it was: HEAD, the branches, the working directory's status
/// with ignored files, and the index's entries.
fn all_of(dir: &Path) -> (String, String, String, String) {
    let (head, status, branches) = everything(dir);
    (head, status, branches, git(dir, &["ls-files", "--stage"]))
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
    let before = all_of(&scratch.0);
    let error = GitWriter.write(&scratch.0, &switch).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("staged changes to f are in the way"),
        "{error}"
    );
    assert_eq!(all_of(&scratch.0), before);
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
    let (scratch, main, side) = with_side("worktree");
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
    let before = all_of(&scratch.0);
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
        let error = GitWriter.write(&scratch.0, &write).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("checked out in the working tree"),
            "{error}"
        );
    }
    assert_eq!(all_of(&scratch.0), before);
    let _ = main;
}

#[test]
// Verifies: ADR-0033, G-4
fn a_branch_whose_name_clashes_with_another_is_refused_with_nothing_changed() {
    let (scratch, main, side) = with_side("clash");
    git(&scratch.0, &["branch", "feature/x", &main]);
    let before = all_of(&scratch.0);
    let create = Write::CreateBranch {
        name: "feature".into(),
        commit: side.clone(),
        switch: true,
    };
    let error = GitWriter.write(&scratch.0, &create).unwrap_err();
    assert!(
        error.to_string().contains("cannot be created here"),
        "{error}"
    );
    assert_eq!(all_of(&scratch.0), before);

    // A switch refused after the branch was created takes the branch away again.
    write_file(&scratch.0, "f", "mine\n");
    let before = all_of(&scratch.0);
    let create = Write::CreateBranch {
        name: "topic".into(),
        commit: side.clone(),
        switch: true,
    };
    let error = GitWriter.write(&scratch.0, &create).unwrap_err();
    assert!(error.to_string().contains("in the way"), "{error}");
    assert_eq!(all_of(&scratch.0), before);
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
    let before = all_of(&clone.0);
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
    assert_eq!(all_of(&clone.0), before, "only the remote branches move");
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
    let origin = git(&clone.0, &["rev-parse", "origin/main"]);
    let fetch = |name: &str| Write::Fetch {
        remote: name.into(),
    };
    let error = GitWriter
        .write_reporting(&clone.0, &fetch("origin"), &mut |_| false)
        .unwrap_err();
    assert!(error.to_string().contains("cancelled"), "{error}");
    let error = GitWriter.write(&clone.0, &fetch("nowhere")).unwrap_err();
    assert!(error.to_string().contains("no remote named"), "{error}");
    assert_eq!(git(&clone.0, &["rev-parse", "origin/main"]), origin);
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
    let before = all_of(&clone.0);
    GitWriter
        .write(&clone.0, &pull_of(&clone.0, "main"))
        .unwrap();
    assert_eq!(all_of(&clone.0), before);
}

#[test]
// Verifies: ADR-0035, G-4
fn a_pull_is_refused_where_the_branch_diverged_has_no_upstream_or_is_not_checked_out() {
    let (_remote, clone, later) = with_remote("pull-refused");
    write_file(&clone.0, "f", "mine\n");
    git(&clone.0, &["commit", "--quiet", "-am", "mine"]);
    let before = all_of(&clone.0);
    let error = GitWriter
        .write(&clone.0, &pull_of(&clone.0, "main"))
        .unwrap_err();
    assert!(error.to_string().contains("have diverged"), "{error}");
    assert_eq!(all_of(&clone.0), before);
    assert_eq!(
        git(&clone.0, &["rev-parse", "origin/main"]),
        later,
        "the fetch was done"
    );

    git(&clone.0, &["branch", "loose"]);
    git(&clone.0, &["switch", "--quiet", "loose"]);
    let error = GitWriter
        .write(&clone.0, &pull_of(&clone.0, "loose"))
        .unwrap_err();
    assert!(error.to_string().contains("no upstream"), "{error}");
    let error = GitWriter
        .write(&clone.0, &pull_of(&clone.0, "main"))
        .unwrap_err();
    assert!(error.to_string().contains("not checked out"), "{error}");
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
    let before = all_of(&clone.0);
    let error = GitWriter
        .write(&clone.0, &push_of(&clone.0, "main"))
        .unwrap_err();
    assert!(error.to_string().contains("pull first"), "{error}");
    assert_eq!(
        git(&remote.0, &["rev-parse", "main"]),
        later,
        "the remote kept its branch"
    );
    assert_eq!(all_of(&clone.0), before);
}

#[test]
// Verifies: ADR-0035, G-4
fn a_commit_made_while_a_pull_fetches_is_neither_lost_nor_checked_out_over() {
    let (_remote, clone, _) = with_remote("pull-race");
    let pull = pull_of(&clone.0, "main");
    let dir = clone.0.clone();
    let mut committed = false;
    let error = GitWriter
        .write_reporting(&clone.0, &pull, &mut |_| {
            if !committed {
                committed = true;
                write_file(&dir, "x", "made meanwhile\n");
                git(&dir, &["add", "x"]);
                git(&dir, &["commit", "--quiet", "-m", "meanwhile"]);
            }
            true
        })
        .unwrap_err();
    assert!(
        error.to_string().contains("no longer where it was shown"),
        "{error}"
    );
    assert_eq!(git(&clone.0, &["log", "-1", "--format=%s"]), "meanwhile");
    assert_eq!(
        std::fs::read_to_string(clone.0.join("x")).unwrap(),
        "made meanwhile\n"
    );
}

#[test]
// Verifies: ADR-0035
fn a_pull_whose_branch_another_program_holds_locked_changes_nothing() {
    let (_remote, clone, _) = with_remote("pull-locked");
    let pull = pull_of(&clone.0, "main");
    let lock = clone.0.join(".git/refs/heads/main.lock");
    std::fs::write(&lock, "").unwrap();
    let before = all_of(&clone.0);
    let error = GitWriter.write(&clone.0, &pull).unwrap_err();
    assert!(
        error.to_string().contains("locked by another program"),
        "{error}"
    );
    std::fs::remove_file(&lock).unwrap();
    assert_eq!(all_of(&clone.0), before);
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
    let error = GitWriter
        .write(&clone.0, &push_of(&clone.0, "feat"))
        .unwrap_err();
    assert!(error.to_string().contains("named differently"), "{error}");
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
        let error = GitWriter
            .write(&clone.0, &push_of(&clone.0, "feat"))
            .unwrap_err();
        assert!(error.to_string().contains(why), "{mode}: {error}");
    }

    // An upstream on a local branch is no remote to push to.
    git(&clone.0, &["config", "--unset", "push.default"]);
    git(
        &clone.0,
        &["branch", "--quiet", "--set-upstream-to", "main"],
    );
    let error = GitWriter
        .write(&clone.0, &push_of(&clone.0, "feat"))
        .unwrap_err();
    assert!(error.to_string().contains("local branch"), "{error}");
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
