//! Verifies: ADR-0008, ADR-0015, ADR-0030, G-2, G-9, G-10, G-15
//!
//! The layout is carried over unchanged: for 500 generated histories, with merges, octopus
//! merges, several roots, equal commit times, parents outside the loaded set, and stash and
//! working-directory nodes, the serialized graph hashes to the value the reference
//! implementation produced. A difference in a single row, lane, edge, or field fails.

mod history;

use ocelli_core::graph::build_graph;
use ocelli_core::model::{CommitData, CommitKind};
use sha2::{Digest, Sha256};

fn commits(seed: u64) -> Vec<CommitData> {
    history::history(seed)
        .into_iter()
        .map(|s| CommitData {
            sha: s.sha,
            message: s.message,
            author_name: s.author_name,
            author_email: s.author_email,
            author_time: s.author_time,
            committer_name: s.committer_name,
            committer_email: s.committer_email,
            commit_time: s.commit_time,
            refs: s.refs,
            parent_shas: s.parent_shas,
            kind: match s.kind {
                0 => CommitKind::Commit,
                1 => CommitKind::Stash,
                _ => CommitKind::Working,
            },
            stash: None,
        })
        .collect()
}

#[test]
fn every_generated_history_lays_out_as_the_reference_did() {
    let expected = include_str!("fixtures/layout.sha256");
    let mut mismatches = Vec::new();
    for line in expected.lines() {
        let (seed, hash) = line.split_once(' ').expect("a line is `<seed> <sha256>`");
        let seed: u64 = seed.parse().expect("the seed is a number");
        let json =
            serde_json::to_string(&build_graph(commits(seed))).expect("the graph serializes");
        let actual: String = Sha256::digest(json.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        if actual != hash {
            mismatches.push(seed);
        }
    }
    assert_eq!(expected.lines().count(), 500);
    assert!(
        mismatches.is_empty(),
        "layouts differ for {} seeds, the first {:?}",
        mismatches.len(),
        &mismatches[..mismatches.len().min(10)]
    );
}
