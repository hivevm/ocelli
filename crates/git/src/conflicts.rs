//! Resolving a stopped operation's conflicts file by file, and continuing or aborting the
//! operation, whoever started it, running no program the repository names (ADR-0031).

use crate::checkout::{entry_as_in, named};
use crate::git::WORKTREE;
use crate::guard::{bytes_to_path, exact_path};
use crate::message::Made;
use crate::operations::{
    UNAPPLIED, commit_in_progress, committer, rebase_dir, run_rebase, set_orig_head,
};
use crate::refs::{conflicts_of, stopped};
use anyhow::{Context, Result, bail};
use git2::build::CheckoutBuilder;
use git2::{
    ErrorCode, IndexConflict, IndexEntry, ObjectType, Oid, Repository, RepositoryState, ResetType,
    Status, StatusOptions,
};
use ocelli_core::merge::{MergeFile, as_text, blocks, has_marker, with_line_endings};
use ocelli_core::model::{Conflict, Operation};
use ocelli_core::port::Take;
use std::path::{Path, PathBuf};

/// Resolves the conflicted file `path`, still in the conflict `expected`, by writing the side
/// `take` names, or the working directory's file as the developer edited it, to the index, as
/// `git checkout --ours/--theirs` and `git add` do (ADR-0031).
pub(crate) fn resolve(
    repo: &Repository,
    path: &str,
    expected: &Conflict,
    take: Take,
) -> Result<()> {
    crate::guard::named_exactly(repo, path)?;
    let mut index = repo.index()?;
    if conflicts_of(&index).iter().find(|c| c.path == path) != Some(expected) {
        bail!("{path} no longer conflicts as it was shown: reload to see it as it is");
    }
    let root = repo
        .workdir()
        .context("a bare repository has no working directory")?;
    let file = root.join(path);
    within(root, Path::new(path), &|_| false)?;
    // A link is looked at, never followed: what it points to may lie outside the repository.
    let there = file.symlink_metadata().ok();
    match take {
        Take::Ours | Take::Theirs => {
            let IndexConflict {
                ancestor,
                our: ours,
                their: theirs,
            } = conflict_at(&index, path)?;
            let side = if take == Take::Ours { &ours } else { &theirs };
            let side = side.as_ref().map(|e| IndexEntry {
                flags: e.flags & !STAGE_MASK,
                path: e.path.clone(),
                ..*e
            });
            // A directory the developer made in the file's place is theirs to move, not ours to
            // delete; a submodule's is its checkout.
            let gitlink = [&ancestor, &ours, &theirs]
                .into_iter()
                .flatten()
                .any(|e| e.mode == 0o160000);
            if there.as_ref().is_some_and(|m| m.is_dir()) && !gitlink {
                bail!("{path} is a directory now: move it out of the way first");
            }
            index.conflict_remove(Path::new(path))?;
            match side {
                Some(entry) => {
                    index.add(&entry)?;
                    index.write()?;
                    let mut checkout = CheckoutBuilder::new();
                    #[expect(
                        clippy::disallowed_methods,
                        reason = "the pattern matching is turned off, and the path is a conflict libgit2 read"
                    )]
                    checkout.force().disable_pathspec_match(true).path(path);
                    let written = crate::workdir::checkout_index(repo, &mut index, &mut checkout)
                        .and_then(|w| w.map_err(anyhow::Error::from));
                    if let Err(e) = written {
                        // The file could not be written: the conflict stays as it was.
                        restore(repo, path, [&ancestor, &ours, &theirs])?;
                        return Err(e);
                    }
                }
                None => {
                    index.write()?;
                    if there.is_some_and(|m| !m.is_dir()) {
                        #[expect(
                            clippy::disallowed_methods,
                            reason = "within() refused a link or a file above it, and a link itself is removed, never followed"
                        )]
                        std::fs::remove_file(&file)?;
                    }
                }
            }
        }
        Take::AsEdited => {
            if there.as_ref().is_some_and(|m| m.is_file())
                && let Ok(text) = std::fs::read(&file)
                && has_marker(&String::from_utf8_lossy(&text))
            {
                bail!("{path} still holds a conflict marker: finish editing it first");
            }
            if there.is_some() {
                index.add_path(Path::new(path))?;
            } else {
                index.remove_path(Path::new(path))?;
            }
            index.write()?;
        }
    }
    Ok(())
}

