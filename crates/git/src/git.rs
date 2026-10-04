use anyhow::Result;
use git2::{DiffOptions, ObjectType, Repository, Sort, Status, StatusOptions};
use ocelli_core::model::{
    CommitData, CommitKind, DiffLine, DiffLineKind, FileChange, FileStatus, MAX_COMMITS,
    WORKDIR_SHA,
};
use std::collections::HashMap;
use std::path::Path;

/// Loads commits from a git repository and returns them as CommitData.
/// Reports progress through `progress`, one message per step.
pub fn load_repository(
    repo_path: &Path,
    progress: &mut dyn FnMut(String),
) -> Result<Vec<CommitData>> {
    // 1. Open repo (walks up to find .git, supports bare repos)
    let mut repo = Repository::discover(repo_path)?;

    // 2. Collect all refs and map peeled SHA -> Vec<String> ref names
    let mut refs_by_sha: HashMap<String, Vec<String>> = HashMap::new();
    let mut start_oids: Vec<git2::Oid> = Vec::new();

    for reference in repo.references()? {
        let reference = reference?;
        let name = reference.name().unwrap_or("").to_string();

        // refs/stash is handled separately (see load_stashes) so the stash's
        // internal index/untracked parent commits never leak into the graph.
        if name == "refs/stash" {
            continue;
        }
        let pretty = pretty_ref_name(&name);

        // Peel to commit OID
        let target_oid = match reference.peel(ObjectType::Commit) {
            Ok(obj) => obj.id(),
            Err(_) => continue, // skip blobs/trees/non-commit tags
        };

        refs_by_sha
            .entry(target_oid.to_string())
            .or_default()
            .push(pretty);

        // Also try unpeeled for markStart
        if let Some(oid) = reference.target() {
            start_oids.push(oid);
        }
    }

    // 3. Set up revwalk (all commits, topological + time sort)
    let mut revwalk = repo.revwalk()?;
    revwalk.set_sorting(Sort::TOPOLOGICAL | Sort::TIME)?;

    for oid in &start_oids {
        let _ = revwalk.push(*oid); // ignore non-commit oids
    }

    // 4. Walk commits, build CommitData list
    let mut commits: Vec<CommitData> = Vec::with_capacity(MAX_COMMITS);
    let mut count = 0usize;

    for oid_result in revwalk {
        if count >= MAX_COMMITS {
            break;
        }
        let oid = oid_result?;
        let commit = repo.find_commit(oid)?;

        let sha = oid.to_string();
        let parent_shas = commit.parents().map(|p| p.id().to_string()).collect();

        let refs = refs_by_sha.get(&sha).cloned().unwrap_or_default();

        commits.push(CommitData {
            sha,
            message: commit.message().unwrap_or("").to_string(),
            author_name: commit.author().name().unwrap_or("").to_string(),
            author_email: commit.author().email().unwrap_or("").to_string(),
            author_time: commit.author().when().seconds(),
            committer_name: commit.committer().name().unwrap_or("").to_string(),
            committer_email: commit.committer().email().unwrap_or("").to_string(),
            commit_time: commit.time().seconds(),
            refs,
            parent_shas,
            kind: CommitKind::Commit,
        });

        count += 1;
        if count.is_multiple_of(500) {
            progress(format!("Loaded {} commits...", count));
        }
    }

    // 5. Append stash entries (each becomes a node hanging off its base commit)
    let walked = commits.len();
    load_stashes(&mut repo, &mut commits);

    // 6. Append a synthetic "uncommitted changes" node when the worktree is dirty.
    //    Its timestamp is "now", so the temporal sort places it at row 0.
    if let Some(wip) = working_dir_node(&repo) {
        commits.push(wip);
    }

    // 7. Bound the whole graph, synthetic nodes included.
    apply_commit_limit(&mut commits, walked);

    Ok(commits)
}

/// Enforces the commit limit over the *whole* graph, synthetic nodes included.
///
/// `walked` is how many entries at the front came from the revwalk. Room is made
/// by dropping the oldest walked commits rather than the stash and
/// working-directory nodes, which are the rows a reader cares about most.
fn apply_commit_limit(commits: &mut Vec<CommitData>, walked: usize) {
    if commits.len() <= MAX_COMMITS {
        return;
    }
    let overflow = commits.len() - MAX_COMMITS;
    let cut_from = walked.saturating_sub(overflow);
    commits.drain(cut_from..walked.min(commits.len()));
    // Backstop for the degenerate case of more synthetic nodes than the limit.
    commits.truncate(MAX_COMMITS);
}

