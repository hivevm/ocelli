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
    let scratch = Scratch::new("programs");
    let (one, _) = repository(&scratch.0);
    let markers = scratch.0.join("markers");
    std::fs::create_dir_all(&markers).unwrap();
    let hooks = scratch.0.join(".git/hooks");
    for hook in [
        "reference-transaction",
        "post-commit",
        "pre-commit",
        "post-checkout",
    ] {
        let path = hooks.join(hook);
        std::fs::write(
            &path,
            format!("#!/bin/sh\ntouch {}/{hook}\n", markers.display()),
        )
        .unwrap();
        Command::new("chmod").arg("+x").arg(&path).status().unwrap();
    }
    git(
        &scratch.0,
        &[
            "config",
            "core.fsmonitor",
            &format!("touch {}/fsmonitor", markers.display()),
        ],
    );
    // What `git tag -a` would run: a signing program.
    let gpg = scratch.0.join("gpg");
    std::fs::write(
        &gpg,
        format!("#!/bin/sh\ntouch {}/gpg\n", markers.display()),
    )
    .unwrap();
    Command::new("chmod").arg("+x").arg(&gpg).status().unwrap();
    git(&scratch.0, &["config", "tag.gpgSign", "true"]);
    git(
        &scratch.0,
        &["config", "gpg.program", gpg.to_str().unwrap()],
    );
    GitWriter
        .write(&scratch.0, &tag("v1", &one, Some("release")))
        .unwrap();
    GitWriter.write(&scratch.0, &tag("v2", &one, None)).unwrap();
    let ran: Vec<_> = std::fs::read_dir(&markers).unwrap().collect();
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