/// The stages of the conflicted file `path` in `index`.
fn conflict_at(index: &git2::Index, path: &str) -> Result<IndexConflict> {
    index
        .conflicts()?
        .flatten()
        .find(|c| {
            let any = c.our.as_ref().or(c.their.as_ref()).or(c.ancestor.as_ref());
            any.is_some_and(|e| e.path == path.as_bytes())
        })
        .context("the conflict is gone from the index")
}

/// Puts the conflict of `path` back into the index as its stages were.
fn restore(repo: &Repository, path: &str, stages: [&Option<IndexEntry>; 3]) -> Result<()> {
    let mut index = repo.index()?;
    index.remove_path(Path::new(path))?;
    for staged in stages.into_iter().flatten() {
        index.add(staged)?;
    }
    index.write()?;
    Ok(())
}

/// Whether both sides of `conflict`, ours and theirs, are files holding text, the conflicts the
/// merge editor opens (ADR-0031): neither deleted, a link, or a submodule, and neither holding a
/// NUL byte or anything but UTF-8.
pub(crate) fn editable(repo: &Repository, conflict: &IndexConflict) -> bool {
    [&conflict.our, &conflict.their].into_iter().all(|side| {
        side.as_ref().is_some_and(|e| {
            matches!(e.mode, 0o100644 | 0o100755)
                && repo
                    .find_blob(e.id)
                    .is_ok_and(|b| as_text(b.content()).is_some())
        })
    })
}

/// The working directory's conflicted file `path`, read for the merge editor with the bytes it
/// holds: refused where the editor does not open it.
fn opened_file(repo: &Repository, path: &str) -> Result<(PathBuf, Vec<u8>)> {
    let index = repo.index()?;
    let conflict =
        conflict_at(&index, path).with_context(|| format!("{path} is not conflicted"))?;
    if !editable(repo, &conflict) {
        bail!(
            "the merge editor opens a file whose sides both hold text: resolve {path} by ours, \
             theirs, or as edited"
        );
    }
    let root = repo
        .workdir()
        .context("a bare repository has no working directory")?;
    within(root, Path::new(path), &|_| false)?;
    let file = root.join(path);
    // A link is looked at, never followed: what it points to may lie outside the repository.
    if !file.symlink_metadata().is_ok_and(|m| m.is_file()) {
        bail!("{path} is no file in the working directory: resolve it by ours or theirs");
    }
    let bytes = std::fs::read(&file)?;
    Ok((file, bytes))
}

/// Reads the conflicted file `path` for the merge editor: its text as the working directory
/// holds it, edits made since included, its blob id, and its blocks (ADR-0031).
pub(crate) fn load_merge_file(repo_path: &Path, path: &str) -> Result<MergeFile> {
    crate::guard::repository_path(path)?;
    let repo = crate::confine::open(repo_path)?;
    crate::guard::named_exactly(&repo, path)?;
    let (_, bytes) = opened_file(&repo, path)?;
    let text = as_text(&bytes).with_context(|| {
        format!("{path} no longer holds text in the working directory: resolve it as edited")
    })?;
    Ok(MergeFile {
        text: text.to_string(),
        blob: Oid::hash_object(ObjectType::Blob, &bytes)?.to_string(),
        blocks: blocks(text),
    })
}