/// Collects stash entries via the stash reflog and appends one node per stash.
/// Only the first parent (the base commit the stash was created on) is kept, so
/// the internal index/untracked parent commits stay out of the graph.
fn load_stashes(repo: &mut Repository, commits: &mut Vec<CommitData>) {
    // stash_foreach hands us (index, message, oid); we only collect here because
    // the &mut borrow forbids touching the repo inside the callback.
    let mut entries: Vec<(usize, String, git2::Oid)> = Vec::new();
    let _ = repo.stash_foreach(|index, message, oid| {
        entries.push((index, message.to_string(), *oid));
        true
    });

    for (index, message, oid) in entries {
        let Ok(commit) = repo.find_commit(oid) else {
            continue;
        };
        let parent_shas = commit
            .parent_id(0)
            .map(|p| vec![p.to_string()])
            .unwrap_or_default();

        commits.push(CommitData {
            sha: oid.to_string(),
            message: message.clone(),
            author_name: commit.author().name().unwrap_or("").to_string(),
            author_email: commit.author().email().unwrap_or("").to_string(),
            author_time: commit.author().when().seconds(),
            committer_name: commit.committer().name().unwrap_or("").to_string(),
            committer_email: commit.committer().email().unwrap_or("").to_string(),
            commit_time: commit.time().seconds(),
            refs: vec![format!("📦 stash@{{{}}}", index)],
            parent_shas,
            kind: CommitKind::Stash,
        });
    }
}

/// Builds the synthetic working-directory node, or `None` if the worktree is
/// clean / HEAD cannot be resolved (e.g. an unborn branch).
fn working_dir_node(repo: &Repository) -> Option<CommitData> {
    let head_commit = repo.head().ok()?.peel_to_commit().ok()?;
    if !has_uncommitted_changes(repo) {
        return None;
    }

    // Author the node with the repo's configured identity, when available.
    let (name, email) = match repo.signature() {
        Ok(sig) => (
            sig.name().unwrap_or("").to_string(),
            sig.email().unwrap_or("").to_string(),
        ),
        Err(_) => (String::new(), String::new()),
    };
    let now = chrono::Utc::now().timestamp();

    Some(CommitData {
        sha: WORKDIR_SHA.to_string(),
        message: "Uncommitted changes".to_string(),
        author_name: name,
        author_email: email,
        author_time: now,
        committer_name: String::new(),
        committer_email: String::new(),
        commit_time: now, // newest → sorts to row 0
        refs: Vec::new(),
        parent_shas: vec![head_commit.id().to_string()],
        kind: CommitKind::Working,
    })
}

/// True if the index or worktree has any tracked/untracked change (ignoring
/// ignored files), i.e. there is something to commit.
fn has_uncommitted_changes(repo: &Repository) -> bool {
    let mut opts = StatusOptions::new();
    opts.include_untracked(true)
        .include_ignored(false)
        .exclude_submodules(true);

    let Ok(statuses) = repo.statuses(Some(&mut opts)) else {
        return false;
    };
    let dirty = Status::INDEX_NEW
        | Status::INDEX_MODIFIED
        | Status::INDEX_DELETED
        | Status::INDEX_RENAMED
        | Status::INDEX_TYPECHANGE
        | Status::WT_NEW
        | Status::WT_MODIFIED
        | Status::WT_DELETED
        | Status::WT_TYPECHANGE
        | Status::WT_RENAMED;
    statuses.iter().any(|e| e.status().intersects(dirty))
}

