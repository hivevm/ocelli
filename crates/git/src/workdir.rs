//! Every libgit2 call that writes the working directory, each behind the one guard
//! (ADR-0033, ADR-0039). libgit2 follows a link it meets where it expects a directory, and where
//! the index does not hold a link as one it writes a file through it, out of the repository
//! perhaps; it opens a conflicted path to write a side of the conflict there, and writes a side of
//! a conflict between a file and a directory, or between two renames, under the path with `~` and
//! a label added, which it opens through a link that points nowhere; a name the file system cannot
//! hold stops it half done; and it does not fsck a tree a hostile remote delivered. So before each
//! call, every path the call can change is looked at, as the difference between what it writes and
//! what is there, and every tree it reads along such a path: a link at such a path or a directory
//! above it that the checked-out commit and the index do not both hold as a link this write
//! replaces, a name this system cannot give a file, a tree naming one entry twice or two entries
//! one name where case is folded, a conflict between a file and a directory, or a link at a
//! conflicted path or beside it under its name with `~` refuses the write before libgit2 is
//! called. A link only staged is never trusted. `clippy.toml` disallows these calls everywhere but
//! here. A write that only ever checks out a result index libgit2 built itself (a merge, a stash's
//! apply, a rebase step) cannot meet a malformed tree there, so reading its trees is a safeguard
//! against a future libgit2, not a path a repository can reach today.

use crate::checkout::named;
use crate::guard::exact_path;
use anyhow::{Result, bail};
use git2::build::CheckoutBuilder;
use git2::{
    AnnotatedCommit, Commit, Diff, DiffOptions, Index, ObjectType, Oid, Repository, ResetType,
    Signature, StashApplyOptions, StashFlags, Tree,
};
use std::collections::{BTreeSet, HashMap};

/// The mode of a link in a tree or the index.
const LINK: u32 = 0o120000;

/// The longest name of a file a supported file system takes, in bytes, and on Windows in UTF-16
/// units.
const LONGEST_NAME: usize = 255;

/// What a write can change in the working directory, in bytes: the paths it writes or removes,
/// and the conflicts it writes, with a conflict between a file and a directory of one name.
#[derive(Default)]
struct Reach {
    paths: BTreeSet<Vec<u8>>,
    conflicts: BTreeSet<Vec<u8>>,
    directory_file: Option<Vec<u8>>,
}

impl Reach {
    /// Every path `diff` names, on either side.
    fn diff(&mut self, diff: &Diff) {
        for delta in diff.deltas() {
            for file in [delta.old_file(), delta.new_file()] {
                if let Some(path) = file.path_bytes() {
                    self.paths.insert(path.to_vec());
                }
            }
        }
    }

    /// What writing the index `result` over the checked-out commit `head` changes: its
    /// difference from `head`, and its conflicts.
    fn result(&mut self, repo: &Repository, head: Option<&Tree>, result: &Index) -> Result<()> {
        self.diff(&repo.diff_tree_to_index(head, Some(result), Some(&mut diff_options()))?);
        for conflict in result.conflicts()? {
            let conflict = conflict?;
            for entry in [conflict.ancestor, conflict.our, conflict.their]
                .into_iter()
                .flatten()
            {
                self.paths.insert(entry.path.clone());
                self.conflicts.insert(entry.path);
            }
        }
        // libgit2 writes a side of a conflict between a file and a directory of one name, which
        // it finds as a conflicted path with an entry below it, under another name.
        for path in &self.conflicts {
            let mut below = path.clone();
            below.push(b'/');
            let under = result.find_prefix(below).is_ok();
            let over = above(path).find(|a| {
                exact_path(a).is_ok_and(|at| {
                    (0..=3).any(|stage| crate::git::index_entry(result, &at, stage).is_some())
                })
            });
            if under {
                self.directory_file = Some(path.clone());
            } else if let Some(over) = over {
                self.directory_file = Some(over.to_vec());
            }
            if self.directory_file.is_some() {
                break;
            }
        }
        Ok(())
    }
}

/// How a difference is read for the guard: a change of kind as one, links as links.
fn diff_options() -> DiffOptions {
    let mut options = DiffOptions::new();
    options.include_typechange(true);
    options
}

/// The trees a write that merges `commit` reads: its own and its parents'.
fn trees_of<'r>(commit: &Commit<'r>) -> Result<Vec<Tree<'r>>> {
    let mut trees = vec![commit.tree()?];
    for parent in commit.parents() {
        trees.push(parent.tree()?);
    }
    Ok(trees)
}