/// Saves the merge editor's `result` as the resolution of `path`, still in the conflict
/// `expected` and its file still the blob `opened`: the working directory gets the result, its
/// line endings as the file had them, through libgit2's checkout, and the index what `git add`
/// stages for the file holding it (ADR-0031).
pub(crate) fn save_merge(
    repo: &Repository,
    path: &str,
    expected: &Conflict,
    opened: &str,
    result: &str,
) -> Result<()> {
    crate::guard::named_exactly(repo, path)?;
    let mut index = repo.index()?;
    if conflicts_of(&index).iter().find(|c| c.path == path) != Some(expected) {
        bail!("{path} no longer conflicts as it was shown: reload to see it as it is");
    }
    let (_, bytes) = opened_file(repo, path)?;
    if Oid::hash_object(ObjectType::Blob, &bytes)?.to_string() != opened {
        bail!(
            "{path} changed in the working directory since the merge editor opened it: open it again"
        );
    }
    if has_marker(result) {
        bail!("{path} still holds a conflict marker: finish editing it first");
    }
    let written = with_line_endings(&String::from_utf8_lossy(&bytes), result);
    let IndexConflict {
        ancestor,
        our: ours,
        their: theirs,
    } = conflict_at(&index, path)?;
    let side = ours.as_ref().context("the conflict has no side of ours")?;
    // The file as it is: its mode the working directory's, which `git add` takes, not ours.
    let entry = IndexEntry {
        flags: side.flags & !STAGE_MASK,
        path: side.path.clone(),
        id: repo.blob(written.as_bytes())?,
        file_size: u32::try_from(written.len()).unwrap_or(u32::MAX),
        mode: mode_of(repo, path, side.mode),
        ..*side
    };
    index.conflict_remove(Path::new(path))?;
    index.add(&entry)?;
    index.write()?;
    // The result as it stands, no filter between: `git add` below cleans it as it cleans a file.
    let mut checkout = CheckoutBuilder::new();
    #[expect(
        clippy::disallowed_methods,
        reason = "the pattern matching is turned off, and the path is a conflict libgit2 read"
    )]
    checkout
        .force()
        .disable_filters(true)
        .disable_pathspec_match(true)
        .path(path);
    let checked_out = crate::workdir::checkout_index(repo, &mut index, &mut checkout)
        .and_then(|w| w.map_err(anyhow::Error::from));
    // The conflict back as it was, so that `git add` meets the stages it meets: the file it
    // stages is cleaned against them, a CRLF the result was written with taken back to LF.
    restore(repo, path, [&ancestor, &ours, &theirs])?;
    checked_out?;
    let mut index = repo.index()?;
    index.add_path(Path::new(path))?;
    index.write()?;
    Ok(())
}

/// The mode of the working directory's file at `path`, as `git add` stages it: executable where
/// the file is and `core.fileMode` reads it, else `ours`'s.
fn mode_of(repo: &Repository, path: &str, ours: u32) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let file_mode = repo
            .config()
            .and_then(|c| c.get_bool("core.fileMode"))
            .unwrap_or(true);
        if let (true, Some(root)) = (file_mode, repo.workdir())
            && let Ok(meta) = root.join(path).symlink_metadata()
        {
            return if meta.permissions().mode() & 0o111 != 0 {
                0o100755
            } else {
                0o100644
            };
        }
    }
    let _ = (repo, path);
    ours
}

/// Refuses a `path` of the working directory below a link or a file, other than one `its_own`
/// says the operation put there: writing it would follow the link, out of the repository
/// perhaps, or fail half done.
fn within(root: &Path, path: &Path, its_own: &dyn Fn(&Path) -> bool) -> Result<()> {
    let mut above: Vec<&Path> = path
        .ancestors()
        .skip(1)
        .filter(|a| !a.as_os_str().is_empty())
        .collect();
    above.reverse();
    for dir in above {
        if its_own(dir) {
            continue;
        }
        match root.join(dir).symlink_metadata() {
            Ok(m) if m.file_type().is_symlink() => bail!(
                "{} is a link where {} needs a directory: move it out of the way first",
                named(dir),
                named(path)
            ),
            Ok(m) if !m.is_dir() => bail!(
                "{} is a file where {} needs a directory: move it out of the way first",
                named(dir),
                named(path)
            ),
            Ok(_) => {}
            Err(_) => break,
        }
    }
    Ok(())
}

/// The bits of an index entry's flags that hold its stage.
const STAGE_MASK: u16 = 0x3000;

