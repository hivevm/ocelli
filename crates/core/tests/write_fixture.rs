//! Verifies: ADR-0033
//!
//! The web interface hands the core a `Write` as JSON from a union it keeps by hand in
//! `src-web/src/writes.ts`, and nothing but this pair of tests compares the two. This test writes
//! one example of every write, as the core serializes it, to `fixtures/writes.json`; the web
//! interface's `writes.test.ts` reads the same file against its union. A write added, or a field
//! named differently, on either side fails one of the two.
//!
//! The fixture is the core's output: where the file on disk differs from it, this test writes it
//! anew and fails, so that the change to it is committed with the change to `Write`.

use ocelli_core::port::{Conflict, Operation, ResetMode, Take, Write};
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

/// One example of every write, in the order the enum declares them. Every optional field holds a
/// value, so that its name reaches the fixture as the core spells it.
fn examples() -> Vec<Write> {
    let s = |text: &str| text.to_string();
    vec![
        Write::CreateTag {
            name: s("v1"),
            commit: s("c"),
            message: Some(s("The first release")),
        },
        Write::DeleteTag {
            name: s("v1"),
            commit: s("c"),
            annotated: true,
        },
        Write::CreateStash {
            message: Some(s("wip")),
            untracked: true,
        },
        Write::ApplyStash {
            index: 1,
            commit: s("s"),
            drop: true,
        },
        Write::DropStash {
            index: 1,
            commit: s("s"),
        },
        Write::StageFile {
            path: s("f"),
            shown: Some(s("b")),
        },
        Write::UnstageFile {
            path: s("f"),
            shown: Some(s("b")),
        },
        Write::Commit {
            message: s("Change f"),
            head: Some(s("h")),
        },
        Write::CreateBranch {
            name: s("topic"),
            commit: s("c"),
            switch: true,
        },
        Write::SwitchBranch {
            name: s("topic"),
            tip: s("t"),
        },
        Write::Detach { commit: s("c") },
        Write::TrackRemote {
            remote: s("origin"),
            branch: s("topic"),
            tip: s("t"),
        },
        Write::DeleteBranch {
            name: s("topic"),
            tip: s("t"),
            merged: false,
        },
        Write::Fetch {
            remote: s("origin"),
        },
        Write::Pull {
            branch: s("main"),
            tip: s("t"),
        },
        Write::Push {
            branch: s("main"),
            tip: s("t"),
        },
        Write::Merge {
            branch: s("main"),
            tip: s("t"),
            commit: s("c"),
            name: Some(s("topic")),
        },
        Write::Rebase {
            branch: s("topic"),
            tip: s("t"),
            onto: s("c"),
        },
        Write::CherryPick {
            branch: s("main"),
            tip: s("t"),
            commit: s("c"),
        },
        Write::Revert {
            branch: s("main"),
            tip: s("t"),
            commit: s("c"),
        },
        Write::Reset {
            branch: s("main"),
            tip: s("t"),
            commit: s("c"),
            mode: ResetMode::Hard,
        },
        Write::Resolve {
            path: s("f"),
            conflict: Conflict {
                path: s("f"),
                ancestor: Some(s("a")),
                ours: Some(s("o")),
                theirs: Some(s("t")),
            },
            take: Take::AsEdited,
        },
        Write::Continue {
            operation: Operation::CherryPick,
            head: s("h"),
        },
        Write::Abort {
            operation: Operation::Merge,
            head: s("h"),
        },
    ]
}

/// The kinds the core takes, as serde names them in its refusal of a kind it does not know: the
/// one list of them a test can read without repeating it.
fn kinds_the_core_takes() -> Vec<String> {
    let refusal = serde_json::from_str::<Write>(r#"{"kind":"?"}"#)
        .expect_err("a kind the core does not know is refused")
        .to_string();
    let (_, expected) = refusal
        .split_once("expected one of ")
        .unwrap_or_else(|| panic!("the refusal lists the kinds: {refusal}"));
    expected
        .split(", ")
        .map(|kind| {
            kind.split('`')
                .nth(1)
                .expect("each kind is quoted")
                .to_string()
        })
        .collect()
}

fn kind_of(write: &serde_json::Value) -> String {
    write["kind"]
        .as_str()
        .expect("a write has a kind")
        .to_string()
}

#[test]
fn every_kind_the_core_takes_has_one_example() {
    let kinds = kinds_the_core_takes();
    assert!(kinds.len() > 1, "the refusal lists more than one kind");
    let examples: Vec<String> = examples()
        .iter()
        .map(|write| kind_of(&serde_json::to_value(write).expect("a write serializes")))
        .collect();
    assert_eq!(examples, kinds);
    assert_eq!(
        examples.iter().collect::<BTreeSet<_>>().len(),
        examples.len(),
        "no kind has two examples"
    );
}

#[test]
fn every_example_reads_back_as_itself() {
    for write in examples() {
        let json = serde_json::to_string(&write).expect("a write serializes");
        let back: Write = serde_json::from_str(&json).expect("the core reads what it writes");
        assert_eq!(back, write);
    }
}

#[test]
fn the_fixture_is_what_the_core_writes() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/writes.json");
    let written = serde_json::to_string_pretty(&examples()).expect("the examples serialize") + "\n";
    let on_disk = fs::read_to_string(&path).unwrap_or_default();
    if on_disk != written {
        fs::write(&path, &written).expect("the fixture is written");
        panic!(
            "{} did not hold what the core writes; it is written anew, commit it",
            path.display()
        );
    }
}

