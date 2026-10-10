//! The writes a test makes as the web interface makes them, from what the repository shows now:
//! the content of a file in the working directory and in the index, and every kind of write the
//! core takes. Shared by the writer's tests and the remote server's.

// A test binary uses what it needs of the module and leaves the rest.
#![allow(dead_code)]

use ocelli_core::port::Write;
use std::path::Path;

/// The content of `path` in the working directory and in the index, read with git2, which runs
/// none of the programs the repository names, as the git binary would.
pub fn shown_in(dir: &Path, path: &str) -> (Option<String>, Option<String>) {
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
pub fn stage_in(dir: &Path, path: &str) -> Write {
    Write::StageFile {
        path: path.to_string(),
        shown: shown_in(dir, path).0,
    }
}

/// Unstaging `path` as shown now.
pub fn unstage_in(dir: &Path, path: &str) -> Write {
    Write::UnstageFile {
        path: path.to_string(),
        shown: shown_in(dir, path).1,
    }
}

/// Every kind of `Write` the core takes, as serde lists them in its refusal of a kind it does not
/// know: the one list a test can read without repeating it, and longer with each new write.
pub fn kinds_of_write() -> Vec<String> {
    let Err(refusal) = serde_json::from_str::<Write>(r#"{"kind":"?"}"#) else {
        panic!("a kind the core does not know is refused");
    };
    let refusal = refusal.to_string();
    let (_, expected) = refusal
        .split_once("expected one of ")
        .unwrap_or_else(|| panic!("the refusal lists the kinds: {refusal}"));
    expected
        .split(", ")
        .map(|kind| kind.split('`').nth(1).unwrap().to_string())
        .collect()
}
