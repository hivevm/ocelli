//! Verifies: ADR-0039, Q-5
//!
//! Confined to a folder, as the remote server confines it, the adapter opens a repository only
//! at the folder it is given: a folder without its own `.git`, or one whose `.git` vanished
//! after the server checked it, opens nothing instead of the repository around it. The test runs
//! in a binary of its own, since the confinement holds for the whole process.

#![allow(
    clippy::disallowed_methods,
    reason = "a test sets a repository up by hand, the way a developer would"
)]

mod support;
use ocelli_core::port::RepositoryReader;
use ocelli_git::GitReader;
use support::{Scratch, git};

#[test]
fn confined_no_read_searches_upwards_out_of_the_folder() {
    let scratch = Scratch::new("confined");
    let enclosing = scratch.0.join("encl");
    std::fs::create_dir_all(&enclosing).unwrap();
    git(&enclosing, &["init", "--quiet", "-b", "main"]);
    std::fs::write(enclosing.join("f"), "one\n").unwrap();
    git(&enclosing, &["add", "f"]);
    git(&enclosing, &["commit", "--quiet", "-m", "one"]);
    let root = enclosing.join("root");
    let vanished = root.join("vanished");
    std::fs::create_dir_all(&vanished).unwrap();
    let root = root.canonicalize().unwrap();

    // Not confined, a read searches upwards and finds the enclosing repository.
    assert!(GitReader.load_refs(&vanished).is_ok());
    assert!(GitReader.load_main_tip(&vanished).unwrap().is_some());

    ocelli_git::confine::confine_to(&root);
    assert!(GitReader.load_refs(&vanished).is_err());
    assert!(GitReader.load_main_tip(&vanished).is_err());
    assert!(
        GitReader
            .load_chunks(&vanished, &mut |_| {}, &mut |_| true)
            .is_err()
    );
}