/// Refuses unless `operation` is the one in progress and `HEAD` is still at `head`.
fn stopped_as_shown(repo: &Repository, operation: Operation, head: &str) -> Result<()> {
    if stopped(repo).map(|s| s.operation) != Some(operation) {
        bail!("no {} is in progress any more: reload", operation.name());
    }
    let at = repo.head()?.peel_to_commit()?.id().to_string();
    if at != head {
        bail!("HEAD is no longer where it was shown: reload to see it as it is");
    }
    Ok(())
}

/// Continues the stopped `operation` once nothing conflicts, as `git <operation> --continue`
/// does: commits it, a rebase going on to its next step (ADR-0031).
pub(crate) fn continue_stopped(
    repo: &Repository,
    operation: Operation,
    head: &str,
) -> Result<Option<String>> {
    stopped_as_shown(repo, operation, head)?;
    left_to_git(repo, "continue")?;
    if repo.index()?.has_conflicts() {
        bail!("files still conflict: resolve each before continuing");
    }
    let me = committer(repo)?;
    match operation {
        Operation::Rebase => {
            if started_by_git(repo) {
                bail!(
                    "this rebase was started by git, whose steps libgit2 cannot read: continue \
                     it with git rebase --continue, or abort it here"
                );
            }
            if rebase_dir(repo).is_some_and(|d| d.join(UNAPPLIED).exists()) {
                bail!(
                    "a step of this rebase could not be applied, files in the working directory \
                     in its way: abort the rebase, which puts the branch back as it was"
                );
            }
            let mut rebase = crate::workdir::Rebase::open(repo)?;
            // A step resolved to nothing is dropped, as `git rebase --continue` drops it.
            match rebase.commit(&me) {
                Ok(_) => {}
                Err(e) if e.code() == ErrorCode::Applied => {}
                Err(e) => return Err(e.into()),
            }
            run_rebase(repo, &mut rebase, &me)
        }
        Operation::CherryPick => {
            let picked = repo.find_reference("CHERRY_PICK_HEAD")?.peel_to_commit()?;
            let author = picked.author().to_owned();
            commit_in_progress(repo, operation, Made::PickContinued, &me, Some(&author)).map(Some)
        }
        Operation::Revert => {
            commit_in_progress(repo, operation, Made::PickContinued, &me, None).map(Some)
        }
        Operation::Merge => {
            commit_in_progress(repo, operation, Made::MergeContinued, &me, None).map(Some)
        }
    }
}