/// The tree of the checked-out commit, where there is one.
fn head_tree(repo: &Repository) -> Option<Tree<'_>> {
    repo.head().ok().and_then(|h| h.peel_to_tree().ok())
}

/// What checking `tree` out over the index and the working directory, everything in them put
/// back, changes. The index on its own too: an entry staged where no file stands differs from
/// `tree` there, while the working directory, missing it as `tree` does, would hide it.
fn over_the_working_directory(repo: &Repository, tree: &Tree) -> Result<Reach> {
    let mut reach = Reach::default();
    let diff = repo.diff_tree_to_workdir_with_index(Some(tree), Some(&mut diff_options()))?;
    reach.diff(&diff);
    let staged = repo.diff_tree_to_index(Some(tree), None, Some(&mut diff_options()))?;
    reach.diff(&staged);
    Ok(reach)
}

/// Refuses a write that reaches `reach`, reading `trees` besides the checked-out commit's, where
/// one of those trees is malformed along a path it changes, where the working directory holds a
/// link at such a path or a directory above one that the checked-out commit and the index do not
/// both hold as a link the write replaces, where it writes a name this system cannot hold or one
/// path both as a file and as a directory, or where it writes a conflict libgit2 would write
/// through a link or under another name.
fn guard(repo: &Repository, reach: &Reach, trees: &[&Tree]) -> Result<()> {
    let Some(root) = repo.workdir() else {
        return Ok(());
    };
    for path in &reach.paths {
        can_name(path)?;
    }
    let head = head_tree(repo);
    sound(repo, head.iter().chain(trees.iter().copied()), &reach.paths)?;
    let index = repo.index()?;
    // Only a link the checked-out commit and the index both hold, and that this write replaces,
    // is one libgit2 removes before it writes: a link staged alone, or one the write leaves,
    // it goes through.
    let replaced = |path: &[u8]| {
        reach.paths.contains(path)
            && exact_path(path).is_ok_and(|at| {
                head.as_ref()
                    .and_then(|h| h.get_path(&at).ok())
                    .is_some_and(|e| e.filemode() as u32 == LINK)
                    && (0..=3).any(|stage| {
                        crate::git::index_entry(&index, &at, stage).is_some_and(|e| e.mode == LINK)
                    })
            })
    };
    if let Some(path) = &reach.directory_file {
        bail!(
            "a file and a directory named {} conflict, which libgit2 would write under another \
             name: resolve this with git",
            named(&exact_path(path)?)
        );
    }

    // What stands at each path and directory, looked at once.
    let mut found: HashMap<Vec<u8>, Found> = HashMap::new();
    let mut at = |part: &[u8]| -> Result<Found> {
        if let Some(there) = found.get(part) {
            return Ok(*there);
        }
        let there = Found::at(&root.join(exact_path(part)?));
        found.insert(part.to_vec(), there);
        Ok(there)
    };
    for path in &reach.paths {
        for part in above(path)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .chain([&path[..]])
        {
            let last = part.rsplit(|&c| c == b'/').next().unwrap_or_default();
            if last.is_empty() || last == b"." || last == b".." {
                // Not a name libgit2 writes: it refuses the path itself.
                break;
            }
            match at(part)? {
                Found::Link if !replaced(part) => bail!(
                    "{} is a link, which writing {} would go through: move it first",
                    named(&exact_path(part)?),
                    named(&exact_path(path)?)
                ),
                Found::Directory => {}
                // Nothing there, a link the write replaces, or a file: libgit2 writes the path
                // without going through anything.
                Found::Nothing | Found::Link | Found::Other => break,
            }
        }
    }

    for path in &reach.conflicts {
        let at = exact_path(path)?;
        if matches!(Found::at(&root.join(&at)), Found::Link) {
            bail!(
                "{} is a link, which libgit2 would write a side of its conflict through: \
                 move it first",
                named(&at)
            );
        }
        // A side written under the name with `~` and a label, and `_` and a number where that
        // is taken: libgit2 opens it through a link that points nowhere.
        let (Some(name), Some(dir)) = (
            at.file_name(),
            root.join(&at).parent().map(|d| d.to_path_buf()),
        ) else {
            continue;
        };
        let mut prefix = name.as_encoded_bytes().to_vec();
        prefix.push(b'~');
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let sibling = entry.file_name();
            if sibling.as_encoded_bytes().starts_with(&prefix)
                && entry.file_type().is_ok_and(|t| t.is_symlink())
            {
                bail!(
                    "{} is a link where libgit2 would write a side of the conflict on {}: move \
                     it first",
                    named(&at.with_file_name(&sibling)),
                    named(&at)
                );
            }
        }
    }
    Ok(())
}

