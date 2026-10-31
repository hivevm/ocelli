//! Verifies: ADR-0037, ADR-0035
//!
//! Confined to a folder, as the remote server confines it, the adapter fetches from and pushes to
//! a repository on this machine only where it lies below the folder: libgit2's local transport
//! opens a remote named by a path or a `file://` address in-process, so a fetch from one outside
//! would copy its objects inside, where the browser reads them, and a push would write it. The
//! address checked is the one libgit2 uses, `insteadOf` and `pushInsteadOf` applied. The test
//! runs in a binary of its own, since the confinement holds for the whole process.

#![allow(
    clippy::disallowed_methods,
    reason = "a test sets a repository up by hand, the way a developer would"
)]
// A `file://` address is spelled as unix spells an absolute path.
#![cfg(unix)]

mod support;
use ocelli_core::port::{RepositoryWriter, Write};
use ocelli_git::GitWriter;
use std::path::Path;
use support::{Scratch, git, refused, write_file};

const OUTSIDE: &str = "outside the server root";

fn fetch(remote: &str) -> Write {
    Write::Fetch {
        remote: remote.into(),
    }
}

fn push(repo: &Path, to: &str) -> Write {
    git(repo, &["config", "branch.main.pushRemote", to]);
    Write::Push {
        branch: "main".into(),
        tip: git(repo, &["rev-parse", "HEAD"]),
    }
}

/// The refs of the bare repository `dir`.
fn refs(dir: &Path) -> String {
    git(dir, &["for-each-ref"])
}

#[test]
fn confined_a_remote_on_this_machine_is_reached_only_below_the_root() {
    let scratch = Scratch::new("confined-remotes");
    let base = scratch.0.canonicalize().unwrap();
    let root = base.join("root");
    let (repo, inside, outside) = (
        root.join("repo"),
        root.join("inside.git"),
        base.join("outside.git"),
    );
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "--quiet", "-b", "main"]);
    git(&repo, &["config", "user.name", "Ada"]);
    git(&repo, &["config", "user.email", "ada@example.invalid"]);
    write_file(&repo, "f", "one\n");
    git(&repo, &["add", "f"]);
    git(&repo, &["commit", "--quiet", "-m", "one"]);
    for bare in [&inside, &outside] {
        git(
            &base,
            &[
                "init",
                "--quiet",
                "--bare",
                "-b",
                "main",
                bare.to_str().unwrap(),
            ],
        );
        git(&repo, &["push", "--quiet", bare.to_str().unwrap(), "main"]);
    }
    // Something to fetch from each, which a fetch would copy in.
    let seed = base.join("seed");
    git(
        &base,
        &["clone", "--quiet", outside.to_str().unwrap(), "seed"],
    );
    write_file(&seed, "g", "outside\n");
    git(&seed, &["add", "g"]);
    git(
        &seed,
        &[
            "-c",
            "user.name=A",
            "-c",
            "user.email=a@b",
            "commit",
            "--quiet",
            "-m",
            "two",
        ],
    );
    git(
        &seed,
        &["push", "--quiet", "origin", "main", "main:refs/heads/x"],
    );
    git(
        &seed,
        &[
            "push",
            "--quiet",
            inside.to_str().unwrap(),
            "main:refs/heads/x",
        ],
    );
    // A local commit, which a push would send.
    write_file(&repo, "f", "two\n");
    git(&repo, &["commit", "--quiet", "-am", "mine"]);

    let out = outside.to_str().unwrap();
    let file_out = format!("file://{out}");
    let inn = inside.to_str().unwrap();
    let file_in = format!("file://{inn}");
    // The same repository under another spelling, which only `pushy` uses.
    let also_in = format!("{}/./inside.git", root.display());
    for (name, url) in [
        ("out", out),
        ("fileout", file_out.as_str()),
        ("relative", "../../outside.git"),
        ("in", inn),
        ("filein", file_in.as_str()),
        ("sneaky", "sneaky:"),
        ("pushy", also_in.as_str()),
    ] {
        git(&repo, &["remote", "add", name, url]);
    }
    // `sneaky:` reads as `outside` for every transfer, and `pushy` only for a push.
    git(
        &repo,
        &["config", &format!("url.{out}.insteadOf"), "sneaky:"],
    );
    git(
        &repo,
        &["config", &format!("url.{out}.pushInsteadOf"), &also_in],
    );

    ocelli_git::confine::confine_to(&root);
    let before = refs(&outside);
    for name in ["out", "fileout", "relative", "sneaky"] {
        refused(&repo, &fetch(name), OUTSIDE);
        let pushed = push(&repo, name);
        refused(&repo, &pushed, OUTSIDE);
    }
    let pushed = push(&repo, "pushy");
    refused(&repo, &pushed, OUTSIDE);
    assert_eq!(
        refs(&outside),
        before,
        "the repository outside is left alone"
    );

    // Below the root, both are reached.
    for name in ["in", "filein", "pushy"] {
        GitWriter.write(&repo, &fetch(name)).unwrap();
        assert_eq!(
            git(&repo, &["rev-parse", &format!("{name}/x")]),
            git(&inside, &["rev-parse", "x"]),
            "{name}"
        );
    }
    for name in ["in", "filein"] {
        let pushed = push(&repo, name);
        GitWriter.write(&repo, &pushed).unwrap();
        assert_eq!(
            git(&inside, &["rev-parse", "main"]),
            git(&repo, &["rev-parse", "main"])
        );
    }
}
