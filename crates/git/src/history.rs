//! The writes of G-5 and G-6 with git2: merge, rebase, cherry-pick, revert, and reset from a
//! clean tree, stopping on a conflict in `git`'s own state (ADR-0033); and resolving such a
//! conflict file by file, then continuing or aborting the operation, whoever started it
//! (ADR-0033). No hook and no program the repository names runs (ADR-0033).

use crate::write::{
    NO_IDENTITY, branch_as_shown, check_out, entry_as_in, in_progress, refname_of,
    refuse_where_signing_is_asked,
};
use anyhow::{Context, Result, bail};
use git2::build::CheckoutBuilder;
use git2::{
    BranchType, Commit, ErrorCode, IndexEntry, Oid, Repository, RepositoryState, ResetType,
    Signature, Status, StatusOptions,
};
use ocelli_core::model::{Conflict, InProgress, Operation};
use ocelli_core::port::{ResetMode, Take};
use std::path::Path;

/// Refuses unless the branch `name` is checked out at `tip`, no operation is in progress, and no
/// tracked file has an uncommitted change: where every operation of ADR-0033 starts.
fn clean_start<'r>(repo: &'r Repository, name: &str, tip: &str) -> Result<git2::Branch<'r>> {
    let branch = branch_as_shown(repo, name, tip)?;
    if !branch.is_head() {
        bail!("'{name}' is not checked out: the operation works on the checked-out branch");
    }
    if repo.state() != RepositoryState::Clean {
        bail!(
            "a {} is in progress: finish or abort it first",
            in_progress(repo.state())
        );
    }
    let mut options = StatusOptions::new();
    options.include_untracked(false).include_ignored(false);
    let changed: Vec<String> = repo
        .statuses(Some(&mut options))?
        .iter()
        .filter(|e| e.status() != Status::CURRENT && !e.status().is_ignored())
        .filter_map(|e| e.path().map(str::to_string))
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

fn short(oid: Oid) -> String {
    oid.to_string()[..7].to_string()
}