/// Aborts the stopped `operation`, back to where the developer was, as `git <operation> --abort`
/// does: the files it touched as the checked-out commit has them, its state gone; a change of the
/// developer's own to another file stays (ADR-0031).
pub(crate) fn abort(repo: &Repository, operation: Operation, head: &str) -> Result<()> {
    stopped_as_shown(repo, operation, head)?;
    left_to_git(repo, "abort")?;
    if operation == Operation::Rebase {
        return abort_rebase(repo);
    }
    let head = repo.head()?.peel_to_commit()?;
    let tree = head.tree()?;
    let index = repo.index()?;
    // Each path in the bytes libgit2 read, as it takes it back, a name that is not UTF-8 among
    // them, which Windows and macOS cannot hold and refuse.
    let mut touched = repo
        .diff_tree_to_index(Some(&tree), Some(&index), None)?
        .deltas()
        .filter_map(|d| d.new_file().path_bytes().or(d.old_file().path_bytes()))
        .map(exact_path)
        .collect::<Result<Vec<PathBuf>>>()?;
    let conflicted = index
        .conflicts()?
        .flatten()
        .filter_map(|c| c.our.or(c.their).or(c.ancestor))
        .map(|e| exact_path(&e.path))
        .collect::<Result<Vec<PathBuf>>>()?;
    let root = repo
        .workdir()
        .context("a bare repository has no working directory")?;
    // The working directory against the index, every file in it named.
    let mut options = StatusOptions::new();
    options
        .include_untracked(true)
        .recurse_untracked_dirs(true)
        .include_ignored(false)
        // A submodule's own checkout is its own: an abort moves only the commit it is pinned at.
        .exclude_submodules(true);
    let changed: std::collections::HashMap<PathBuf, Status> = repo
        .statuses(Some(&mut options))?
        .iter()
        .map(|e| (bytes_to_path(e.path_bytes()), e.status()))
        .filter(|(_, s)| s.intersects(WORKTREE))
        .collect();
    // A file the operation brought in cleanly or removed, which the developer changed or made
    // since, stays, as `git <operation> --abort` refuses rather than lose it.
    let below = |dir: &Path| {
        let dir = [dir.as_os_str().as_encoded_bytes(), b"/"].concat();
        changed
            .keys()
            .any(|p| p.as_os_str().as_encoded_bytes().starts_with(&dir))
    };
    // A conflicted file stands to be lost, but not a directory the developer made in its place.
    for path in touched.iter().chain(&conflicted) {
        if (!conflicted.contains(path) && changed.contains_key(path)) || below(path) {
            bail!(
                "{} was changed after the {} stopped: commit, move, or undo that change \
                 before aborting",
                named(path),
                operation.name()
            );
        }
    }
    touched.extend(conflicted);
    touched.sort();
    touched.dedup();
    if !touched.is_empty() {
        // A file or a link in a directory's place is let be only where the operation put it
        // there, unchanged; it is removed before the checkout, which would write through a link.
        let own_parents = std::cell::RefCell::new(Vec::new());
        let its_own = |dir: &Path| {
            let own = crate::git::index_entry(&index, dir, 0).is_some()
                && !changed.contains_key(dir)
                && root.join(dir).symlink_metadata().is_ok_and(|m| !m.is_dir());
            if own {
                own_parents.borrow_mut().push(dir.to_path_buf());
            }
            own
        };
        for path in &touched {
            within(root, path, &its_own)?;
        }
        let mut own_parents = own_parents.into_inner();
        // A file or link the operation put where the checked-out commit has a submodule goes too:
        // libgit2 would meet it where it makes the submodule's directory.
        for path in &touched {
            let gitlink = tree
                .get_path(path)
                .is_ok_and(|e| e.kind() == Some(git2::ObjectType::Commit));
            if gitlink
                && root
                    .join(path)
                    .symlink_metadata()
                    .is_ok_and(|m| !m.is_dir())
                && crate::git::index_entry(&index, path, 0).is_some()
                && !changed.contains_key(path)
            {
                own_parents.push(path.clone());
            }
        }
        own_parents.sort();
        own_parents.dedup();
        // The index as it is, put back should the checkout fail, the operation still stopped.
        let index_file = repo.path().join("index");
        let kept = std::fs::read(&index_file)?;
        // Each path as the checked-out commit has it, in the index, taken literally.
        let mut index = repo.index()?;
        for path in &touched {
            index.conflict_remove(path).or_else(not_found)?;
            entry_as_in(&mut index, &tree, path)?;
        }
        index.write()?;
        // libgit2's checkout writes each file with its mode, a link as a link, and removes what
        // the checked-out commit does not have.
        let mut checkout = CheckoutBuilder::new();
        checkout
            .force()
            .remove_untracked(true)
            .disable_pathspec_match(true);
        for path in &touched {
            #[expect(
                clippy::disallowed_methods,
                reason = "the pattern matching is turned off, and a path libgit2 read holds no NUL \
                          and is UTF-8 on Windows, where git2 would unwrap it as such"
            )]
            checkout.path(path.as_path());
        }
        // Each moved aside, into the repository's own directory, and back should the abort fail.
        let aside = |i: usize| repo.path().join(format!("ocelli-aside-{i}"));
        let mut moved = 0;
        let mut done = Ok(());
        for (i, dir) in own_parents.iter().enumerate() {
            #[expect(
                clippy::disallowed_methods,
                reason = "a file or link the operation made, unchanged, moved aside into git's \
                          own directory; a link is moved, never followed"
            )]
            let renamed = std::fs::rename(root.join(dir), aside(i));
            done = renamed.map_err(anyhow::Error::from);
            if done.is_err() {
                break;
            }
            moved += 1;
        }
        if done.is_ok() {
            done = crate::workdir::checkout_head(repo, &mut checkout)
                .and_then(|c| c.map_err(anyhow::Error::from));
        }
        if done.is_ok() {
            for i in 0..moved {
                #[expect(
                    clippy::disallowed_methods,
                    reason = "what was moved aside into git's own directory"
                )]
                std::fs::remove_file(aside(i))?;
            }
        }
        if let Err(e) = done {
            for (i, dir) in own_parents.iter().enumerate().take(moved) {
                #[expect(
                    clippy::disallowed_methods,
                    reason = "what was moved aside put back as it was"
                )]
                let _ = std::fs::rename(aside(i), root.join(dir));
            }
            #[expect(
                clippy::disallowed_methods,
                reason = "the index file of git's own directory, put back as it was"
            )]
            std::fs::write(&index_file, kept)?;
            bail!(
                "the {} could not be aborted, its conflicts kept: {e}",
                operation.name()
            );
        }
    }
    repo.cleanup_state()?;
    // `git <operation> --abort` resets to the checked-out commit, which points ORIG_HEAD there.
    set_orig_head(repo, Some(repo.head()?.peel_to_commit()?.id()))?;
    no_auto_merge(repo)
}

