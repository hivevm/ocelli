//! Verifies: ADR-0031, G-15
//!
//! The merge editor's file read from a conflict `git` left, and its result saved as the file's
//! resolution, on repositories made with git: the read in each conflict style, the conflicts it
//! does not open, the save compared with `git add` of the same file on a twin, and each refusal
//! leaving the repository as it was.
#![allow(
    clippy::disallowed_methods,
    reason = "a test sets a repository up by hand, the way a developer would"
)]
#![cfg(unix)]

mod support;

use ocelli_core::port::{RepositoryReader, RepositoryWriter, Write};
use ocelli_git::{GitReader, GitWriter};
use std::path::Path;
use support::{Origin, Scratch, assert_twins_alike, git, git_may_fail, refused, twin, write_file};

/// A repository whose `main` and `side` both start from `f` = `base`; `main` then writes `ours`
/// to it, `side` `theirs`, `None` deleting it. Identity in its configuration.
fn diverged(name: &str, base: &[u8], ours: Option<&[u8]>, theirs: Option<&[u8]>) -> Scratch {
    let s = Scratch::new(name);
    let d = &s.0;
    git(d, &["init", "--quiet", "-b", "main"]);
    git(d, &["config", "user.name", "Ada"]);
    git(d, &["config", "user.email", "ada@example.invalid"]);
    std::fs::write(d.join("f"), base).unwrap();
    git(d, &["add", "f"]);
    git(d, &["commit", "--quiet", "-m", "base"]);
    for (branch, content) in [("side", theirs), ("main", ours)] {
        if branch == "side" {
            git(d, &["switch", "--quiet", "-c", "side"]);
        } else {
            git(d, &["switch", "--quiet", "main"]);
        }
        match content {
            Some(c) => {
                std::fs::write(d.join("f"), c).unwrap();
                git(d, &["add", "f"]);
            }
            None => {
                git(d, &["rm", "--quiet", "f"]);
            }
        }
        git(d, &["commit", "--quiet", "-m", branch]);
    }
    s
}

/// [`diverged`] with text on both sides, merged in the conflict style `style`.
fn conflicted(name: &str, style: &str) -> Scratch {
    let repo = diverged(
        name,
        b"a\nbase\nz\n",
        Some(b"a\nours\nz\n"),
        Some(b"a\ntheirs\nz\n"),
    );
    git(&repo.0, &["config", "merge.conflictStyle", style]);
    assert!(!git_may_fail(&repo.0, &["merge", "side"]));
    repo
}

fn editable(dir: &Path) -> Vec<String> {
    GitReader
        .load_refs(dir)
        .unwrap()
        .in_progress
        .unwrap()
        .editable
}

fn save(dir: &Path, result: &str) -> Write {
    let opened = GitReader.load_merge_file(dir, "f").unwrap();
    Write::SaveMerge {
        path: "f".into(),
        conflict: GitReader
            .load_refs(dir)
            .unwrap()
            .in_progress
            .unwrap()
            .conflicts
            .into_iter()
            .find(|c| c.path == "f")
            .unwrap(),
        opened: opened.blob,
        result: result.into(),
    }
}

#[test]
fn a_conflict_in_each_style_is_read_as_the_file_with_its_blob_and_blocks() {
    for (style, has_base) in [("merge", false), ("diff3", true), ("zdiff3", true)] {
        let repo = conflicted(&format!("read-{style}"), style);
        assert_eq!(editable(&repo.0), ["f"], "{style}");
        let file = GitReader.load_merge_file(&repo.0, "f").unwrap();
        assert_eq!(
            file.text,
            std::fs::read_to_string(repo.0.join("f")).unwrap(),
            "{style}"
        );
        assert_eq!(file.blob, git(&repo.0, &["hash-object", "f"]), "{style}");
        assert_eq!(file.blocks.len(), 1, "{style}");
        assert_eq!(file.blocks[0].ours, "ours\n", "{style}");
        assert_eq!(file.blocks[0].theirs, "theirs\n", "{style}");
        assert_eq!(file.blocks[0].base.is_some(), has_base, "{style}");
    }
}

#[test]
fn a_file_edited_since_the_merge_stopped_is_read_as_it_is_now() {
    let repo = conflicted("read-edited", "merge");
    write_file(&repo.0, "f", "edited\n");
    let file = GitReader.load_merge_file(&repo.0, "f").unwrap();
    assert_eq!(file.text, "edited\n");
    assert!(file.blocks.is_empty());
}