/// The identity a commit is made by, where signing is not asked for (ADR-0033).
fn committer(repo: &Repository) -> Result<Signature<'static>> {
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
    repo.merge(&[&annotated], None, None).map_err(in_the_way)?;
    // libgit2 prepared its own subject; git's goes first, its list of conflicts kept.
    let prepared = repo.path().join("MERGE_MSG");
    let rest: String = std::fs::read_to_string(&prepared)
        .unwrap_or_default()
        .lines()
        .skip(1)
        .map(|l| format!("{l}\n"))
        .collect();
    std::fs::write(&prepared, format!("{subject}\n{rest}"))?;
    if repo.index()?.has_conflicts() {
        return Ok(None);
    }
    commit_in_progress(repo, Operation::Merge, &me, None).map(Some)
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
            match entry?.value() {
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

/// A pattern matched as git's wildmatch matches a ref with `WM_PATHNAME`: `*` and `?` stop at a
/// `/`, `**` as a whole component does not, `[...]` is a class, `[!...]` or `[^...]` its
/// complement, `\` escapes.
fn wildmatch(pattern: &[u8], text: &[u8]) -> bool {
    matches_from(pattern, text, true)
}

/// `wildmatch`, `component` telling whether `pattern` starts a component of the path.
fn matches_from(pattern: &[u8], text: &[u8], component: bool) -> bool {
    let wildmatch = |p: &[u8], t: &[u8]| matches_from(p, t, false);
    match pattern {
        [] => text.is_empty(),
        [b'*', b'*', rest @ ..] if component && matches!(rest, [] | [b'/', ..]) => match rest {
            [] => true,
            [_, rest @ ..] => (0..=text.len())
                .filter(|&i| i == 0 || text[i - 1] == b'/')
                .any(|i| matches_from(rest, &text[i..], true)),
        },
        [b'*', rest @ ..] => {
            let rest = rest.strip_prefix(b"*").unwrap_or(rest);
            (0..=text.len())
                .take_while(|&i| i == 0 || text[i - 1] != b'/')
                .any(|i| matches_from(rest, &text[i..], false))
        }
        [b'?', rest @ ..] => {
            matches!(text.first(), Some(c) if *c != b'/') && wildmatch(rest, &text[1..])
        }
        [b'[', rest @ ..] => {
            let Some((&c, after)) = text.split_first() else {
                return false;
            };
            let (negated, mut class) = match rest {
                [b'!' | b'^', class @ ..] => (true, class),
                class => (false, class),
            };
            let mut hit = false;
            let mut first = true;
            loop {
                if let [b'[', b':', more @ ..] = class
                    && let Some(end) = more.windows(2).position(|w| w == b":]")
                {
                    hit |= posix_class(&more[..end], c);
                    class = &more[end + 2..];
                    first = false;
                    continue;
                }
                let (lo, more) = match class {
                    [] => return false,
                    [b']', more @ ..] if !first => {
                        class = more;
                        break;
                    }
                    [b'\\', x, more @ ..] | [x, more @ ..] => (*x, more),
                };
                let (hi, more) = match more {
                    [b'-', b'\\', hi, more @ ..] => (*hi, more),
                    [b'-', hi, more @ ..] if *hi != b']' => (*hi, more),
                    _ => (lo, more),
                };
                // git takes a range's first character on its own, whatever the range.
                hit |= c == lo || (lo..=hi).contains(&c);
                class = more;
                first = false;
            }
            hit != negated && c != b'/' && wildmatch(class, after)
        }
        [b'\\', c, rest @ ..] | [c, rest @ ..] => {
            text.first() == Some(c) && matches_from(rest, &text[1..], *c == b'/')
        }
    }
}

/// Whether `c` is in the POSIX class `name`, as in `[[:digit:]]`.
fn posix_class(name: &[u8], c: u8) -> bool {
    match name {
        b"alnum" => c.is_ascii_alphanumeric(),
        b"alpha" => c.is_ascii_alphabetic(),
        b"blank" => c == b' ' || c == b'\t',
        b"cntrl" => c.is_ascii_control(),
        b"digit" => c.is_ascii_digit(),
        b"graph" => c.is_ascii_graphic(),
        b"lower" => c.is_ascii_lowercase(),
        b"print" => c.is_ascii_graphic() || c == b' ',
        b"punct" => c.is_ascii_punctuation(),
        b"space" => c.is_ascii_whitespace() || c == 0x0b,
        b"upper" => c.is_ascii_uppercase(),
        b"xdigit" => c.is_ascii_hexdigit(),
        _ => false,
    }
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
    transaction
        .lock_ref(refname)
        .with_context(|| format!("'{name}' is locked by another program: try again later"))?;
    branch_as_shown(repo, name, tip)?;
    check_out(repo, target)?;
    transaction.set_target(refname, target.id(), None, message)?;
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
    match operation {
        Operation::Revert => repo.revert(&picked, None).map_err(in_the_way)?,
        _ => repo.cherrypick(&picked, None).map_err(in_the_way)?,
    }
    if repo.index()?.has_conflicts() {
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
    commit_in_progress(repo, operation, &me, author.as_ref()).map(Some)
}

/// Commits what the stopped `operation` left in the index, with the message it prepared, by
/// `author` where it keeps the original one, and clears `git`'s state of it.
fn commit_in_progress(
    repo: &Repository,
    operation: Operation,
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
    let message = git2::message_prettify(without_conflict_list(&prepared), None)?;
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
    let mut rebase = repo
        .rebase(Some(&ours), Some(&onto), None, None)
        .map_err(in_the_way)?;
    // A step that cannot be applied, an untracked file in its way, undoes the whole rebase:
    // nothing of the developer's is in it yet, and a rebase left at that step would skip it.
    run_rebase(repo, &mut rebase, &me).inspect_err(|_| {
        let _ = rebase.abort();
    })
}

/// Commits each step of `rebase` in turn, a step already on the new base skipped, until it ends
/// or a step conflicts.
fn run_rebase(
    repo: &Repository,
    rebase: &mut git2::Rebase,
    me: &Signature,
) -> Result<Option<String>> {
    while let Some(step) = rebase.next() {
        if let Err(e) = step {
            // libgit2 has moved on to the step it could not apply: a later commit would take it
            // for one resolved to nothing and drop it, so it is marked as never applied.
            if let Some(dir) = rebase_dir(repo) {
                std::fs::write(dir.join(UNAPPLIED), "")?;
            }
            return Err(in_the_way(e).into());
        }
        if repo.index()?.has_conflicts() {
            return Ok(None);
        }
        match rebase.commit(None, me, None) {
            Ok(_) => {}
            Err(e) if e.code() == ErrorCode::Applied => {}
            Err(e) => return Err(e.into()),
        }
    }
    rebase.finish(Some(me))?;
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
    if repo.state() != RepositoryState::Clean {
        bail!(
            "a {} is in progress: finish or abort it first",
            in_progress(repo.state())
        );
    }
    let target = commit_of(repo, commit)?;
    let kind = match mode {
        ResetMode::Soft => ResetType::Soft,
        ResetMode::Mixed => ResetType::Mixed,
        ResetMode::Hard => ResetType::Hard,
    };
    repo.reset(target.as_object(), kind, None)?;
    Ok(Some(target.id().to_string()))
}

/// The operation stopped in the repository, as `git status` reads it, with its conflicted files
/// (ADR-0033).
pub(crate) fn stopped(repo: &Repository) -> Option<InProgress> {
    let operation = match repo.state() {
        RepositoryState::Merge => Operation::Merge,
        RepositoryState::Revert | RepositoryState::RevertSequence => Operation::Revert,
        RepositoryState::CherryPick | RepositoryState::CherryPickSequence => Operation::CherryPick,
        RepositoryState::Rebase
        | RepositoryState::RebaseInteractive
        | RepositoryState::RebaseMerge => Operation::Rebase,
        _ => return None,
    };
    let index = repo.index().ok()?;
    let conflicts = conflicts_of(&index);
    let head = repo.head().ok()?.peel_to_commit().ok()?.id().to_string();
    Some(InProgress {
        operation,
        conflicts,
        head,
    })
}

fn conflicts_of(index: &git2::Index) -> Vec<Conflict> {
    let Ok(conflicts) = index.conflicts() else {
        return Vec::new();
    };
    let mut out: Vec<Conflict> = conflicts
        .flatten()
        .filter_map(|c| {
            let any = c
                .our
                .as_ref()
                .or(c.their.as_ref())
                .or(c.ancestor.as_ref())?;
            let id = |e: &Option<git2::IndexEntry>| e.as_ref().map(|e| e.id.to_string());
            Some(Conflict {
                path: String::from_utf8_lossy(&any.path).into_owned(),
                ancestor: id(&c.ancestor),
                ours: id(&c.our),
                theirs: id(&c.their),
            })
        })
        .collect();
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// Whether a line is one of the markers `git` writes into a conflicted file.
fn is_marker(line: &str) -> bool {
    ["<<<<<<< ", ">>>>>>> ", "||||||| "]
        .iter()
        .any(|m| line.starts_with(m))
        || line == "======="
        || line == "<<<<<<<"
        || line == ">>>>>>>"
}

/// Resolves the conflicted file `path`, still in the conflict `expected`, by writing the side
/// `take` names, or the working directory's file as the developer edited it, to the index, as
/// `git checkout --ours/--theirs` and `git add` do (ADR-0033).
pub(crate) fn resolve(
    repo: &Repository,
    path: &str,
    expected: &Conflict,
    take: Take,
) -> Result<()> {
    let mut index = repo.index()?;
    if conflicts_of(&index).iter().find(|c| c.path == path) != Some(expected) {
        bail!("{path} no longer conflicts as it was shown: reload to see it as it is");
    }
    let root = repo
        .workdir()
        .context("a bare repository has no working directory")?;
    let file = root.join(path);
    within(root, path, &|_| false)?;
    // A link is looked at, never followed: what it points to may lie outside the repository.
    let there = file.symlink_metadata().ok();
    match take {
        Take::Ours | Take::Theirs => {
            let conflict = index
                .conflicts()?
                .flatten()
                .find(|c| {
                    let any = c.our.as_ref().or(c.their.as_ref()).or(c.ancestor.as_ref());
                    any.is_some_and(|e| e.path == path.as_bytes())
                })
                .context("the conflict is gone from the index")?;
            let git2::IndexConflict {
                ancestor,
                our: ours,
                their: theirs,
            } = conflict;
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
                    checkout.force().disable_pathspec_match(true).path(path);
                    if let Err(e) = repo.checkout_index(Some(&mut index), Some(&mut checkout)) {
                        // The file could not be written: the conflict stays as it was.
                        let mut index = repo.index()?;
                        index.remove_path(Path::new(path))?;
                        for staged in [&ancestor, &ours, &theirs].into_iter().flatten() {
                            index.add(staged)?;
                        }
                        index.write()?;
                        return Err(e.into());
                    }
                }
                None => {
                    index.write()?;
                    if there.is_some_and(|m| !m.is_dir()) {
                        std::fs::remove_file(&file)?;
                    }
                }
            }
        }
        Take::AsEdited => {
            if there.as_ref().is_some_and(|m| m.is_file())
                && let Ok(text) = std::fs::read(&file)
                && String::from_utf8_lossy(&text).lines().any(is_marker)
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

/// Refuses a `path` of the working directory below a link or a file, other than one `its_own`
/// says the operation put there: writing it would follow the link, out of the repository
/// perhaps, or fail half done.
fn within(root: &Path, path: &str, its_own: &dyn Fn(&str) -> bool) -> Result<()> {
    let mut dir = root.to_path_buf();
    let parts: Vec<&str> = path.split('/').collect();
    for (i, part) in parts[..parts.len().saturating_sub(1)].iter().enumerate() {
        dir.push(part);
        if its_own(&parts[..=i].join("/")) {
            continue;
        }
        match dir.symlink_metadata() {
            Ok(m) if m.file_type().is_symlink() => bail!(
                "{} is a link where {path} needs a directory: move it out of the way first",
                dir.strip_prefix(root).unwrap_or(&dir).display()
            ),
            Ok(m) if !m.is_dir() => bail!(
                "{} is a file where {path} needs a directory: move it out of the way first",
                dir.strip_prefix(root).unwrap_or(&dir).display()
            ),
            Ok(_) => {}
            Err(_) => break,
        }
    }
    Ok(())
}

/// A file of the working directory the index does not hold as it is.
const WORKDIR_CHANGES: Status = Status::WT_NEW
    .union(Status::WT_MODIFIED)
    .union(Status::WT_DELETED)
    .union(Status::WT_TYPECHANGE)
    .union(Status::WT_RENAMED);

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
/// does: commits it, a rebase going on to its next step (ADR-0033).
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
            let mut rebase = repo.open_rebase(None)?;
            // A step resolved to nothing is dropped, as `git rebase --continue` drops it.
            match rebase.commit(None, &me, None) {
                Ok(_) => {}
                Err(e) if e.code() == ErrorCode::Applied => {}
                Err(e) => return Err(e.into()),
            }
            run_rebase(repo, &mut rebase, &me)
        }
        Operation::CherryPick => {
            let picked = repo.find_reference("CHERRY_PICK_HEAD")?.peel_to_commit()?;
            let author = picked.author().to_owned();
            commit_in_progress(repo, operation, &me, Some(&author)).map(Some)
        }
        _ => commit_in_progress(repo, operation, &me, None).map(Some),
    }
}

/// Aborts the stopped `operation`, back to where the developer was, as `git <operation> --abort`
/// does: the files it touched as the checked-out commit has them, its state gone; a change of the
/// developer's own to another file stays (ADR-0033).
pub(crate) fn abort(repo: &Repository, operation: Operation, head: &str) -> Result<()> {
    stopped_as_shown(repo, operation, head)?;
    left_to_git(repo, "abort")?;
    if operation == Operation::Rebase {
        return abort_rebase(repo);
    }
    let head = repo.head()?.peel_to_commit()?;
    let tree = head.tree()?;
    let index = repo.index()?;
    let mut touched: Vec<String> = repo
        .diff_tree_to_index(Some(&tree), Some(&index), None)?
        .deltas()
        .filter_map(|d| d.new_file().path().or(d.old_file().path()))
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    let conflicted: Vec<String> = conflicts_of(&index).into_iter().map(|c| c.path).collect();
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
    let changed: std::collections::HashMap<String, Status> = repo
        .statuses(Some(&mut options))?
        .iter()
        .filter_map(|e| e.path().map(|p| (p.to_string(), e.status())))
        .filter(|(_, s)| s.intersects(WORKDIR_CHANGES))
        .collect();
    // A file the operation brought in cleanly or removed, which the developer changed or made
    // since, stays, as `git <operation> --abort` refuses rather than lose it.
    let below = |dir: &str| {
        let dir = format!("{dir}/");
        changed.keys().any(|p| p.starts_with(&dir))
    };
    // A conflicted file stands to be lost, but not a directory the developer made in its place.
    for path in touched.iter().chain(&conflicted) {
        if (!conflicted.contains(path) && changed.contains_key(path)) || below(path) {
            bail!(
                "{path} was changed after the {} stopped: commit, move, or undo that change \
                 before aborting",
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
        let its_own = |dir: &str| {
            let own = index.get_path(Path::new(dir), 0).is_some()
                && !changed.contains_key(dir)
                && root.join(dir).symlink_metadata().is_ok_and(|m| !m.is_dir());
            if own {
                own_parents.borrow_mut().push(dir.to_string());
            }
            own
        };
        for path in &touched {
            within(root, path, &its_own)?;
        }
        let mut own_parents = own_parents.into_inner();
        own_parents.sort();
        own_parents.dedup();
        // The index as it is, put back should the checkout fail, the operation still stopped.
        let index_file = repo.path().join("index");
        let kept = std::fs::read(&index_file)?;
        // Each path as the checked-out commit has it, in the index, taken literally.
        let mut index = repo.index()?;
        for path in &touched {
            let at = Path::new(path);
            index.conflict_remove(at).or_else(not_found)?;
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
            checkout.path(path.as_str());
        }
        // Each moved aside, into the repository's own directory, and back should the abort fail.
        let aside = |i: usize| repo.path().join(format!("ocelli-aside-{i}"));
        let mut moved = 0;
        let mut done = Ok(());
        for (i, dir) in own_parents.iter().enumerate() {
            done = std::fs::rename(root.join(dir), aside(i)).map_err(anyhow::Error::from);
            if done.is_err() {
                break;
            }
            moved += 1;
        }
        if done.is_ok() {
            done = repo
                .checkout_head(Some(&mut checkout))
                .map_err(anyhow::Error::from);
        }
        if done.is_ok() {
            for i in 0..moved {
                std::fs::remove_file(aside(i))?;
            }
        }
        if let Err(e) = done {
            for (i, dir) in own_parents.iter().enumerate().take(moved) {
                let _ = std::fs::rename(aside(i), root.join(dir));
            }
            std::fs::write(&index_file, kept)?;
            bail!(
                "the {} could not be aborted, its conflicts kept: {e}",
                operation.name()
            );
        }
    }
    repo.cleanup_state()?;
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

/// The file in libgit2's rebase directory that marks its current step as never applied.
const UNAPPLIED: &str = "ocelli-unapplied";

/// The directory a rebase in progress keeps its state in, `git`'s or libgit2's.
fn rebase_dir(repo: &Repository) -> Option<std::path::PathBuf> {
    ["rebase-merge", "rebase-apply"]
        .iter()
        .map(|d| repo.path().join(d))
        .find(|d| d.is_dir())
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
fn abort_rebase(repo: &Repository) -> Result<()> {
    let dir = rebase_dir(repo).context("the rebase keeps no state to abort from")?;
    let read = |name: &str| -> Result<String> {
        Ok(std::fs::read_to_string(dir.join(name))
            .with_context(|| format!("the rebase's {name} cannot be read"))?
            .trim()
            .to_string())
    };
    let before = repo.find_commit(Oid::from_str(&read("orig-head")?)?)?;
    let branch = read("head-name")?;
    repo.reset(before.as_object(), ResetType::Hard, None)?;
    if branch.starts_with("refs/heads/") {
        repo.reference(&branch, before.id(), true, "rebase: abort")?;
        repo.set_head(&branch)?;
    } else {
        repo.set_head_detached(before.id())?;
    }
    // A hard reset clears libgit2's own record of the rebase already.
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    Ok(())
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

/// libgit2's message where a file is in an operation's way, which says only "no error", made to
/// say what it is.
fn in_the_way(e: git2::Error) -> git2::Error {
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

/// A message `MERGE_MSG` holds, without the list of conflicts libgit2 appends to it.
fn without_conflict_list(prepared: &str) -> String {
    let mut kept = Vec::new();
    let mut listing = false;
    for line in prepared.lines() {
        if line
            .trim_start_matches('#')
            .trim()
            .eq_ignore_ascii_case("conflicts:")
            && line.starts_with('#')
        {
            listing = true;
            continue;
        }
        if listing && line.starts_with('#') {
            continue;
        }
        listing = false;
        kept.push(line);
    }
    kept.join("\n")
}

#[cfg(test)]
mod tests {
    use super::wildmatch;

    #[test]
    fn a_pattern_matches_a_branch_as_git_wildmatch_does() {
        let cases = [
            ("fe*", "feat", true),
            ("fe*", "fe/x", false),
            ("fe**", "fe/x", false),
            ("**/x", "a/b/x", true),
            ("**/x", "x", true),
            ("x/**", "x/a/b", true),
            ("rel-[0-9]", "rel-1", true),
            ("rel-[!0-9]", "rel-1", false),
            ("a[b]", "ab", true),
            ("a[]]", "a]", true),
            ("a\\*", "a*", true),
            ("a\\*", "ab", false),
            ("ma?n", "main", true),
            ("ma?n", "ma/n", false),
            ("main", "master", false),
            ("rel-[[:digit:]]", "rel-1", true),
            ("rel-[[:alpha:]]", "rel-1", false),
            ("a[a-\\z]", "ab", true),
            ("a[b-a]", "ab", true),
        ];
        for (pattern, name, matched) in cases {
            assert_eq!(
                wildmatch(pattern.as_bytes(), name.as_bytes()),
                matched,
                "{pattern} {name}"
            );
        }
    }
}