/// A tree still to look at: its id, the directory it stands for, and the paths below it.
type Below<'p> = (Oid, Vec<u8>, Vec<&'p [u8]>);

/// Refuses where a tree of `trees`, or one below it along `paths`, names an entry twice, as
/// `git fsck` reports, or names two entries one name where case is not told apart: libgit2 writes
/// both, one through the other where one is a link. Read from the entries as stored, in bytes.
fn sound<'t>(
    repo: &Repository,
    trees: impl Iterator<Item = &'t Tree<'t>>,
    paths: &BTreeSet<Vec<u8>>,
) -> Result<()> {
    let folded = repo
        .config()
        .and_then(|c| c.get_bool("core.ignorecase"))
        .unwrap_or(false)
        || cfg!(any(windows, target_os = "macos"));
    let fold = |name: &[u8]| -> Vec<u8> {
        if folded {
            String::from_utf8_lossy(name).to_lowercase().into_bytes()
        } else {
            name.to_vec()
        }
    };
    let all: Vec<&[u8]> = paths.iter().map(|p| &p[..]).collect();
    let mut seen = std::collections::HashSet::new();
    let mut stack: Vec<Below> = trees.map(|t| (t.id(), Vec::new(), all.clone())).collect();
    while let Some((id, dir, below)) = stack.pop() {
        if !seen.insert((id, dir.clone())) {
            continue;
        }
        let tree = repo.find_tree(id)?;
        let mut names: Vec<(Vec<u8>, &[u8])> = Vec::new();
        let entries: Vec<_> = tree.iter().collect();
        for entry in &entries {
            names.push((fold(entry.name_bytes()), entry.name_bytes()));
        }
        names.sort();
        if let Some(pair) = names.windows(2).find(|w| w[0].0 == w[1].0) {
            let mut at = dir.clone();
            if !at.is_empty() {
                at.push(b'/');
            }
            at.extend_from_slice(pair[0].1);
            if pair[0].1 == pair[1].1 {
                bail!(
                    "a tree names {} twice, which git fsck reports: it is not written",
                    named(&exact_path(&at)?)
                );
            }
            bail!(
                "a tree names {} and {}, one name where case is not told apart: it is not written",
                named(&exact_path(&at)?),
                String::from_utf8_lossy(pair[1].1)
            );
        }
        // The paths below each directory of this tree, by the directory's name.
        let mut by_name: HashMap<&[u8], Vec<&[u8]>> = HashMap::new();
        for path in &below {
            if let Some(slash) = path.iter().position(|&c| c == b'/') {
                by_name
                    .entry(&path[..slash])
                    .or_default()
                    .push(&path[slash + 1..]);
            }
        }
        for entry in &entries {
            if entry.kind() != Some(ObjectType::Tree) {
                continue;
            }
            let name = entry.name_bytes();
            if let Some(rest) = by_name.get(name) {
                let mut sub = dir.clone();
                if !sub.is_empty() {
                    sub.push(b'/');
                }
                sub.extend_from_slice(name);
                stack.push((entry.id(), sub, rest.clone()));
            }
        }
    }
    Ok(())
}

/// The directories above `path`, the nearest first.
fn above(path: &[u8]) -> impl Iterator<Item = &[u8]> {
    path.iter()
        .enumerate()
        .rev()
        .filter(|(_, c)| **c == b'/')
        .map(move |(i, _)| &path[..i])
}

/// What the working directory holds at a path, a link not followed.
#[derive(Clone, Copy)]
enum Found {
    Nothing,
    Directory,
    Link,
    Other,
}

impl Found {
    fn at(path: &std::path::Path) -> Found {
        match path.symlink_metadata() {
            Err(_) => Found::Nothing,
            Ok(m) if m.file_type().is_symlink() => Found::Link,
            Ok(m) if m.is_dir() => Found::Directory,
            Ok(_) => Found::Other,
        }
    }
}

/// Refuses a path this system cannot give a file: one that is not UTF-8 on Windows and macOS, or
/// one with a name longer than file systems take.
fn can_name(path: &[u8]) -> Result<()> {
    let at = exact_path(path)?;
    for part in path.split(|&c| c == b'/') {
        let length = if cfg!(windows) {
            String::from_utf8_lossy(part).encode_utf16().count()
        } else {
            part.len()
        };
        if length > LONGEST_NAME {
            bail!(
                "{} has a name longer than a file system gives a file",
                named(&at)
            );
        }
    }
    Ok(())
}

