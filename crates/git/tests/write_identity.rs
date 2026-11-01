//! Verifies: ADR-0031
//!
//! A write that records a person refuses where no identity is configured. libgit2 looks for the
//! identity in the global and system configuration too, which `git_command` puts out of reach of
//! every test.

#![allow(
    clippy::disallowed_methods,
    reason = "a test sets a repository up by hand, the way a developer would"
)]
#![cfg(unix)]

mod git_command;
use git_command::git_command;
use ocelli_core::port::{RepositoryWriter, Write};
use ocelli_git::GitWriter;

#[test]
fn an_annotated_tag_without_an_identity_is_refused_with_nothing_changed() {
    let dir = std::env::temp_dir().join(format!("ocelli-write-identity-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let git = |args: &[&str]| {
        let out = git_command(&repo)
            .args(["-c", "user.name=F", "-c", "user.email=f@example.invalid"])
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
    let one = git(&["rev-parse", "HEAD"]);

    let write = Write::CreateTag {
        name: "v1".into(),
        commit: one,
        message: Some("release".into()),
    };
    let error = GitWriter.write(&repo, &write).unwrap_err();
    assert!(error.to_string().contains("no identity"), "{error}");
    assert_eq!(git(&["tag", "--list"]), "");
    let _ = std::fs::remove_dir_all(&dir);
}