/// Loads the changed files for a single commit SHA.
/// The synthetic [`WORKDIR_SHA`] yields the uncommitted working-directory diff.
pub fn load_commit_files(repo_path: &Path, sha: &str) -> Result<Vec<FileChange>> {
    let repo = Repository::discover(repo_path)?;

    let mut opts = DiffOptions::new();
    opts.ignore_whitespace(false);

    let diff = if sha == WORKDIR_SHA {
        opts.include_untracked(true).recurse_untracked_dirs(true);
        let head_tree = repo.head()?.peel_to_commit()?.tree()?;
        repo.diff_tree_to_workdir_with_index(Some(&head_tree), Some(&mut opts))?
    } else {
        let oid = repo.revparse_single(sha)?.id();
        let commit = repo.find_commit(oid)?;
        let commit_tree = commit.tree()?;
        let parent_tree = if commit.parent_count() > 0 {
            Some(commit.parent(0)?.tree()?)
        } else {
            None
        };
        repo.diff_tree_to_tree(parent_tree.as_ref(), Some(&commit_tree), Some(&mut opts))?
    };

    // Pass 1: collect file list from deltas
    let mut files: Vec<FileChange> = diff
        .deltas()
        .map(|delta| {
            let status = match delta.status() {
                git2::Delta::Added => FileStatus::Added,
                git2::Delta::Deleted => FileStatus::Deleted,
                git2::Delta::Modified => FileStatus::Modified,
                git2::Delta::Renamed => FileStatus::Renamed,
                git2::Delta::Copied => FileStatus::Copied,
                _ => FileStatus::Unknown,
            };
            let path = delta
                .new_file()
                .path()
                .or_else(|| delta.old_file().path())
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
            let old_path = if delta.status() == git2::Delta::Renamed {
                delta
                    .old_file()
                    .path()
                    .map(|p| p.to_string_lossy().into_owned())
            } else {
                None
            };
            FileChange {
                path,
                old_path,
                status,
                additions: 0,
                deletions: 0,
            }
        })
        .collect();

    // Pass 2: count additions/deletions per file via line callback
    {
        let files_ref = &mut files;
        diff.foreach(
            &mut |_, _| true,
            None,
            None,
            Some(&mut |delta, _hunk, line| {
                let key = delta
                    .new_file()
                    .path()
                    .or_else(|| delta.old_file().path())
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if let Some(f) = files_ref
                    .iter_mut()
                    .find(|f| f.path == key || f.old_path.as_deref() == Some(key.as_str()))
                {
                    match line.origin_value() {
                        git2::DiffLineType::Addition => f.additions += 1,
                        git2::DiffLineType::Deletion => f.deletions += 1,
                        _ => {}
                    }
                }
                true
            }),
        )?;
    }

    Ok(files)
}

/// Returns the unified diff of a single file in a commit as a Vec of DiffLine.
pub fn load_file_diff(repo_path: &Path, sha: &str, file_path: &str) -> Result<Vec<DiffLine>> {
    let repo = Repository::discover(repo_path)?;

    let mut opts = DiffOptions::new();
    opts.pathspec(file_path);
    opts.context_lines(3);

    let diff = if sha == WORKDIR_SHA {
        opts.include_untracked(true).recurse_untracked_dirs(true);
        let head_tree = repo.head()?.peel_to_commit()?.tree()?;
        repo.diff_tree_to_workdir_with_index(Some(&head_tree), Some(&mut opts))?
    } else {
        let oid = repo.revparse_single(sha)?.id();
        let commit = repo.find_commit(oid)?;
        let commit_tree = commit.tree()?;
        let parent_tree = if commit.parent_count() > 0 {
            Some(commit.parent(0)?.tree()?)
        } else {
            None
        };
        repo.diff_tree_to_tree(parent_tree.as_ref(), Some(&commit_tree), Some(&mut opts))?
    };

    // Collect hunk headers and line content separately to satisfy borrow checker.
    // Each element is (hunk_index_at_insertion, DiffLine).
    let mut hunk_lines: Vec<(usize, DiffLine)> = Vec::new(); // (insertion_idx, line)
    let mut body_lines: Vec<DiffLine> = Vec::new();

    {
        let hb = &mut hunk_lines;
        diff.foreach(
            &mut |_, _| true,
            None,
            Some(&mut |_, hunk| {
                let header = String::from_utf8_lossy(hunk.header()).into_owned();
                hb.push((
                    body_lines.len(),
                    DiffLine {
                        kind: DiffLineKind::Hunk,
                        content: header.trim_end().to_string(),
                        old_lineno: None,
                        new_lineno: None,
                    },
                ));
                true
            }),
            None,
        )?;
    }
    // Second pass: body lines only
    {
        let bl = &mut body_lines;
        diff.foreach(
            &mut |_, _| true,
            None,
            None,
            Some(&mut |_, _, line| {
                let content = String::from_utf8_lossy(line.content()).into_owned();
                let content = content
                    .trim_end_matches('\n')
                    .trim_end_matches('\r')
                    .to_string();
                let (kind, old_lineno, new_lineno) = match line.origin_value() {
                    git2::DiffLineType::Addition => (DiffLineKind::Add, None, line.new_lineno()),
                    git2::DiffLineType::Deletion => (DiffLineKind::Delete, line.old_lineno(), None),
                    git2::DiffLineType::Context => {
                        (DiffLineKind::Context, line.old_lineno(), line.new_lineno())
                    }
                    _ => (DiffLineKind::Meta, None, None),
                };
                bl.push(DiffLine {
                    kind,
                    content,
                    old_lineno,
                    new_lineno,
                });
                true
            }),
        )?;
    }

    // Merge: interleave hunk headers at the right positions
    let mut lines: Vec<DiffLine> = Vec::with_capacity(body_lines.len() + hunk_lines.len());
    let mut hi = 0usize; // hunk_lines index
    for (i, bl) in body_lines.into_iter().enumerate() {
        while hi < hunk_lines.len() && hunk_lines[hi].0 == i {
            lines.push(hunk_lines[hi].1.clone());
            hi += 1;
        }
        lines.push(bl);
    }
    // trailing hunk headers (edge case: empty hunk at end)
    while hi < hunk_lines.len() {
        lines.push(hunk_lines[hi].1.clone());
        hi += 1;
    }
    Ok(lines)
}

