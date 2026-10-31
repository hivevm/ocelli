//! Verifies: ADR-draft-merge-editor
//!
//! A write `git` would sign is refused where the global configuration asks for a signature, as
//! where the repository's does. libgit2 reads the global configuration from its search path,
//! which holds for the whole process: this test runs in a binary of its own.

#![allow(
    clippy::disallowed_methods,
    reason = "a test sets a repository up by hand, the way a developer would"
)]
#![cfg(unix)]

mod git_command;
use git_command::git_command;
use git2::ConfigLevel;
use ocelli_core::port::{RepositoryWriter, Write};
use ocelli_git::GitWriter;

#[test]
#[allow(unsafe_code)]
fn a_commit_and_an_annotated_tag_are_refused_where_the_global_configuration_signs() {
    let dir = std::env::temp_dir().join(format!("ocelli-write-signing-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let global = dir.join("global");
    let repo = dir.join("repo");
    std::fs::create_dir_all(&global).unwrap();
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::write(
        global.join(".gitconfig"),
        "[user]\n\tname = Ada\n\temail = ada@example.invalid\n\
         [commit]\n\tgpgSign = true\n[tag]\n\tgpgSign = true\n",
    )
    .unwrap();
    for level in [ConfigLevel::Global, ConfigLevel::XDG, ConfigLevel::System] {
        // SAFETY: this binary holds this one test, and nothing else uses libgit2 while it runs.
        unsafe { git2::opts::set_search_path(level, &global) }.unwrap();
    }
    let git = |args: &[&str]| {
        let out = git_command(&repo)
            .args(["-c", "user.name=F", "-c", "user.email=f@example.invalid"])
            .args(["-c", "commit.gpgSign=false"])
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    };
    git(&["init", "--quiet", "-b", "main"]);
    git(&["commit", "--quiet", "--allow-empty", "-m", "one"]);
    let head = git(&["rev-parse", "HEAD"]);
    std::fs::write(repo.join("f"), "staged\n").unwrap();
    git(&["add", "f"]);

    let commit = Write::Commit {
        message: "two".into(),
        head: Some(head.clone()),
    };
    let error = GitWriter.write(&repo, &commit).unwrap_err();
    assert!(error.to_string().contains("commit.gpgSign"), "{error}");
    let tag = Write::CreateTag {
        name: "v1".into(),
        commit: head.clone(),
        message: Some("release".into()),
    };
    let error = GitWriter.write(&repo, &tag).unwrap_err();
    assert!(error.to_string().contains("tag.gpgSign"), "{error}");
    assert_eq!(git(&["rev-parse", "HEAD"]), head);
    assert_eq!(git(&["tag", "--list"]), "");
    let _ = std::fs::remove_dir_all(&dir);
}
