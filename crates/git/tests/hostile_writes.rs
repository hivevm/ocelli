//! Verifies: ADR-0031, ADR-0037, ADR-0035
//!
//! A write's strings come from the caller, a browser of the remote server among them: names,
//! ids, messages, paths, and a remote's name. git2 turns some into C strings by unwrapping, and a
//! release build aborts on a panic, so a NUL in any of them would take the whole server down.
//! Every kind of write, each made from a repository in the state it is written in, is written
//! with each of its string fields set in turn to each hostile value, each time in a fresh copy of
//! that repository, and none may panic.

#![allow(
    clippy::disallowed_methods,
    reason = "a test sets a repository up by hand, the way a developer would"
)]

mod reachable;
mod shown;
mod support;
use ocelli_core::port::{RepositoryWriter, Write};
use ocelli_git::GitWriter;
use reachable::Repositories;
use serde_json::Value;
use std::path::Path;
use support::{Scratch, git, hostile, shortened, write_file};

/// Every string of `value` but a write's kind and the names of an enum's variants, which a
/// different string would only make unreadable, each as the path of keys that leads to it.
fn string_fields(value: &Value, at: &mut Vec<String>, found: &mut Vec<Vec<String>>) {
    match value {
        Value::Object(map) => {
            for (key, field) in map {
                if matches!(key.as_str(), "kind" | "mode" | "take" | "operation") {
                    continue;
                }
                at.push(key.clone());
                string_fields(field, at, found);
                at.pop();
            }
        }
        Value::String(_) => found.push(at.clone()),
        _ => {}
    }
}

fn set(value: &mut Value, path: &[String], to: &str) {
    match path.split_first() {
        Some((key, rest)) => set(&mut value[key.as_str()], rest, to),
        None => *value = Value::String(to.to_string()),
    }
}

/// A repository with a little of everything a write reaches: branches, a tag, a stash, a change.
fn repository(dir: &Path) {
    git(dir, &["init", "--quiet", "-b", "main"]);
    git(dir, &["config", "user.name", "Ada"]);
    git(dir, &["config", "user.email", "ada@example.invalid"]);
    write_file(dir, "f", "1\n");
    git(dir, &["add", "f"]);
    git(dir, &["commit", "--quiet", "-m", "one"]);
    git(dir, &["branch", "topic"]);
    git(dir, &["tag", "v1"]);
    write_file(dir, "f", "stashed\n");
    git(dir, &["stash", "push", "--quiet"]);
    write_file(dir, "f", "changed\n");
}

#[test]
fn no_string_a_caller_gives_in_any_write_makes_the_writer_panic() {
    let scratch = Scratch::new("hostile");
    let repositories = Repositories::new(&scratch.0);
    let dir = scratch.0.join("written");
    let values = hostile();
    let mut panicked = Vec::new();
    for (state, make) in reachable::examples(&repositories) {
        let example = serde_json::to_value(make(&repositories.of(state))).unwrap();
        let kind = &example["kind"];
        // As made, the write is written: a hostile value in its place reaches as far.
        repositories.fresh(state, &dir);
        let write: Write = serde_json::from_value(example.clone()).unwrap();
        if let Err(e) = GitWriter.write(&dir, &write) {
            panic!("{kind} as made is refused: {e:#}");
        }
        let mut fields = Vec::new();
        string_fields(&example, &mut Vec::new(), &mut fields);
        // Every kind of write takes a string: its fields are set, not none of them.
        assert!(!fields.is_empty(), "{example}");
        for field in &fields {
            for value in &values {
                let mut sent = example.clone();
                set(&mut sent, field, value);
                let write: Write = serde_json::from_value(sent).unwrap();
                repositories.fresh(state, &dir);
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    GitWriter.write(&dir, &write)
                }));
                if result.is_err() {
                    let failure = format!("{kind} {field:?} = {}", shortened(value));
                    eprintln!("panicked: {failure}");
                    panicked.push(failure);
                }
            }
        }
    }
    assert!(panicked.is_empty(), "the writer panicked on: {panicked:#?}");
}

#[test]
fn a_nul_in_any_string_of_a_write_is_refused_before_the_repository_is_reached() {
    let scratch = Scratch::new("hostile-nul");
    let dir = scratch.0.join("repo");
    std::fs::create_dir_all(&dir).unwrap();
    repository(&dir);
    let before = support::snapshot(&dir);
    for write in [
        Write::CreateTag {
            name: "a\0b".into(),
            commit: git(&dir, &["rev-parse", "HEAD"]),
            message: None,
        },
        Write::CreateBranch {
            name: "a\0b".into(),
            commit: git(&dir, &["rev-parse", "HEAD"]),
            switch: false,
        },
        Write::CreateStash {
            message: Some("wip\0".into()),
            untracked: false,
        },
    ] {
        support::refused(&dir, &write, "holds a NUL character");
    }
    assert_eq!(support::snapshot(&dir), before);
}
