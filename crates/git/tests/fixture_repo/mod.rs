//! Builds a small, fully deterministic repository with git2 for the loading reference test.
//! The same builder produced `fixtures/repo-*.json` from GitGraph's `load_repository`,
//! `load_repo_refs`, `load_commit_files`, and `load_file_diff` at commit 9dd6ec9 of `main`, by
//! the steps of `load_reference.rs` with each result written as pretty-printed JSON.

use git2::{Oid, Repository, Signature, Time};
use std::path::Path;

fn sig(name: &str, t: i64) -> Signature<'static> {
    Signature::new(
        name,
        &format!("{}@example.org", name.to_lowercase()),
        &Time::new(t, 0),
    )
    .unwrap()
}

fn commit(
    repo: &Repository,
    refname: Option<&str>,
    file: &str,
    body: &str,
    msg: &str,
    t: i64,
    parents: &[Oid],
) -> Oid {
    std::fs::write(repo.workdir().unwrap().join(file), body).unwrap();
    let mut index = repo.index().unwrap();
    index.add_path(Path::new(file)).unwrap();
    index.write().unwrap();
    let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
    let ps: Vec<_> = parents
        .iter()
        .map(|p| repo.find_commit(*p).unwrap())
        .collect();
    let pr: Vec<_> = ps.iter().collect();
    let s = sig(if t % 2 == 0 { "Ada" } else { "Linus" }, t);
    repo.commit(refname, &s, &s, msg, &tree, &pr).unwrap()
}

pub fn build(dir: &Path) -> Repository {
    crate::support::git_command::libgit2_without_developer_configuration();
    let mut opts = git2::RepositoryInitOptions::new();
    opts.initial_head("main");
    let repo = Repository::init_opts(dir, &opts).unwrap();
    {
        let mut cfg = repo.config().unwrap();
        cfg.set_str("user.name", "Ada").unwrap();
        cfg.set_str("user.email", "ada@example.org").unwrap();
    }
    let t0 = 1_700_000_000;
    let a = commit(&repo, Some("HEAD"), "a.txt", "one\n", "first", t0, &[]);
    let b = commit(
        &repo,
        Some("HEAD"),
        "a.txt",
        "one\ntwo\n",
        "second",
        t0 + 100,
        &[a],
    );
    let f1 = commit(
        &repo,
        None,
        "f.txt",
        "feature\n",
        "feature work",
        t0 + 150,
        &[b],
    );
    let f2 = commit(
        &repo,
        None,
        "f.txt",
        "feature\nmore\n",
        "more feature",
        t0 + 250,
        &[f1],
    );
    repo.reference("refs/heads/feature", f2, true, "").unwrap();
    let c = commit(
        &repo,
        Some("HEAD"),
        "b.txt",
        "bee\n",
        "third",
        t0 + 200,
        &[b],
    );
    // The merge's tree: main's files plus the feature file.
    let m = commit(
        &repo,
        Some("HEAD"),
        "f.txt",
        "feature\nmore\n",
        "Merge feature",
        t0 + 300,
        &[c, f2],
    );
    let h1 = commit(&repo, None, "h.txt", "fix\n", "hotfix", t0 + 320, &[m]);
    repo.reference("refs/heads/hotfix", h1, true, "").unwrap();
    repo.reference("refs/remotes/origin/main", m, true, "")
        .unwrap();
    {
        let mc = repo.find_commit(m).unwrap();
        repo.tag(
            "v1.0",
            mc.as_object(),
            &sig("Ada", t0 + 310),
            "release",
            false,
        )
        .unwrap();
        let bc = repo.find_commit(b).unwrap();
        repo.tag_lightweight("light", bc.as_object(), false)
            .unwrap();
    }
    let d = commit(
        &repo,
        Some("HEAD"),
        "a.txt",
        "one\ntwo\nthree\n",
        "fourth",
        t0 + 400,
        &[m],
    );
    let _ = d;
    // Restore the workdir to HEAD, then stash a change.
    repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
        .unwrap();
    std::fs::write(dir.join("a.txt"), "stashed\n").unwrap();
    let mut repo = repo;
    repo.stash_save(&sig("Ada", t0 + 500), "WIP stash", None)
        .unwrap();
    repo
}
