//! What a load holds in memory, counted by a global allocator of this test binary alone.
//!
//! ADR-0015 keeps the layout's arena and its SHA index for the whole load at a few allocations
//! per chunk, not several per commit; ADR-0031 and Q-3 keep the history the front ends read
//! compact. A history of 30 000 commits in chunks of 3 000 is laid out and taken in, and what
//! each holds afterwards is held to a budget.

use ocelli_core::graph::Layout;
use ocelli_core::history::History;
use ocelli_core::model::{CHUNK_COMMITS, CommitData, CommitKind};
use std::alloc::{GlobalAlloc, Layout as Shape, System};
use std::sync::atomic::{AtomicIsize, Ordering};

struct Counting;

static LIVE_BYTES: AtomicIsize = AtomicIsize::new(0);
static LIVE_BLOCKS: AtomicIsize = AtomicIsize::new(0);

// SAFETY: every call is passed on to the system allocator unchanged; the counters only add and
// subtract what passes through.
#[allow(unsafe_code)]
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, shape: Shape) -> *mut u8 {
        LIVE_BYTES.fetch_add(shape.size() as isize, Ordering::Relaxed);
        LIVE_BLOCKS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: the shape is the caller's, passed on unchanged.
        unsafe { System.alloc(shape) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, shape: Shape) {
        LIVE_BYTES.fetch_sub(shape.size() as isize, Ordering::Relaxed);
        LIVE_BLOCKS.fetch_sub(1, Ordering::Relaxed);
        // SAFETY: the pointer and shape are the caller's, passed on unchanged.
        unsafe { System.dealloc(ptr, shape) }
    }
}

#[global_allocator]
static COUNTING: Counting = Counting;

/// The live blocks and bytes now.
fn live() -> (isize, isize) {
    (
        LIVE_BLOCKS.load(Ordering::Relaxed),
        LIVE_BYTES.load(Ordering::Relaxed),
    )
}

const COMMITS: usize = 30_000;

/// A main line with a side branch of three merged back every twenty commits, newest first as
/// the walk yields them, with SHAs of forty hexadecimal digits spread as git's are.
fn history() -> Vec<CommitData> {
    let sha = |i: usize| {
        use sha2::{Digest, Sha256};
        Sha256::digest(i.to_le_bytes())[..20]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    };
    let commit = |i: usize, parents: Vec<String>| CommitData {
        sha: sha(i),
        message: format!("Change {i}\n\nWhy it changed."),
        author_name: "Ada".into(),
        author_email: "ada@example.invalid".into(),
        author_time: i as i64,
        committer_name: "Ada".into(),
        committer_email: "ada@example.invalid".into(),
        commit_time: i as i64,
        refs: vec![],
        parent_shas: parents,
        kind: CommitKind::Commit,
    };
    let mut commits = vec![commit(1, vec![])];
    let mut main = 1;
    let mut i = 2;
    while i <= COMMITS {
        if i % 20 == 0 && i + 4 <= COMMITS {
            let fork = main;
            for side in 0..3 {
                let parent = if side == 0 { fork } else { i - 1 };
                commits.push(commit(i, vec![sha(parent)]));
                i += 1;
            }
            commits.push(commit(i, vec![sha(main), sha(i - 1)]));
        } else {
            commits.push(commit(i, vec![sha(main)]));
        }
        main = i;
        i += 1;
    }
    commits.reverse();
    commits
}

#[test]
// Verifies: ADR-0015, ADR-0031, Q-3
fn a_load_holds_a_few_blocks_per_chunk_and_a_bounded_number_of_bytes_per_commit() {
    let chunks: Vec<Vec<CommitData>> = history().chunks(CHUNK_COMMITS).map(<[_]>::to_vec).collect();
    let n = chunks.len() as isize;

    // The layout alone: each chunk laid out and dropped, as if taken in elsewhere.
    let before = live();
    let mut layout = Layout::new();
    for chunk in chunks.clone() {
        drop(layout.lay_out(chunk));
    }
    let after = live();
    let (blocks, bytes) = (after.0 - before.0, after.1 - before.1);
    assert!(
        blocks <= 10 * n,
        "the layout holds {blocks} blocks for {n} chunks"
    );
    assert!(
        bytes <= 128 * COMMITS as isize,
        "the layout holds {bytes} bytes for {COMMITS} commits"
    );
    drop(layout);

    // The history alone, once the load's layout is gone; the input is made inside the count, so
    // that what it frees as it is taken in is not taken off what the history holds.
    let before = live();
    let mut layout = Layout::new();
    let mut history = History::new();
    for chunk in chunks.clone() {
        history.append(layout.lay_out(chunk));
    }
    drop(layout);
    let after = live();
    let (blocks, bytes) = (after.0 - before.0, after.1 - before.1);
    assert_eq!(history.rows(), COMMITS);
    assert!(
        blocks <= COMMITS as isize / 16,
        "the history holds {blocks} blocks for {COMMITS} commits"
    );
    // Its text, about 160 bytes a commit here, two SHAs and two people among it, and some 250
    // bytes of rows, spans, edges, and indices.
    assert!(
        bytes <= 512 * COMMITS as isize,
        "the history holds {bytes} bytes for {COMMITS} commits"
    );
}
