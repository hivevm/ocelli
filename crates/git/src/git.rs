use anyhow::Result;
use git2::{DiffOptions, Repository, Sort, Status, StatusOptions};
use ocelli_core::model::{
    CHUNK_COMMITS, CommitData, CommitKind, DiffLine, DiffLineKind, FileChange, FileStatus, Shown,
    Staged, Staging, WORKDIR_SHA,
};
use std::collections::HashMap;
use std::path::Path;

/// Loads the first chunk of a repository's history: the newest commits of the walk with the
/// stash and working-directory nodes, at most [`CHUNK_COMMITS`] together (ADR-0016).
/// Reports progress through `progress`, one message per step.
pub fn load_first_chunk(
    repo_path: &Path,
    progress: &mut dyn FnMut(String),
) -> Result<Vec<CommitData>> {
    let mut first = Vec::new();
    load_chunks(repo_path, progress, &mut |chunk| {
        first = chunk;
        false
    })?;
    Ok(first)
}

/// Loads a repository's whole history in chunks of [`CHUNK_COMMITS`] in the commit-date order of
/// the walk over every ref, handing each to `on_chunk` as it is read; `on_chunk`
/// returns whether to go on (ADR-0016). The first chunk carries the stash and working-directory
/// nodes and makes room for them by moving its oldest walked commits on to the second chunk.
/// Reports progress through `progress`, one message per step.
pub fn load_chunks(
    repo_path: &Path,
    progress: &mut dyn FnMut(String),
    on_chunk: &mut dyn FnMut(Vec<CommitData>) -> bool,
) -> Result<()> {
    // The walk parses every commit of the history once and never again; libgit2's object cache
    // would keep them all, and the memory it frees afterwards stays with the process (Q-3). The
    // setting holds for the whole process: the per-commit reads after it read a commit at a time
    // and gain little from the cache.
    git2::opts::enable_caching(false);
    // 1. Open repo (walks up to find .git, supports bare repos)
    let repo = crate::confine::open(repo_path)?;

    // 2. Collect all refs and map peeled SHA -> Vec<String> ref names
    let mut refs_by_sha: HashMap<String, Vec<String>> = HashMap::new();
    let mut starts: Vec<(i64, git2::Oid)> = Vec::new();

    for reference in repo.references()? {
        let reference = reference?;
        let name = reference.name().unwrap_or("").to_string();

        // refs/stash is handled separately (see load_stashes) so the stash's
        // internal index/untracked parent commits never leak into the graph.
        if name == "refs/stash" {
            continue;
        }
        let pretty = pretty_ref_name(&name);

        // Peel to the commit, whose date orders the start of the walk
        let target = match reference.peel_to_commit() {
            Ok(commit) => commit,
            Err(_) => continue, // skip blobs/trees/non-commit tags
        };
        let target_oid = target.id();

        refs_by_sha
            .entry(target_oid.to_string())
            .or_default()
            .push(pretty);

        starts.push((target.time().seconds(), target_oid));
    }

    // 3. Set up the walk in commit-date order (ADR-0016). Any sorting libgit2 offers reads the
    //    whole history before the first commit; unsorted, it keeps the commits it has reached
    //    ordered by date and yields each after one of its children. It takes the starts in the
    //    reverse of the order they are pushed, so the oldest is pushed first and the newest is
    //    where the walk begins.
    let mut revwalk = repo.revwalk()?;
    revwalk.set_sorting(Sort::NONE)?;
    starts.sort_unstable();
    starts.dedup_by_key(|(_, oid)| *oid);
    for (_, oid) in &starts {
        revwalk.push(*oid)?;
    }

    // 4. Walk commits, build CommitData chunks
    let mut commits: Vec<CommitData> = Vec::with_capacity(CHUNK_COMMITS);
    let mut count = 0usize;
    let mut first = true;

    for oid_result in revwalk {
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
            stash: None,
        });

        count += 1;
        if count.is_multiple_of(500) {
            progress(format!("Loaded {} commits...", count));
        }

        if commits.len() == CHUNK_COMMITS {
            let next = if first {
                first = false;
                with_synthetic_nodes(repo_path, &mut commits)?
            } else {
                Vec::with_capacity(CHUNK_COMMITS)
            };
            if !on_chunk(std::mem::replace(&mut commits, next)) {
                return Ok(());
            }
        }
    }

    // The walk has ended: hand over what is left.
    if first {
        let rest = with_synthetic_nodes(repo_path, &mut commits)?;
        let _ = on_chunk(commits) && !rest.is_empty() && on_chunk(rest);
    } else if !commits.is_empty() {
        on_chunk(commits);
    }
    Ok(())
}

