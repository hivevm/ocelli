//! The writes of G-5 and G-6 with git2: merge, rebase, cherry-pick, revert, and reset from a
//! clean tree, stopping on a conflict in `git`'s own state; and resolving such a conflict file
//! by file, then continuing or aborting the operation, whoever started it. No hook and no program
//! the repository names runs (ADR-0033).

use crate::checkout::{check_out, files_written, nothing_untracked_in_the_way};
use crate::guard::{
    NO_IDENTITY, branch_as_shown, nothing_in_progress, refname_of, refuse_where_signing_is_asked,
};
use crate::message::{Made, cleaned};
use crate::wildmatch::wildmatch;
use anyhow::{Context, Result, bail};
use git2::{
    BranchType, Commit, ErrorCode, Oid, Repository, ResetType, Signature, Status, StatusOptions,
};
use ocelli_core::model::Operation;
use ocelli_core::port::ResetMode;

/// Refuses unless the branch `name` is checked out at `tip`, no operation is in progress, and no
/// tracked file has an uncommitted change: where every history operation starts (ADR-0033).
fn clean_start<'r>(repo: &'r Repository, name: &str, tip: &str) -> Result<git2::Branch<'r>> {
    let branch = branch_as_shown(repo, name, tip)?;
    if !branch.is_head() {
        bail!("'{name}' is not checked out: the operation works on the checked-out branch");
    }
    nothing_in_progress(repo, false)?;
    let mut options = StatusOptions::new();
    options.include_untracked(false).include_ignored(false);
    let changed: Vec<String> = repo
        .statuses(Some(&mut options))?
        .iter()
        .filter(|e| e.status() != Status::CURRENT && !e.status().is_ignored())
        .map(|e| String::from_utf8_lossy(e.path_bytes()).into_owned())
        .collect();
    if !changed.is_empty() {
        bail!(
            "uncommitted changes to {}: commit or stash them first",
            changed.join(", ")
        );
    }
    Ok(branch)
}

fn commit_of<'r>(repo: &'r Repository, sha: &str) -> Result<Commit<'r>> {
    Oid::from_str(sha)
        .ok()
        .and_then(|oid| repo.find_commit(oid).ok())
        .with_context(|| format!("the commit {sha} is no longer in the repository"))
}

pub(crate) fn short(oid: Oid) -> String {
    oid.to_string()[..7].to_string()
}

/// The identity a commit is made by, where signing is not asked for (ADR-0033).
pub(crate) fn committer(repo: &Repository) -> Result<Signature<'static>> {
    refuse_where_signing_is_asked(repo, "commit.gpgSign", "a commit")?;
    Ok(repo.signature().context(NO_IDENTITY)?.to_owned())
}

