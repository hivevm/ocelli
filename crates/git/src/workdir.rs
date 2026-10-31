//! Every libgit2 call that writes the working directory, each behind the one guard
//! (ADR-0031, ADR-0032). libgit2 follows a link it meets where it expects a directory, and where
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
//! here. A write that checks out a result index libgit2 built itself, a merge, a stash's apply or
//! a rebase step, reads the trees it merged too: libgit2 fills the result from the three trees
//! without an fsck of its own (`index_from_diff_list`), so a malformed side could in principle
//! leave a stage-0 link `a` beside `a/x` in the result that the result's own conflict check does
//! not see. Reaching that state needs a working directory holding a tree git cannot materialise,
//! so reading the sides is a safeguard, not a path a repository reaches today.

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

/// The label of the checked-out side of a conflict, as `git` writes it, every operation included.
const OURS: &str = "HEAD";

/// Whether a name holds what Windows does not take in a file's name: `<`, `>`, `:`, `"`, `|`,
/// `?`, `*`, or a control character, or ends in a dot or a space.
fn windows_refuses(name: &[u8]) -> bool {
    name.iter().any(|&b| b < 0x20 || b"<>:\"|?*".contains(&b))
        || name.last().is_some_and(|&b| b == b'.' || b == b' ')
}

/// Whether `label` is one libgit2 may write into a path as `<path>~<label>` without spelling a
/// path of its own, on every system: libgit2 appends the label to the conflicted path and creates
/// every directory the result names, so a label holding a separator or `..`, a control character,
/// a NUL, or a character Windows does not take in a name, or one longer than a name a file system
/// takes, could write a side anywhere writable or stop the write half done (ADR-0032).
fn safe_label(label: &str) -> Result<()> {
    let bad = label.is_empty()
        || label.len() > LONGEST_NAME
        || label.contains(['/', '\\'])
        || label.split(['/', '\\']).any(|part| part == "..")
        || label.bytes().any(|b| b == 0x7f)
        || windows_refuses(label.as_bytes());
    if bad {
        bail!("'{label}' is not a label a conflict's side can be written under");
    }
    Ok(())
}

/// The labels of a conflict's sides, as `git` writes them: `theirs`, the side applied, and
/// `ancestor`, the base `git` writes in its `diff3` and `zdiff3` styles; the checked-out side is
/// always `HEAD` (ADR-0032).
struct Labels {
    theirs: String,
    ancestor: String,
}

/// A checkout whose sides carry `HEAD`, `labels.theirs`, and `labels.ancestor`. Refuses a label
/// that is not path-safe before libgit2 sees it.
fn labelled(labels: &Labels) -> Result<CheckoutBuilder<'static>> {
    safe_label(OURS)?;
    safe_label(&labels.theirs)?;
    safe_label(&labels.ancestor)?;
    let mut checkout = CheckoutBuilder::new();
    #[expect(
        clippy::disallowed_methods,
        reason = "every label passes safe_label, which refuses a NUL"
    )]
    checkout
        .our_label(OURS)
        .their_label(&labels.theirs)
        .ancestor_label(&labels.ancestor);
    Ok(checkout)
}

/// A rebase's options: the step's checkout carries its labels. The labels are owned C strings,
/// so the options may be kept past the call that reads them; libgit2 copies the pointer into them
/// and reads it again at each step.
fn rebase_options(labels: &Labels) -> Result<git2::RebaseOptions<'static>> {
    let mut options = git2::RebaseOptions::new();
    options.checkout_options(labelled(labels)?);
    Ok(options)
}

/// The labels `git cherry-pick` writes for `picked`, and `git rebase` for a step: the commit, and
/// `parent of` it as the base, its subject left out (ADR-0032).
fn picked_labels(repo: &Repository, picked: Oid) -> Labels {
    let short = abbrev(repo, picked);
    Labels {
        ancestor: format!("parent of {short}"),
        theirs: short,
    }
}

/// The labels `git revert` writes for `reverted`: `parent of` the commit, and the commit as the
/// base, its subject left out (ADR-0032).
fn revert_labels(repo: &Repository, reverted: Oid) -> Labels {
    let short = abbrev(repo, reverted);
    Labels {
        theirs: format!("parent of {short}"),
        ancestor: short,
    }
}

/// The empty tree, found by its constant id where the object database holds it, which it does
/// once anything empty has been written, or written there once otherwise.
fn empty_tree(repo: &Repository) -> Result<Tree<'_>> {
    if let Ok(id) = Oid::from_str("4b825dc642cb6eb9a060e54bf8d69288fbee4904")
        && let Ok(tree) = repo.find_tree(id)
    {
        return Ok(tree);
    }
    let id = repo.treebuilder(None)?.write()?;
    Ok(repo.find_tree(id)?)
}