#[test]
fn a_deleted_side_a_binary_file_a_link_and_a_submodule_are_not_opened() {
    let deleted = diverged("deleted", b"base\n", Some(b"ours\n"), None);
    let nul = diverged("nul", b"base\n", Some(b"ours\0\n"), Some(b"theirs\n"));
    let latin = diverged("latin", b"base\n", Some(b"ours\n"), Some(b"th\xe9irs\n"));
    let mut cases = vec![("deleted", deleted), ("nul", nul), ("latin", latin)];

    // A link on each side, pointing at different files.
    let link = Scratch::new("link");
    let d = &link.0;
    git(d, &["init", "--quiet", "-b", "main"]);
    write_file(d, "g", "g\n");
    git(d, &["add", "g"]);
    git(d, &["commit", "--quiet", "-m", "base"]);
    for (branch, target) in [("side", "a"), ("main", "b")] {
        if branch == "side" {
            git(d, &["switch", "--quiet", "-c", "side"]);
        } else {
            git(d, &["switch", "--quiet", "main"]);
        }
        std::os::unix::fs::symlink(target, d.join("f")).unwrap();
        git(d, &["add", "f"]);
        git(d, &["commit", "--quiet", "-m", branch]);
    }
    cases.push(("link", link));

    // A submodule's commit on each side, a different one, which git cannot merge.
    let sub = Scratch::new("submodule");
    let d = &sub.0;
    git(d, &["init", "--quiet", "-b", "main"]);
    write_file(d, "g", "g\n");
    git(d, &["add", "g"]);
    git(d, &["commit", "--quiet", "-m", "one"]);
    let one = git(d, &["rev-parse", "HEAD"]);
    git(d, &["commit", "--quiet", "--allow-empty", "-m", "two"]);
    let two = git(d, &["rev-parse", "HEAD"]);
    for (branch, pinned) in [("side", &one), ("main", &two)] {
        if branch == "side" {
            git(d, &["switch", "--quiet", "-c", "side"]);
        } else {
            git(d, &["switch", "--quiet", "main"]);
        }
        git(
            d,
            &[
                "update-index",
                "--add",
                "--cacheinfo",
                &format!("160000,{pinned},f"),
            ],
        );
        git(d, &["commit", "--quiet", "-m", branch]);
    }
    cases.push(("submodule", sub));

    for (what, repo) in &cases {
        assert!(!git_may_fail(&repo.0, &["merge", "side"]), "{what}");
        assert!(
            git(&repo.0, &["diff", "--name-only", "--diff-filter=U"]).contains('f'),
            "{what} conflicts"
        );
        assert!(editable(&repo.0).is_empty(), "{what}");
        let refused = GitReader.load_merge_file(&repo.0, "f").unwrap_err();
        assert!(
            refused.to_string().contains("ours, theirs, or as edited"),
            "{what}: {refused}"
        );
    }
}

#[test]
fn a_result_saved_resolves_the_file_as_git_add_of_it_does() {
    let repo = conflicted("save", "diff3");
    let origin = Origin::of(&repo.0);
    let other = twin(&repo, "save-twin");
    GitWriter
        .write(&repo.0, &save(&repo.0, "a\nours and theirs\nz\n"))
        .unwrap();
    write_file(&other.0, "f", "a\nours and theirs\nz\n");
    git(&other.0, &["add", "f"]);
    assert_twins_alike(&repo.0, &other.0, &origin, "save");
    assert_eq!(
        git(&repo.0, &["diff", "--name-only", "--diff-filter=U"]),
        ""
    );
}

#[test]
fn a_result_saved_keeps_the_line_endings_the_file_had() {
    let repo = diverged(
        "crlf",
        b"a\r\nbase\r\n",
        Some(b"a\r\nours\r\n"),
        Some(b"a\r\ntheirs\r\n"),
    );
    assert!(!git_may_fail(&repo.0, &["merge", "side"]));
    let origin = Origin::of(&repo.0);
    let other = twin(&repo, "crlf-twin");
    // An editor ends its lines with `\n`.
    GitWriter
        .write(&repo.0, &save(&repo.0, "a\nboth\n"))
        .unwrap();
    assert_eq!(std::fs::read(repo.0.join("f")).unwrap(), b"a\r\nboth\r\n");
    std::fs::write(other.0.join("f"), b"a\r\nboth\r\n").unwrap();
    git(&other.0, &["add", "f"]);
    assert_twins_alike(&repo.0, &other.0, &origin, "crlf");
}

#[test]
fn a_result_with_a_marker_a_file_changed_since_or_a_conflict_moved_on_is_refused() {
    let repo = conflicted("refuse-marker", "merge");
    let marked = save(&repo.0, "a\n=======\nz\n");
    refused(&repo.0, &marked, "conflict marker");

    let repo = conflicted("refuse-changed", "merge");
    let stale = save(&repo.0, "a\nboth\nz\n");
    write_file(&repo.0, "f", "edited elsewhere\n");
    refused(&repo.0, &stale, "since the merge editor opened it");

    let repo = conflicted("refuse-moved-on", "merge");
    let stale = save(&repo.0, "a\nboth\nz\n");
    git(&repo.0, &["add", "f"]);
    refused(&repo.0, &stale, "no longer conflicts");
}