/// Formats a git ref name (removes refs/ prefix, adds emoji for tags)
fn pretty_ref_name(name: &str) -> String {
    if let Some(s) = name.strip_prefix("refs/heads/") {
        return s.to_string();
    }
    if let Some(s) = name.strip_prefix("refs/remotes/") {
        return s.to_string();
    }
    if let Some(s) = name.strip_prefix("refs/tags/") {
        return format!("🏷 {}", s);
    }
    name.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(sha: &str, kind: CommitKind) -> CommitData {
        CommitData {
            sha: sha.to_string(),
            message: String::new(),
            author_name: String::new(),
            author_email: String::new(),
            author_time: 0,
            committer_name: String::new(),
            committer_email: String::new(),
            commit_time: 0,
            refs: vec![],
            parent_shas: vec![],
            kind,
        }
    }

    /// `pretty_ref_name` strips the ref namespace and marks tags.
    #[test]
    fn ref_names_are_shortened_per_namespace() {
        assert_eq!(pretty_ref_name("refs/heads/main"), "main");
        assert_eq!(pretty_ref_name("refs/remotes/origin/main"), "origin/main");
        assert_eq!(pretty_ref_name("refs/tags/v1.0.0"), "🏷 v1.0.0");
        assert_eq!(pretty_ref_name("refs/notes/commits"), "refs/notes/commits");
    }

    /// Under the limit nothing is touched.
    #[test]
    // Verifies: ADR-0016
    fn commit_limit_leaves_a_small_graph_alone() {
        let mut commits: Vec<CommitData> = (0..10)
            .map(|i| entry(&format!("c{i}"), CommitKind::Commit))
            .collect();
        let walked = commits.len();
        apply_commit_limit(&mut commits, walked);
        assert_eq!(commits.len(), 10);
    }

    /// The limit bounds the *whole* graph, and the synthetic nodes survive it:
    /// room is made by dropping the oldest walked commits instead.
    /// This is the regression guard for synthetic nodes being appended past the cap.
    #[test]
    // Verifies: ADR-0016, ADR-0024
    fn commit_limit_bounds_the_whole_graph_and_keeps_synthetic_nodes() {
        let mut commits: Vec<CommitData> = (0..MAX_COMMITS)
            .map(|i| entry(&format!("c{i}"), CommitKind::Commit))
            .collect();
        let walked = commits.len();
        commits.push(entry("stash0", CommitKind::Stash));
        commits.push(entry("WORKING_DIRECTORY", CommitKind::Working));

        apply_commit_limit(&mut commits, walked);

        assert_eq!(
            commits.len(),
            MAX_COMMITS,
            "the cap covers synthetic nodes too"
        );
        assert!(
            commits.iter().any(|c| c.kind == CommitKind::Working),
            "the working-directory node must not be the one dropped",
        );
        assert!(commits.iter().any(|c| c.kind == CommitKind::Stash));
        assert_eq!(commits[0].sha, "c0", "the newest walked commits are kept");
        assert!(
            !commits
                .iter()
                .any(|c| c.sha == format!("c{}", MAX_COMMITS - 1)),
            "the oldest walked commits are the ones dropped",
        );
    }

    /// Degenerate case: more synthetic nodes than the limit allows.
    #[test]
    // Verifies: ADR-0016
    fn commit_limit_holds_even_with_absurdly_many_synthetic_nodes() {
        let mut commits: Vec<CommitData> = vec![entry("c0", CommitKind::Commit)];
        let walked = commits.len();
        for i in 0..(MAX_COMMITS + 5) {
            commits.push(entry(&format!("stash{i}"), CommitKind::Stash));
        }

        apply_commit_limit(&mut commits, walked);

        assert_eq!(commits.len(), MAX_COMMITS);
    }
}