/// libgit2's checkout of `target`'s tree over the checked-out one, behind the guard.
pub(crate) fn checkout_tree(
    repo: &Repository,
    target: &Commit,
    options: &mut CheckoutBuilder,
) -> Result<Result<(), git2::Error>> {
    let mut reach = Reach::default();
    let goal = target.tree()?;
    let diff = repo.diff_tree_to_tree(
        head_tree(repo).as_ref(),
        Some(&goal),
        Some(&mut diff_options()),
    )?;
    reach.diff(&diff);
    guard(repo, &reach, &[&goal])?;
    #[expect(clippy::disallowed_methods, reason = "behind the guard")]
    Ok(repo.checkout_tree(target.as_object(), Some(options)))
}

/// libgit2's checkout of the index over the working directory, as the checked-out commit has
/// what it does not hold, behind the guard.
pub(crate) fn checkout_head(
    repo: &Repository,
    options: &mut CheckoutBuilder,
) -> Result<Result<(), git2::Error>> {
    let mut reach = match head_tree(repo) {
        Some(tree) => over_the_working_directory(repo, &tree)?,
        None => Reach::default(),
    };
    let diff = repo.diff_index_to_workdir(None, Some(&mut diff_options()))?;
    reach.diff(&diff);
    guard(repo, &reach, &[])?;
    #[expect(clippy::disallowed_methods, reason = "behind the guard")]
    Ok(repo.checkout_head(Some(options)))
}

/// libgit2's checkout of `index`, behind the guard.
pub(crate) fn checkout_index(
    repo: &Repository,
    index: &mut Index,
    options: &mut CheckoutBuilder,
) -> Result<Result<(), git2::Error>> {
    let mut reach = Reach::default();
    let diff = repo.diff_index_to_workdir(Some(index), Some(&mut diff_options()))?;
    reach.diff(&diff);
    guard(repo, &reach, &[])?;
    #[expect(clippy::disallowed_methods, reason = "behind the guard")]
    Ok(repo.checkout_index(Some(index), Some(options)))
}

/// libgit2's reset to `target`, behind the guard where it writes the working directory.
pub(crate) fn reset(repo: &Repository, target: &Commit, kind: ResetType) -> Result<()> {
    if kind == ResetType::Hard {
        let goal = target.tree()?;
        guard(repo, &over_the_working_directory(repo, &goal)?, &[&goal])?;
    }
    #[expect(clippy::disallowed_methods, reason = "behind the guard")]
    repo.reset(target.as_object(), kind, None)?;
    Ok(())
}

/// libgit2's merge of `theirs`, whose result is `result`, behind the guard.
pub(crate) fn merge(
    repo: &Repository,
    theirs: &AnnotatedCommit,
    result: &Index,
) -> Result<Result<(), git2::Error>> {
    let mut reach = Reach::default();
    reach.result(repo, head_tree(repo).as_ref(), result)?;
    let mut trees = trees_of(&repo.find_commit(theirs.id())?)?;
    let head = repo.head()?.peel_to_commit()?.id();
    if let Ok(base) = repo.merge_base(head, theirs.id()) {
        trees.push(repo.find_commit(base)?.tree()?);
    }
    guard(repo, &reach, &trees.iter().collect::<Vec<_>>())?;
    #[expect(clippy::disallowed_methods, reason = "behind the guard")]
    Ok(repo.merge(&[theirs], None, None))
}

/// libgit2's cherry-pick of `picked`, or its revert where `revert` says so, whose result is
/// `result`, behind the guard.
pub(crate) fn pick(
    repo: &Repository,
    picked: &Commit,
    revert: bool,
    result: &Index,
) -> Result<Result<(), git2::Error>> {
    let mut reach = Reach::default();
    reach.result(repo, head_tree(repo).as_ref(), result)?;
    let trees = trees_of(picked)?;
    guard(repo, &reach, &trees.iter().collect::<Vec<_>>())?;
    #[expect(clippy::disallowed_methods, reason = "behind the guard")]
    Ok(if revert {
        repo.revert(picked, None)
    } else {
        repo.cherrypick(picked, None)
    })
}

/// libgit2's stash of the changes, which puts the working directory back as the checked-out
/// commit has it, behind the guard.
pub(crate) fn stash_save(
    repo: &mut Repository,
    stasher: &Signature,
    message: Option<&str>,
    flags: StashFlags,
) -> Result<Result<Oid, git2::Error>> {
    if let Some(tree) = head_tree(repo) {
        guard(repo, &over_the_working_directory(repo, &tree)?, &[])?;
    }
    #[expect(clippy::disallowed_methods, reason = "behind the guard")]
    Ok(repo.stash_save2(stasher, message, Some(flags)))
}