/// `name` with its case folded character by character, as a file system that does not tell case
/// apart compares names: each character's own lower case, with no rule that looks at the
/// characters around it. A macOS file system also compares names in one Unicode normal form;
/// Rust's standard library has no normalisation, and adding a crate for it is a decision of its
/// own, so names that differ only in normal form are not folded together here.
fn fold_case(name: &[u8]) -> Vec<u8> {
    String::from_utf8_lossy(name)
        .chars()
        .flat_map(char::to_lowercase)
        .collect::<String>()
        .into_bytes()
}

/// The abbreviated id of `oid`: libgit2's, `core.abbrev` characters where it is a number, seven
/// otherwise, longer only where that prefix is not unique; `git` lengthens it with the number of
/// objects instead, the departure ADR-0032 lists. A hex prefix, always path-safe.
fn abbrev(repo: &Repository, oid: Oid) -> String {
    repo.find_object(oid, None)
        .ok()
        .and_then(|o| o.short_id().ok())
        .and_then(|b| b.as_str().map(str::to_string))
        .unwrap_or_else(|| oid.to_string()[..7.min(oid.to_string().len())].to_string())
}

/// The labels of a merge of `theirs`: the branch the developer named, where it is a local or a
/// remote branch at `theirs`, with `/` flattened to `_` as `git` flattens it for the path it
/// writes a side to, and where that is path-safe; the abbreviated commit otherwise. `git` keeps
/// the `/` in the marker itself, the departure ADR-0032 lists: libgit2 takes one label for both.
/// The base is the merge base's abbreviated commit, `merged common ancestors` where there are
/// several, and `empty tree` where there is none, as `git` writes it.
fn merge_labels(repo: &Repository, theirs: Oid, name: Option<&str>, bases: &[Oid]) -> Labels {
    let at_theirs = |n: &str| {
        [git2::BranchType::Local, git2::BranchType::Remote]
            .into_iter()
            .any(|kind| {
                repo.find_branch(n, kind)
                    .ok()
                    .and_then(|b| b.get().peel_to_commit().ok())
                    .is_some_and(|c| c.id() == theirs)
            })
    };
    let theirs = name
        .filter(|n| at_theirs(n))
        .map(|n| n.replace('/', "_"))
        .filter(|n| safe_label(n).is_ok())
        .unwrap_or_else(|| abbrev(repo, theirs));
    let ancestor = match bases {
        [] => "empty tree".to_string(),
        [base] => abbrev(repo, *base),
        _ => "merged common ancestors".to_string(),
    };
    Labels { theirs, ancestor }
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
/// through a link or under a name with `~` and one of `labels`.
fn guard(repo: &Repository, reach: &Reach, trees: &[&Tree], labels: &[&str]) -> Result<()> {
    let Some(root) = repo.workdir() else {
        return Ok(());
    };
    for path in &reach.paths {
        can_name(path)?;
    }
    // Where the file system, or `core.ignorecase`, does not tell two names apart, a sibling
    // whose name folds to the same is the one libgit2 would meet.
    let folded = repo
        .config()
        .and_then(|c| c.get_bool("core.ignorecase"))
        .unwrap_or(false)
        || cfg!(any(windows, target_os = "macos"));
    let fold = |name: &[u8]| -> Vec<u8> {
        if folded {
            fold_case(name)
        } else {
            name.to_vec()
        }
    };
    let head = head_tree(repo);
    sound(
        repo,
        head.iter().chain(trees.iter().copied()),
        &reach.paths,
        folded,
    )?;
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
        // Where libgit2 cannot write a side at the conflicted path it writes it at the path with
        // `~` and one of this write's labels, and `_` and a number where that is taken. Those
        // names, which it creates every directory of, must be ones the file system holds, or it
        // stops half done.
        for label in labels {
            let mut suffixed = path.clone();
            suffixed.extend_from_slice(format!("~{label}").as_bytes());
            can_name(&suffixed)?;
        }
        let at = exact_path(path)?;
        if matches!(Found::at(&root.join(&at)), Found::Link) {
            bail!(
                "{} is a link, which libgit2 would write a side of its conflict through: \
                 move it first",
                named(&at)
            );
        }
        // A side written under the name with `~` and a label, and `_` and a number where that is
        // taken: libgit2 opens it through a link that points nowhere, out of the repository.
        let (Some(name), Some(dir)) = (
            at.file_name(),
            root.join(&at).parent().map(|d| d.to_path_buf()),
        ) else {
            continue;
        };
        let mut prefix = name.as_encoded_bytes().to_vec();
        prefix.push(b'~');
        let prefix = fold(&prefix);
        // libgit2 writes a merged side through a lock file, `<path>.lock`, opened O_CREAT|O_EXCL:
        // a link is never followed, but anything already there, a link or a file, fails the open
        // with the merge half done, so a lock file in the way refuses the write first.
        let mut lock = name.as_encoded_bytes().to_vec();
        lock.extend_from_slice(b".lock");
        let lock = fold(&lock);
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let sibling = entry.file_name();
            let name_folded = fold(sibling.as_encoded_bytes());
            let link = entry.file_type().is_ok_and(|t| t.is_symlink());
            let suffixed = name_folded.starts_with(&prefix) && link;
            let locked = name_folded == lock;
            if suffixed || locked {
                bail!(
                    "{} is in the way of a side of the conflict on {}: move it first",
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
    folded: bool,
) -> Result<()> {
    let fold = |name: &[u8]| -> Vec<u8> {
        if folded {
            fold_case(name)
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
        if cfg!(windows) && windows_refuses(part) {
            bail!(
                "{} has a name Windows does not give a file: a character it refuses, or a dot or \
                 a space at its end",
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
    guard(repo, &reach, &[&goal], &[])?;
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
    guard(repo, &reach, &[], &[])?;
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
    guard(repo, &reach, &[], &[])?;
    #[expect(clippy::disallowed_methods, reason = "behind the guard")]
    Ok(repo.checkout_index(Some(index), Some(options)))
}

/// libgit2's reset to `target`, behind the guard where it writes the working directory.
pub(crate) fn reset(repo: &Repository, target: &Commit, kind: ResetType) -> Result<()> {
    if kind == ResetType::Hard {
        let goal = target.tree()?;
        guard(
            repo,
            &over_the_working_directory(repo, &goal)?,
            &[&goal],
            &[],
        )?;
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
    name: Option<&str>,
) -> Result<Result<(), git2::Error>> {
    let mut reach = Reach::default();
    reach.result(repo, head_tree(repo).as_ref(), result)?;
    let mut trees = trees_of(&repo.find_commit(theirs.id())?)?;
    let head = repo.head()?.peel_to_commit()?.id();
    // Every merge base, not one: a criss-cross merge reads each, and libgit2 builds its virtual
    // base by merging them, so a malformed one among them reaches the result. Only "no merge
    // base" is a state, not a fault; any other error is the repository's and is raised.
    let bases: Vec<Oid> = match repo.merge_bases(head, theirs.id()) {
        Ok(bases) => bases.iter().copied().collect(),
        Err(e) if e.code() == git2::ErrorCode::NotFound => Vec::new(),
        Err(e) => return Err(e.into()),
    };
    for base in &bases {
        trees.push(repo.find_commit(*base)?.tree()?);
    }
    let labels = merge_labels(repo, theirs.id(), name, &bases);
    guard(
        repo,
        &reach,
        &trees.iter().collect::<Vec<_>>(),
        &[OURS, &labels.theirs],
    )?;
    #[expect(clippy::disallowed_methods, reason = "behind the guard")]
    Ok(repo.merge(&[theirs], None, Some(&mut labelled(&labels)?)))
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
    let labels = if revert {
        revert_labels(repo, picked.id())
    } else {
        picked_labels(repo, picked.id())
    };
    guard(
        repo,
        &reach,
        &trees.iter().collect::<Vec<_>>(),
        &[OURS, &labels.theirs],
    )?;
    #[expect(clippy::disallowed_methods, reason = "behind the guard")]
    Ok(if revert {
        let mut options = git2::RevertOptions::new();
        options.checkout_builder(labelled(&labels)?);
        repo.revert(picked, Some(&mut options))
    } else {
        let mut options = git2::CherrypickOptions::new();
        options.checkout_builder(labelled(&labels)?);
        repo.cherrypick(picked, Some(&mut options))
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
        guard(repo, &over_the_working_directory(repo, &tree)?, &[], &[])?;
    }
    #[expect(clippy::disallowed_methods, reason = "behind the guard")]
    Ok(repo.stash_save2(stasher, message, Some(flags)))
}

/// libgit2's apply of the stash `stash` at `index`, behind the guard: its changes merged into
/// the checked-out commit, and the untracked files it took along.
pub(crate) fn stash_apply(repo: &mut Repository, index: usize, stash: Oid) -> Result<()> {
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
        // libgit2's stash keeps its own path-safe labels, which `git stash` writes too.
        guard(
            repo,
            &reach,
            &trees.iter().collect::<Vec<_>>(),
            &["Updated upstream", "Stashed changes"],
        )?;
    }
    // A second guard behind the check: an ignored file is never overwritten. The labels are
    // libgit2's own, which `git` writes; only the ignored-file strategy is set.
    let mut checkout = CheckoutBuilder::new();
    checkout.overwrite_ignored(false);
    let mut options = StashApplyOptions::new();
    options.checkout_options(checkout);
    #[expect(clippy::disallowed_methods, reason = "behind the guard")]
    repo.stash_apply(index, Some(&mut options))?;
    Ok(())
}

/// A rebase of libgit2's on disk, each step that writes the working directory behind the guard.
/// Undone by `conflicts::abort_rebase`, a hard reset behind the guard.
pub(crate) struct Rebase<'r> {
    repo: &'r Repository,
    /// Declared before `_options`, so dropped first: Rust drops fields in the order they are
    /// declared, and libgit2's rebase holds a pointer into the options until it is freed.
    rebase: git2::Rebase<'r>,
    /// The options the rebase was started or opened with, kept for as long as it lives: libgit2
    /// copies them and reads the labels' strings at every step, which git2 does not keep alive.
    _options: git2::RebaseOptions<'static>,
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
        guard(repo, &reach, &[&goal], &[])?;
        // The start writes no conflict, so its labels are never written; each step's are its own.
        let mut options = rebase_options(&picked_labels(repo, onto.id()))?;
        #[expect(clippy::disallowed_methods, reason = "behind the guard")]
        let rebase = repo.rebase(Some(ours), Some(onto), None, Some(&mut options));
        Ok(rebase.map(|rebase| Rebase {
            repo,
            rebase,
            _options: options,
        }))
    }

    /// The rebase in progress. Opening writes nothing, so the label it is opened with is never
    /// written; each step is opened again with its own before it is applied.
    pub(crate) fn open(repo: &'r Repository) -> Result<Self> {
        let mut options = rebase_options(&Labels {
            theirs: OURS.to_string(),
            ancestor: OURS.to_string(),
        })?;
        #[expect(
            clippy::disallowed_methods,
            reason = "opening writes nothing; each step it writes goes through the guard"
        )]
        let rebase = repo.open_rebase(Some(&mut options))?;
        Ok(Rebase {
            repo,
            rebase,
            _options: options,
        })
    }

    /// Applies the next step, behind the guard over what merging its commit into the checked-out
    /// one writes; `None` where none is left. libgit2 takes a step's labels from the options the
    /// rebase was opened with, one set for every step, so the rebase is opened again with this
    /// step's own labels, `HEAD`, its abbreviated commit, and `parent of` it as the base, as
    /// `git` writes them (ADR-0032).
    pub(crate) fn next(&mut self) -> Result<Option<Result<(), git2::Error>>> {
        let next = self.rebase.operation_current().map_or(0, |c| c + 1);
        let step = self.rebase.nth(next).map(|step| step.id());
        if let Some(id) = step {
            let commit = self.repo.find_commit(id)?;
            let head = self.repo.head()?.peel_to_commit()?;
            let mut reach = Reach::default();
            // libgit2's step merges the step's first parent, the checked-out commit, and the
            // step, as `rebase.c` does; a root commit's base is the empty tree. A merge commit
            // libgit2 refuses, so what the plain difference misses there is never written.
            let base = match commit.parent(0) {
                Ok(parent) => parent.tree()?,
                Err(_) => empty_tree(self.repo)?,
            };
            if commit.parent_count() <= 1 {
                let result = self
                    .repo
                    .merge_trees(&base, &head.tree()?, &commit.tree()?, None)?;
                reach.result(self.repo, Some(&head.tree()?), &result)?;
            } else {
                let diff = self.repo.diff_tree_to_tree(
                    Some(&head.tree()?),
                    Some(&commit.tree()?),
                    Some(&mut diff_options()),
                )?;
                reach.diff(&diff);
            }
            let labels = picked_labels(self.repo, id);
            let trees = trees_of(&commit)?;
            guard(
                self.repo,
                &reach,
                &trees.iter().collect::<Vec<_>>(),
                &[OURS, &labels.theirs],
            )?;
            let mut options = rebase_options(&labels)?;
            #[expect(
                clippy::disallowed_methods,
                reason = "opening writes nothing; the step it applies next went through the guard"
            )]
            let rebase = self.repo.open_rebase(Some(&mut options))?;
            // The old rebase goes first, then the options it read: libgit2 holds a pointer into
            // the options of the rebase it opened, kept alive beside it.
            self.rebase = rebase;
            self._options = options;
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