/// Completes the first chunk with the stash and working-directory nodes (ADR-0024) and returns
/// the walked commits it moves on to the second chunk to stay within [`CHUNK_COMMITS`]. It opens
/// the repository a second time: the walk holds the first handle, and reading the stash needs one
/// of its own.
fn with_synthetic_nodes(
    repo_path: &Path,
    commits: &mut Vec<CommitData>,
) -> Result<Vec<CommitData>> {
    let mut repo = crate::confine::open(repo_path)?;
    // Stash entries, each a node hanging off its base commit.
    let walked = commits.len();
    load_stashes(&mut repo, commits);

    // A synthetic "uncommitted changes" node when the worktree is dirty.
    // Its timestamp is "now", so the temporal sort places it at row 0.
    if let Some(wip) = working_dir_node(&repo) {
        commits.push(wip);
    }

    Ok(fit_first_chunk(commits, walked))
}

/// Fits the first chunk, synthetic nodes included, within [`CHUNK_COMMITS`].
///
/// `walked` is how many entries at the front came from the revwalk. Room is made by moving the
/// oldest walked commits on to the next chunk rather than the stash and working-directory nodes,
/// which are the rows a reader cares about most; the moved commits are returned in walk order.
fn fit_first_chunk(commits: &mut Vec<CommitData>, walked: usize) -> Vec<CommitData> {
    if commits.len() <= CHUNK_COMMITS {
        return Vec::new();
    }
    let overflow = commits.len() - CHUNK_COMMITS;
    let cut_from = walked.saturating_sub(overflow);
    let moved: Vec<CommitData> = commits.drain(cut_from..walked.min(commits.len())).collect();
    // Backstop for the degenerate case of more synthetic nodes than a chunk holds.
    commits.truncate(CHUNK_COMMITS);
    moved
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
            stash: Some(index),
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

    // Author the node with the repo's configured identity, when available. Confined to a server
    // root, it is not read: the configuration libgit2 reads, the developer's global one and the
    // files a repository's own one includes, lies outside what ADR-0039 confines, and the
    // identity is the one value of it the node would show.
    let signature = if crate::confine::confined() {
        Err(git2::Error::from_str("not read under confinement"))
    } else {
        repo.signature()
    };
    let (name, email) = match signature {
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
        stash: None,
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
    statuses
        .iter()
        .any(|e| e.status().intersects(INDEX | WORKTREE))
}

/// The changes the index holds against the checked-out commit.
pub(crate) const INDEX: Status = Status::INDEX_NEW
    .union(Status::INDEX_MODIFIED)
    .union(Status::INDEX_DELETED)
    .union(Status::INDEX_RENAMED)
    .union(Status::INDEX_TYPECHANGE);

/// The changes the working directory holds against the index, an untracked file included.
pub(crate) const WORKTREE: Status = Status::WT_NEW
    .union(Status::WT_MODIFIED)
    .union(Status::WT_DELETED)
    .union(Status::WT_RENAMED)
    .union(Status::WT_TYPECHANGE);

/// Loads the changed files for a single commit SHA.
/// The synthetic [`WORKDIR_SHA`] yields the uncommitted working-directory diff.
pub fn load_commit_files(repo_path: &Path, sha: &str) -> Result<Vec<FileChange>> {
    let repo = crate::confine::open(repo_path)?;

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
                staging: None,
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

    if sha == WORKDIR_SHA {
        for f in &mut files {
            f.staging = Some(Staging {
                staged: staged(&repo, &f.path),
                shown: shown(&repo, &f.path),
            });
        }
    }
    Ok(files)
}

/// The content of `path` in the working directory and in the index, by blob id.
pub(crate) fn shown(repo: &Repository, path: &str) -> Shown {
    let workdir = repo
        .workdir()
        .and_then(|root| working_blob(root, path))
        .map(|oid| oid.to_string());
    let index = repo
        .index()
        .ok()
        .and_then(|index| index.get_path(Path::new(path), 0))
        .map(|entry| entry.id.to_string());
    Shown { workdir, index }
}

/// The blob id `git` gives `path` in the working directory `root`, as `git` reads it: a link as
/// its target's text, never the file it names, and nothing for a path that passes through a
/// linked folder, which `git` sees as the link it is and Ocelli never reads through (ADR-0039);
/// nothing for a path that is not there.
fn working_blob(root: &Path, path: &str) -> Option<git2::Oid> {
    let relative = Path::new(path);
    let mut at = root.to_path_buf();
    let mut parts = relative.components().peekable();
    while let Some(part) = parts.next() {
        if !matches!(part, std::path::Component::Normal(_)) {
            return None;
        }
        at.push(part);
        let meta = at.symlink_metadata().ok()?;
        if parts.peek().is_some() {
            if !meta.is_dir() {
                return None;
            }
        } else if meta.file_type().is_symlink() {
            let target = std::fs::read_link(&at).ok()?;
            return git2::Oid::hash_object(
                git2::ObjectType::Blob,
                target.to_string_lossy().as_bytes(),
            )
            .ok();
        } else if !meta.is_file() {
            return None;
        }
    }
    git2::Oid::hash_file(git2::ObjectType::Blob, &at).ok()
}

/// How much of `path`'s uncommitted change the index holds.
fn staged(repo: &Repository, path: &str) -> Staged {
    let status = repo.status_file(Path::new(path)).unwrap_or(Status::empty());
    match (status.intersects(INDEX), status.intersects(WORKTREE)) {
        (true, true) => Staged::Partly,
        (true, false) => Staged::Yes,
        _ => Staged::No,
    }
}

/// Returns the unified diff of a single file in a commit as a Vec of DiffLine.
pub fn load_file_diff(repo_path: &Path, sha: &str, file_path: &str) -> Result<Vec<DiffLine>> {
    let repo = crate::confine::open(repo_path)?;

    let mut opts = DiffOptions::new();
    // The path as written, never a pattern that takes other files with it.
    #[expect(
        clippy::disallowed_methods,
        reason = "the pattern matching is turned off"
    )]
    opts.pathspec(file_path).disable_pathspec_match(true);
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
            stash: None,
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
    fn the_first_chunk_leaves_a_small_graph_alone() {
        let mut commits: Vec<CommitData> = (0..10)
            .map(|i| entry(&format!("c{i}"), CommitKind::Commit))
            .collect();
        let walked = commits.len();
        assert!(fit_first_chunk(&mut commits, walked).is_empty());
        assert_eq!(commits.len(), 10);
    }

    /// The first chunk bounds the *whole* graph, and the synthetic nodes survive it:
    /// room is made by moving the oldest walked commits on to the next chunk instead.
    /// This is the regression guard for synthetic nodes being appended past the chunk.
    #[test]
    // Verifies: ADR-0016, ADR-0024
    fn the_first_chunk_bounds_the_whole_graph_and_keeps_synthetic_nodes() {
        let mut commits: Vec<CommitData> = (0..CHUNK_COMMITS)
            .map(|i| entry(&format!("c{i}"), CommitKind::Commit))
            .collect();
        let walked = commits.len();
        commits.push(entry("stash0", CommitKind::Stash));
        commits.push(entry("WORKING_DIRECTORY", CommitKind::Working));

        let moved = fit_first_chunk(&mut commits, walked);

        assert_eq!(
            commits.len(),
            CHUNK_COMMITS,
            "the chunk covers synthetic nodes too"
        );
        let moved: Vec<&str> = moved.iter().map(|c| c.sha.as_str()).collect();
        let last = format!("c{}", CHUNK_COMMITS - 1);
        let before_last = format!("c{}", CHUNK_COMMITS - 2);
        assert_eq!(
            moved,
            [before_last.as_str(), last.as_str()],
            "moved on, in walk order"
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
                .any(|c| c.sha == format!("c{}", CHUNK_COMMITS - 1)),
            "the oldest walked commits are the ones moved",
        );
    }

    /// Degenerate case: more synthetic nodes than a chunk holds.
    #[test]
    // Verifies: ADR-0016
    fn the_first_chunk_holds_even_with_absurdly_many_synthetic_nodes() {
        let mut commits: Vec<CommitData> = vec![entry("c0", CommitKind::Commit)];
        let walked = commits.len();
        for i in 0..(CHUNK_COMMITS + 5) {
            commits.push(entry(&format!("stash{i}"), CommitKind::Stash));
        }

        fit_first_chunk(&mut commits, walked);

        assert_eq!(commits.len(), CHUNK_COMMITS);
    }
}
