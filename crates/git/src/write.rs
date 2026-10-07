//! The writes of the repository port, with git2 (ADR-0033). libgit2 runs no hook and no program
//! a repository's configuration names, so writing, like reading, runs no code the repository
//! brings (Q-4). Every write checks the state it expects before it changes anything.

use anyhow::{Context, Result, bail};
use git2::{ObjectType, Oid, Reference, Repository};
use ocelli_core::port::Write;
use std::path::Path;

/// Performs `write` on the repository at `repo_path`, or refuses it with nothing changed.
pub fn write(repo_path: &Path, write: &Write) -> Result<()> {
    let repo = Repository::discover(repo_path)?;
    match write {
        Write::CreateTag {
            name,
            commit,
            message,
        } => create_tag(&repo, name, commit, message.as_deref()),
        Write::DeleteTag {
            name,
            commit,
            annotated,
        } => delete_tag(&repo, name, commit, *annotated),
    }
}

fn create_tag(repo: &Repository, name: &str, commit: &str, message: Option<&str>) -> Result<()> {
    let refname = format!("refs/tags/{name}");
    // `git tag` refuses a leading dash too, which would read as an option.
    if name.starts_with('-') || !Reference::is_valid_name(&refname) {
        bail!("'{name}' is not a name a tag can have");
    }
    if repo.find_reference(&refname).is_ok() {
        bail!("a tag named '{name}' exists already");
    }
    let target = Oid::from_str(commit)
        .ok()
        .and_then(|oid| repo.find_object(oid, Some(ObjectType::Commit)).ok())
        .with_context(|| format!("the commit {commit} is no longer in the repository"))?;
    // Cleaned up as `git tag` cleans it: surrounding blank lines and trailing spaces go, and it
    // ends in a newline.
    let message = message
        .map(|m| git2::message_prettify(m, None))
        .transpose()?;
    match message.filter(|m| !m.is_empty()) {
        Some(message) => {
            let tagger = repo
                .signature()
                .context("no identity is configured to tag with: set user.name and user.email")?;
            repo.tag(name, &target, &tagger, &message, false)?;
        }
        None => {
            repo.tag_lightweight(name, &target, false)?;
        }
    }
    Ok(())
}

/// Deletes the tag, if it is still the one the developer saw: on `commit`, and annotated or not
/// as `annotated` says. libgit2 deletes the reference only while it still holds the value read
/// here, so a tag moved in between is not deleted either; it drops the tag's reflog before that
/// comparison, which only a repository logging every ref update keeps for tags.
fn delete_tag(repo: &Repository, name: &str, commit: &str, annotated: bool) -> Result<()> {
    let mut reference = repo
        .find_reference(&format!("refs/tags/{name}"))
        .with_context(|| format!("there is no tag named '{name}' any more"))?;
    let points_at = reference
        .peel(ObjectType::Commit)
        .map(|c| c.id().to_string());
    let is_annotated = reference.peel(ObjectType::Tag).is_ok();
    if points_at.ok().as_deref() != Some(commit) || is_annotated != annotated {
        bail!("the tag '{name}' is no longer the one shown: reload to see it as it is");
    }
    reference.delete()?;
    Ok(())
}
