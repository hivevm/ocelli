//! Verifies: ADR-0033
//!
//! A write that records a person refuses where no identity is configured. libgit2 looks for the
//! identity in the global and system configuration too, so this test points those at an empty
//! directory, which holds for the whole process: it runs in a binary of its own.

#![cfg(unix)]

use git2::ConfigLevel;
use ocelli_core::port::{RepositoryWriter, Write};
use ocelli_git::GitWriter;
use std::process::Command;

#[test]
#[allow(unsafe_code)]
fn an_annotated_tag_without_an_identity_is_refused_with_nothing_changed() {
    let dir = std::env::temp_dir().join(format!("ocelli-write-identity-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let empty = dir.join("no-config");
    let repo = dir.join("repo");
    std::fs::create_dir_all(&empty).unwrap();
    std::fs::create_dir_all(&repo).unwrap();
    for level in [ConfigLevel::Global, ConfigLevel::XDG, ConfigLevel::System] {
        // SAFETY: this binary holds this one test, and nothing else uses libgit2 while it runs.
        unsafe { git2::opts::set_search_path(level, &empty) }.unwrap();
    }
    let git = |args: &[&str]| {
        let out = Command::new("git")
            .arg("-C")
            .arg(&repo)
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