/// A file of the working-directory node as the core serializes it, a staging with both parts,
/// beside one of a commit, without one: the fixture `files.json` the web interface's
/// `types.test.ts` reads against its `FileChange`.
#[test]
fn the_file_fixture_is_what_the_core_writes() {
    use ocelli_core::model::{FileChange, FileStatus, Shown, Staged, Staging};
    let file = |staging| FileChange {
        path: "f".into(),
        old_path: Some("e".into()),
        status: FileStatus::Renamed,
        additions: 1,
        deletions: 2,
        staging,
    };
    let files = vec![
        file(Some(Staging {
            staged: Staged::Partly,
            shown: Shown {
                workdir: Some("w".into()),
                index: Some("i".into()),
            },
        })),
        file(None),
    ];
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/files.json");
    let written = serde_json::to_string_pretty(&files).expect("the files serialize") + "\n";
    let on_disk = fs::read_to_string(&path).unwrap_or_default();
    if on_disk != written {
        fs::write(&path, &written).expect("the fixture is written");
        panic!(
            "{} did not hold what the core writes; it is written anew, commit it",
            path.display()
        );
    }
}

/// The core's answers for the transfers of the checked-out branch over a few sets of refs, and
/// for every write whether it can be stopped: the fixture `transfers.json` the web interface's
/// `transfers.test.ts` holds its helper to.
#[test]
fn the_transfer_fixture_is_what_the_core_answers() {
    use ocelli_core::model::{LocalBranch, RepoRefs};
    let refs = |upstream: Option<Option<&str>>, default_remote: Option<&str>| RepoRefs {
        locals: upstream
            .map(|upstream| LocalBranch {
                name: "main".into(),
                sha: "s".into(),
                short_sha: "s".into(),
                is_head: true,
                upstream: upstream.map(str::to_string),
                ahead: 0,
                behind: 0,
                merged: true,
            })
            .into_iter()
            .collect(),
        default_remote: default_remote.map(str::to_string),
        ..RepoRefs::default()
    };
    let answer = |answer: Result<Write, String>| match answer {
        Ok(write) => serde_json::json!({ "write": write }),
        Err(reason) => serde_json::json!({ "reason": reason }),
    };
    // The checked-out branch after another, which the answers must find all the same.
    let mut second = refs(Some(Some("origin/main")), None);
    second.locals.insert(
        0,
        LocalBranch {
            name: "other".into(),
            sha: "o".into(),
            short_sha: "o".into(),
            is_head: false,
            upstream: None,
            ahead: 0,
            behind: 0,
            merged: true,
        },
    );
    let cases: Vec<_> = [
        refs(Some(Some("origin/main")), Some("origin")),
        refs(Some(Some("origin/main")), None),
        refs(Some(None), Some("origin")),
        refs(Some(None), None),
        refs(None, Some("origin")),
        second,
    ]
    .into_iter()
    .map(|refs| {
        serde_json::json!({
            "refs": refs,
            "fetch": answer(refs.fetch()),
            "pull": answer(refs.pull()),
            "push": answer(refs.push()),
        })
    })
    .collect();
    let can_stop: serde_json::Map<_, _> = examples()
        .iter()
        .map(|w| {
            let json = serde_json::to_value(w).expect("the example serializes");
            (kind_of(&json), serde_json::Value::Bool(w.can_stop()))
        })
        .collect();
    let fixture = serde_json::json!({ "cases": cases, "canStop": can_stop });
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/transfers.json");
    let written = serde_json::to_string_pretty(&fixture).expect("the fixture serializes") + "\n";
    let on_disk = fs::read_to_string(&path).unwrap_or_default();
    if on_disk != written {
        fs::write(&path, &written).expect("the fixture is written");
        panic!(
            "{} did not hold what the core answers; it is written anew, commit it",
            path.display()
        );
    }
}

/// The core's history actions for a commit on another branch, one on none, and the checked-out
/// one, and its refusal where no branch is checked out: the fixture `actions.json` the web
/// interface's `actions.test.ts` holds its helper to.
#[test]
fn the_actions_fixture_is_what_the_core_answers() {
    use ocelli_core::model::{LocalBranch, RepoRefs};
    let local = |name: &str, sha: &str, is_head: bool| LocalBranch {
        name: name.into(),
        sha: sha.into(),
        short_sha: sha.into(),
        is_head,
        upstream: None,
        ahead: 0,
        behind: 0,
        merged: true,
    };
    let refs = RepoRefs {
        locals: vec![local("topic", "t", false), local("main", "h", true)],
        ..RepoRefs::default()
    };
    let answer = |refs: &RepoRefs, sha: &str| match refs.history_actions(sha) {
        Ok(actions) => serde_json::json!({ "actions": actions }),
        Err(reason) => serde_json::json!({ "reason": reason }),
    };
    let detached = RepoRefs::default();
    let cases: Vec<_> = [(&refs, "t"), (&refs, "x"), (&refs, "h"), (&detached, "t")]
        .into_iter()
        .map(|(refs, sha)| {
            serde_json::json!({ "refs": refs, "sha": sha, "answer": answer(refs, sha) })
        })
        .collect();
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/actions.json");
    let written = serde_json::to_string_pretty(&cases).expect("the fixture serializes") + "\n";
    let on_disk = fs::read_to_string(&path).unwrap_or_default();
    if on_disk != written {
        fs::write(&path, &written).expect("the fixture is written");
        panic!(
            "{} did not hold what the core answers; it is written anew, commit it",
            path.display()
        );
    }
}