/// Merges `commit` into the checked-out branch, as `git merge` does: nothing where it is in
/// already, a fast-forward where possible unless `merge.ff` is `false`, else a merge commit, or a
/// stop on a conflict in `git`'s state. Returns the commit the branch ends on, where it moved.
pub(crate) fn merge(
    repo: &Repository,
    name: &str,
    tip: &str,
    commit: &str,
    their_name: Option<&str>,
) -> Result<Option<String>> {
    let branch = clean_start(repo, name, tip)?;
    let refname = refname_of(&branch)?;
    let theirs = commit_of(repo, commit)?;
    let annotated = repo.find_annotated_commit(theirs.id())?;
    // The subject `git merge` writes: the branch as the developer named it, or the commit.
    let subject = match their_name {
        Some(n) if repo.find_branch(n, BranchType::Local).is_ok() => format!("Merge branch '{n}'"),
        Some(n) if repo.find_branch(n, BranchType::Remote).is_ok() => {
            format!("Merge remote-tracking branch '{n}'")
        }
        _ => format!("Merge commit '{}'", theirs.id()),
    };
    // git names the branch merged into, unless `merge.suppressDest` leaves it out.
    let subject = if dest_suppressed(repo, name)? {
        subject
    } else {
        format!("{subject} into {name}")
    };
    let (analysis, _) = repo.merge_analysis(&[&annotated])?;
    if analysis.is_up_to_date() {
        return Ok(None);
    }
    // `merge.ff` as git reads it: a boolean, or `only`.
    let ff = repo
        .config()?
        .get_string("merge.ff")
        .unwrap_or_default()
        .to_ascii_lowercase();
    let no_ff = matches!(ff.as_str(), "false" | "no" | "off" | "0");
    if analysis.is_fast_forward() && !no_ff {
        fast_forward(repo, name, &refname, tip, &theirs, "merge: Fast-forward")?;
        return Ok(Some(theirs.id().to_string()));
    }
    if ff == "only" {
        bail!(
            "merge.ff is only, and '{name}' cannot be fast-forwarded to {}",
            short(theirs.id())
        );
    }
    let me = committer(repo)?;
    let head = repo.head()?.peel_to_commit()?;
    let result = repo.merge_commits(&head, &theirs, None)?;
    nothing_untracked_in_the_way(repo, &files_written(&head.tree()?, &result)?)?;
    crate::workdir::merge(repo, &annotated, &result, their_name)?.map_err(in_the_way)?;
    // libgit2 prepared its own subject; git's goes first, its list of conflicts kept.
    let prepared = repo.path().join("MERGE_MSG");
    let rest: String = std::fs::read_to_string(&prepared)
        .unwrap_or_default()
        .lines()
        .skip(1)
        .map(|l| format!("{l}\n"))
        .collect();
    #[expect(
        clippy::disallowed_methods,
        reason = "a file of git's own directory, not of the working directory"
    )]
    std::fs::write(&prepared, format!("{subject}\n{rest}"))?;
    if repo.index()?.has_conflicts() {
        return Ok(None);
    }
    commit_in_progress(repo, Operation::Merge, Made::Merged, &me, None).map(Some)
}

/// Whether git leaves the branch merged into out of a merge's subject: `merge.suppressDest` holds
/// patterns, an empty one clearing those before it, and `main` and `master` where it is unset.
fn dest_suppressed(repo: &Repository, name: &str) -> Result<bool> {
    let config = repo.config()?;
    let mut patterns = Vec::new();
    let mut seen = false;
    if let Ok(mut entries) = config.multivar("merge.suppressDest", None) {
        while let Some(entry) = entries.next() {
            seen = true;
            match crate::guard::config_value(entry?)? {
                Some("") | None => patterns.clear(),
                Some(p) => patterns.push(p.to_string()),
            }
        }
    }
    if !seen {
        patterns = vec!["main".to_string(), "master".to_string()];
    }
    Ok(patterns
        .iter()
        .any(|p| wildmatch(p.as_bytes(), name.as_bytes())))
}

/// Moves the checked-out branch `refname` from `tip` to `target`, the working directory with it,
/// under the branch's lock and checked again there, as a pull's fast-forward does.
fn fast_forward(
    repo: &Repository,
    name: &str,
    refname: &str,
    tip: &str,
    target: &Commit,
    message: &str,
) -> Result<()> {
    let mut transaction = repo.transaction()?;
    crate::guard::lock_ref(&mut transaction, refname)
        .with_context(|| format!("'{name}' is locked by another program: try again later"))?;
    branch_as_shown(repo, name, tip)?;
    check_out(repo, target)?;
    crate::guard::set_target(&mut transaction, refname, target.id(), message)?;
    transaction.commit()?;
    Ok(())
}

/// Applies `commit` onto the checked-out branch and commits it with its author and message, as
/// `git cherry-pick` does, or stops on a conflict.
pub(crate) fn cherry_pick(
    repo: &Repository,
    name: &str,
    tip: &str,
    commit: &str,
) -> Result<Option<String>> {
    apply(repo, name, tip, commit, Operation::CherryPick)
}

/// Applies the inverse of `commit` onto the checked-out branch and commits it with `git`'s
/// message, as `git revert` does, or stops on a conflict.
pub(crate) fn revert(
    repo: &Repository,
    name: &str,
    tip: &str,
    commit: &str,
) -> Result<Option<String>> {
    apply(repo, name, tip, commit, Operation::Revert)
}

