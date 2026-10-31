//! Verifies: ADR-0031
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

/// The core's answers as the desktop host sends them (ADR-0022): the transfers of refs that offer
/// a fetch and a push and refuse a pull, and the operations a commit offers beside a refusal, the
/// fixture `offers.json` the web interface's `writes.test.ts` reads against its types. It holds
/// the shape of the answers only; which answer the refs give is the core's own tests'.
#[test]
// Verifies: ADR-0022
fn the_offers_fixture_is_what_the_core_writes() {
    use ocelli_core::model::{LocalBranch, RepoRefs};
    let refs = RepoRefs {
        locals: vec![LocalBranch {
            name: "main".into(),
            sha: "h".into(),
            short_sha: "h".into(),
            is_head: true,
            upstream: None,
            ahead: 0,
            behind: 0,
            merged: true,
        }],
        default_remote: Some("origin".into()),
        ..RepoRefs::default()
    };
    let fixture = serde_json::json!({
        "transfers": refs.transfers(),
        "offered": [refs.offered("c"), RepoRefs::default().offered("c")],
    });
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/offers.json");
    let written = serde_json::to_string_pretty(&fixture).expect("the fixture serializes") + "\n";
    let on_disk = fs::read_to_string(&path).unwrap_or_default();
    if on_disk != written {
        fs::write(&path, &written).expect("the fixture is written");
        panic!(
            "{} did not hold what the core writes; it is written anew, commit it",
            path.display()
        );
    }
}