/// Removes the `AUTO_MERGE` `git` writes where an operation stops, as its abort does.
fn no_auto_merge(repo: &Repository) -> Result<()> {
    if let Ok(mut auto) = repo.find_reference("AUTO_MERGE") {
        auto.delete()?;
    }
    Ok(())
}

/// Treats a path the index has no conflict for as done.
fn not_found(e: git2::Error) -> std::result::Result<(), git2::Error> {
    if e.code() == ErrorCode::NotFound {
        Ok(())
    } else {
        Err(e)
    }
}

/// Whether the rebase in progress keeps its steps as `git`'s sequencer does, in a todo list
/// libgit2 does not read.
fn started_by_git(repo: &Repository) -> bool {
    rebase_dir(repo)
        .is_some_and(|d| d.join("git-rebase-todo").exists() || d.join("rebasing").exists())
}

/// Aborts a rebase as `git rebase --abort` does, from the files `git` and libgit2 both write:
/// the branch it rebased, `head-name`, checked out again at `orig-head`, where it stood before,
/// and the rebase's state removed.
pub(crate) fn abort_rebase(repo: &Repository) -> Result<()> {
    let dir = rebase_dir(repo).context("the rebase keeps no state to abort from")?;
    let read = |name: &str| -> Result<String> {
        Ok(std::fs::read_to_string(dir.join(name))
            .with_context(|| format!("the rebase's {name} cannot be read"))?
            .trim()
            .to_string())
    };
    let before = repo.find_commit(Oid::from_str(&read("orig-head")?)?)?;
    let branch = read("head-name")?;
    crate::workdir::reset(repo, &before, ResetType::Hard)?;
    if branch.starts_with("refs/heads/") {
        repo.reference(&branch, before.id(), true, "rebase: abort")?;
        repo.set_head(&branch)?;
    } else {
        repo.set_head_detached(before.id())?;
    }
    // A hard reset clears libgit2's own record of the rebase already.
    if dir.exists() {
        #[expect(
            clippy::disallowed_methods,
            reason = "the rebase's state in git's own directory"
        )]
        std::fs::remove_dir_all(&dir)?;
    }
    no_auto_merge(repo)
}

/// Refuses to `act` on what only `git` carries on rightly: a sequence of cherry-picks or reverts,
/// whose later commits `git` keeps in its sequencer, and an operation that holds an autostash,
/// which `git` puts back when it ends.
fn left_to_git(repo: &Repository, act: &str) -> Result<()> {
    if matches!(
        repo.state(),
        RepositoryState::CherryPickSequence | RepositoryState::RevertSequence
    ) || repo.path().join("sequencer").is_dir()
    {
        bail!(
            "this is one step of a sequence git started: {act} it with git, which knows the rest"
        );
    }
    let autostash = repo.path().join("MERGE_AUTOSTASH").exists()
        || rebase_dir(repo).is_some_and(|d| d.join("autostash").exists());
    if autostash {
        bail!("this operation holds an autostash, which git puts back: {act} it with git");
    }
    Ok(())
}