fn apply(
    repo: &Repository,
    name: &str,
    tip: &str,
    commit: &str,
    operation: Operation,
) -> Result<Option<String>> {
    clean_start(repo, name, tip)?;
    let picked = commit_of(repo, commit)?;
    if picked.parent_count() > 1 {
        bail!(
            "{} is a merge commit: which parent is its mainline is for git -m to say",
            short(picked.id())
        );
    }
    let me = committer(repo)?;
    let head = repo.head()?.peel_to_commit()?;
    let before = head.id();
    // What the result would put in the working directory, checked before libgit2 writes it.
    let result = match operation {
        Operation::Revert => repo.revert_commit(&picked, &head, 0, None)?,
        _ => repo.cherrypick_commit(&picked, &head, 0, None)?,
    };
    nothing_untracked_in_the_way(repo, &files_written(&head.tree()?, &result)?)?;
    let revert = operation == Operation::Revert;
    crate::workdir::pick(repo, &picked, revert, &result)?.map_err(in_the_way)?;
    if repo.index()?.has_conflicts() {
        // Stopped, `git` keeps where it started in ORIG_HEAD.
        set_orig_head(repo, Some(before))?;
        return Ok(None);
    }
    // A change already on the branch: refused, the state libgit2 wrote for it cleared again.
    let mut index = repo.index()?;
    if index.write_tree()? == repo.head()?.peel_to_commit()?.tree_id() {
        repo.cleanup_state()?;
        bail!(
            "{}'s changes are on '{name}' already: there is nothing to {}",
            short(picked.id()),
            operation.name()
        );
    }
    let author = (operation == Operation::CherryPick).then(|| picked.author().to_owned());
    commit_in_progress(repo, operation, Made::Taken, &me, author.as_ref()).map(Some)
}

/// Commits what the stopped `operation` left in the index, with the message it prepared, by
/// `author` where it keeps the original one, and clears `git`'s state of it.
pub(crate) fn commit_in_progress(
    repo: &Repository,
    operation: Operation,
    made: Made,
    me: &Signature,
    author: Option<&Signature>,
) -> Result<String> {
    let mut index = repo.index()?;
    if index.has_conflicts() {
        bail!("files still conflict: resolve each before continuing");
    }
    let tree = repo.find_tree(index.write_tree()?)?;
    let head = repo.head()?.peel_to_commit()?;
    if operation != Operation::Merge && tree.id() == head.tree_id() {
        bail!(
            "the {} changes nothing on the branch: abort it",
            operation.name()
        );
    }
    let mut parents = vec![head];
    if operation == Operation::Merge {
        // `MERGE_HEAD` lists the commits merged in, one id a line.
        let heads = std::fs::read_to_string(repo.path().join("MERGE_HEAD"))?;
        for line in heads.lines().filter(|l| !l.trim().is_empty()) {
            parents.push(repo.find_commit(Oid::from_str(line.trim())?)?);
        }
    }
    let prepared = std::fs::read_to_string(repo.path().join("MERGE_MSG")).unwrap_or_default();
    let message = cleaned(repo, &prepared, made)?;
    if message.is_empty() {
        bail!("the {} has no message to commit with", operation.name());
    }
    let parents: Vec<_> = parents.iter().collect();
    let made = repo.commit(
        Some("HEAD"),
        author.unwrap_or(me),
        me,
        &message,
        &tree,
        &parents,
    )?;
    repo.cleanup_state()?;
    Ok(made.to_string())
}

/// Rebases the checked-out branch onto `onto` through libgit2's rebase on disk, as `git rebase`
/// does without interaction, or stops on a conflict at the step that has one.
pub(crate) fn rebase(
    repo: &Repository,
    name: &str,
    tip: &str,
    onto: &str,
) -> Result<Option<String>> {
    let branch = clean_start(repo, name, tip)?;
    let me = committer(repo)?;
    let onto = repo.find_annotated_commit(commit_of(repo, onto)?.id())?;
    let ours = repo.reference_to_annotated_commit(branch.get())?;
    // A refusal leaves ORIG_HEAD as it was, which libgit2 points at the branch as it starts.
    let orig_head = repo
        .find_reference("ORIG_HEAD")
        .ok()
        .and_then(|r| r.target());
    let mut rebase = crate::workdir::Rebase::start(repo, &ours, &onto)?
        .map_err(in_the_way)
        .inspect_err(|_| {
            let _ = set_orig_head(repo, orig_head);
        })?;
    // A step that cannot be applied, an untracked file in its way, undoes the whole rebase:
    // nothing of the developer's is in it yet, and a rebase left at that step would skip it.
    let ran = run_rebase(repo, &mut rebase, &me);
    drop(rebase);
    ran.map_err(|e| {
        let undone = crate::conflicts::abort_rebase(repo);
        let _ = set_orig_head(repo, orig_head);
        match undone {
            Ok(()) => e,
            Err(not) => anyhow::anyhow!(
                "{e}; and the rebase could not be undone, so it is still in progress: {not}"
            ),
        }
    })
}