/// libgit2's apply of the stash `stash` at `index`, behind the guard: its changes merged into
/// the checked-out commit, and the untracked files it took along.
pub(crate) fn stash_apply(
    repo: &mut Repository,
    index: usize,
    stash: Oid,
    options: &mut StashApplyOptions,
) -> Result<()> {
    let mut reach = Reach::default();
    {
        let stash = repo.find_commit(stash)?;
        let head = head_tree(repo);
        let base = stash.parent(0)?.tree()?;
        if let Some(head) = &head {
            let result = repo.merge_trees(&base, head, &stash.tree()?, None)?;
            reach.result(repo, Some(head), &result)?;
        }
        if let Ok(untracked) = stash.parent(2) {
            let diff = repo.diff_tree_to_tree(None, Some(&untracked.tree()?), None)?;
            reach.diff(&diff);
        }
        let trees = trees_of(&stash)?;
        guard(repo, &reach, &trees.iter().collect::<Vec<_>>())?;
    }
    #[expect(clippy::disallowed_methods, reason = "behind the guard")]
    repo.stash_apply(index, Some(options))?;
    Ok(())
}

/// A rebase of libgit2's on disk, each step that writes the working directory behind the guard.
/// Undone by `conflicts::abort_rebase`, a hard reset behind the guard.
pub(crate) struct Rebase<'r> {
    repo: &'r Repository,
    rebase: git2::Rebase<'r>,
}

impl<'r> Rebase<'r> {
    /// Starts rebasing `ours` onto `onto`, which checks `onto` out.
    pub(crate) fn start(
        repo: &'r Repository,
        ours: &AnnotatedCommit,
        onto: &AnnotatedCommit,
    ) -> Result<Result<Self, git2::Error>> {
        let mut reach = Reach::default();
        let goal = repo.find_commit(onto.id())?.tree()?;
        let diff = repo.diff_tree_to_tree(
            head_tree(repo).as_ref(),
            Some(&goal),
            Some(&mut diff_options()),
        )?;
        reach.diff(&diff);
        guard(repo, &reach, &[&goal])?;
        #[expect(clippy::disallowed_methods, reason = "behind the guard")]
        let rebase = repo.rebase(Some(ours), Some(onto), None, None);
        Ok(rebase.map(|rebase| Rebase { repo, rebase }))
    }

    /// The rebase in progress.
    pub(crate) fn open(repo: &'r Repository) -> Result<Self> {
        #[expect(
            clippy::disallowed_methods,
            reason = "opening writes nothing; each step it writes goes through the guard"
        )]
        let rebase = repo.open_rebase(None)?;
        Ok(Rebase { repo, rebase })
    }

    /// Applies the next step, behind the guard over what merging its commit into the checked-out
    /// one writes; `None` where none is left.
    pub(crate) fn next(&mut self) -> Result<Option<Result<(), git2::Error>>> {
        let next = self.rebase.operation_current().map_or(0, |c| c + 1);
        if let Some(step) = self.rebase.nth(next) {
            let commit = self.repo.find_commit(step.id())?;
            let head = self.repo.head()?.peel_to_commit()?;
            let mut reach = Reach::default();
            match self.repo.cherrypick_commit(&commit, &head, 0, None) {
                Ok(result) => reach.result(self.repo, Some(&head.tree()?), &result)?,
                // A commit without one parent: everything its tree differs in, a conflict aside.
                Err(_) => {
                    let diff = self.repo.diff_tree_to_tree(
                        Some(&head.tree()?),
                        Some(&commit.tree()?),
                        Some(&mut diff_options()),
                    )?;
                    reach.diff(&diff);
                }
            }
            let trees = trees_of(&commit)?;
            guard(self.repo, &reach, &trees.iter().collect::<Vec<_>>())?;
        }
        Ok(self.rebase.next().map(|step| step.map(|_| ())))
    }

    /// Commits the step applied.
    pub(crate) fn commit(&mut self, me: &Signature) -> Result<Oid, git2::Error> {
        self.rebase.commit(None, me, None)
    }

    /// Ends the rebase, every step committed.
    pub(crate) fn finish(&mut self, me: &Signature) -> Result<(), git2::Error> {
        self.rebase.finish(Some(me))
    }
}