/// Commits each step of `rebase` in turn, a step already on the new base skipped, until it ends
/// or a step conflicts.
pub(crate) fn run_rebase(
    repo: &Repository,
    rebase: &mut crate::workdir::Rebase,
    me: &Signature,
) -> Result<Option<String>> {
    while let Some(step) = rebase.next()? {
        if let Err(e) = step {
            // libgit2 has moved on to the step it could not apply: a later commit would take it
            // for one resolved to nothing and drop it, so it is marked as never applied.
            if let Some(dir) = rebase_dir(repo) {
                #[expect(clippy::disallowed_methods, reason = "a file of git's own directory")]
                std::fs::write(dir.join(UNAPPLIED), "")?;
            }
            return Err(in_the_way(e).into());
        }
        if repo.index()?.has_conflicts() {
            return Ok(None);
        }
        match rebase.commit(me) {
            Ok(_) => {}
            Err(e) if e.code() == ErrorCode::Applied => {}
            Err(e) => return Err(e.into()),
        }
    }
    rebase.finish(me)?;
    Ok(Some(repo.head()?.peel_to_commit()?.id().to_string()))
}

/// Moves the checked-out branch to `commit`, as `git reset` does in `mode`; a hard one discards
/// work and comes only with the developer's confirmation (ADR-0034).
pub(crate) fn reset(
    repo: &Repository,
    name: &str,
    tip: &str,
    commit: &str,
    mode: ResetMode,
) -> Result<Option<String>> {
    let branch = branch_as_shown(repo, name, tip)?;
    if !branch.is_head() {
        bail!("'{name}' is not checked out: a reset moves the checked-out branch");
    }
    nothing_in_progress(repo, false)?;
    let target = commit_of(repo, commit)?;
    let kind = match mode {
        ResetMode::Soft => ResetType::Soft,
        ResetMode::Mixed => ResetType::Mixed,
        ResetMode::Hard => ResetType::Hard,
    };
    let before = repo.head()?.peel_to_commit()?.id();
    crate::workdir::reset(repo, &target, kind)?;
    // `git reset` keeps where the branch was in ORIG_HEAD.
    set_orig_head(repo, Some(before))?;
    Ok(Some(target.id().to_string()))
}

/// Points `ORIG_HEAD` at `commit`, or removes it where there is none, as `git` keeps where an
/// operation started from.
pub(crate) fn set_orig_head(repo: &Repository, commit: Option<Oid>) -> Result<()> {
    match commit {
        Some(oid) => {
            repo.reference("ORIG_HEAD", oid, true, "")?;
        }
        None => {
            if let Ok(mut orig) = repo.find_reference("ORIG_HEAD") {
                orig.delete()?;
            }
        }
    }
    Ok(())
}

/// libgit2's message where a file is in an operation's way, which says only "no error", made to
/// say what it is.
pub(crate) fn in_the_way(e: git2::Error) -> git2::Error {
    if e.code() == ErrorCode::Conflict {
        git2::Error::new(
            ErrorCode::Conflict,
            e.class(),
            "files in the working directory are in the way: move them, or commit them, first",
        )
    } else {
        e
    }
}

/// The file in libgit2's rebase directory that marks its current step as never applied.
pub(crate) const UNAPPLIED: &str = "ocelli-unapplied";

/// The directory a rebase in progress keeps its state in, `git`'s or libgit2's.
pub(crate) fn rebase_dir(repo: &Repository) -> Option<std::path::PathBuf> {
    ["rebase-merge", "rebase-apply"]
        .iter()
        .map(|d| repo.path().join(d))
        .find(|d| d.is_dir())
}
