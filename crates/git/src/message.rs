//! A commit's or a tag's message cleaned up as `git` cleans it up for the way it is made: what
//! `commit.cleanup` asks, and what `git` does where it is unset, read off `git` run on the same
//! input.

use anyhow::Result;
use git2::Repository;

/// How a message comes to a commit, which decides how `git` cleans it up by default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Made {
    /// Given by the developer, as `git commit -m` takes it: whitespace cleaned up.
    Given,
    /// Taken from the commit a cherry-pick or a revert applies without a conflict, as `git`
    /// commits it without an editor: kept verbatim.
    Taken,
    /// Prepared for a merge without a conflict, as `git merge --no-edit` commits it: whitespace
    /// cleaned up, or kept verbatim, without the line break that ends it.
    Merged,
    /// Prepared for a merge that stopped, as `git merge --continue` commits it through the
    /// editor: comment lines stripped, or cut at the scissors line. Under `whitespace` and
    /// `verbatim`, `git` commits the editor's template along, which Ocelli does not write.
    MergeContinued,
    /// Prepared for a cherry-pick or a revert that stopped, as `git cherry-pick --continue` and
    /// `git revert --continue` commit it: comment lines stripped, whatever `commit.cleanup` says.
    PickContinued,
}

/// `message` cleaned up as `git` cleans it up where it was `made` so.
pub(crate) fn cleaned(repo: &Repository, message: &str, made: Made) -> Result<String> {
    let mode = repo
        .config()?
        .get_string("commit.cleanup")
        .unwrap_or_default();
    let how = match (made, mode.as_str()) {
        (Made::PickContinued, _) => Cleanup::Strip,
        (Made::Merged, "verbatim") => {
            return Ok(message.strip_suffix('\n').unwrap_or(message).to_string());
        }
        (_, "verbatim") => Cleanup::Verbatim,
        (_, "strip") => Cleanup::Strip,
        (Made::MergeContinued, "whitespace") => Cleanup::Whitespace,
        (Made::MergeContinued, "scissors") => Cleanup::Scissors,
        (Made::MergeContinued, _) => Cleanup::Strip,
        (Made::Taken, "whitespace" | "scissors") => Cleanup::Whitespace,
        (Made::Taken, _) => Cleanup::Verbatim,
        (Made::Merged | Made::Given, _) => Cleanup::Whitespace,
    };
    Ok(match how {
        Cleanup::Verbatim => message.to_string(),
        Cleanup::Strip => stripped(repo, message)?,
        Cleanup::Whitespace => git2::message_prettify(message, None)?,
        Cleanup::Scissors => {
            let cut = format!(
                "{} ------------------------ >8 ------------------------",
                comment_string(repo)?
            );
            let kept = message
                .lines()
                .take_while(|l| *l != cut)
                .collect::<Vec<_>>();
            git2::message_prettify(kept.join("\n"), None)?
        }
    })
}

/// How `git` cleans a message up, in the words of `commit.cleanup`.
enum Cleanup {
    Verbatim,
    Whitespace,
    Strip,
    Scissors,
}

/// `message` cleaned up as `git tag` and a stripping commit clean it: comment lines, surrounding
/// blank lines, and trailing spaces go, and it ends in a newline; empty where nothing is left.
pub(crate) fn stripped(repo: &Repository, message: &str) -> Result<String> {
    let comment = comment_string(repo)?;
    let kept: Vec<_> = message
        .lines()
        .filter(|l| !l.starts_with(&comment))
        .collect();
    Ok(git2::message_prettify(kept.join("\n"), None)?)
}

/// What starts a comment line in a message, as `git` reads it: `core.commentString` or
/// `core.commentChar`, whichever the configuration sets last, and `#` where neither is set or
/// it is `auto`, which `git tag` does not adjust.
fn comment_string(repo: &Repository) -> Result<String> {
    let config = repo.config()?.snapshot()?;
    let mut comment = Ok(None);
    config
        .entries(Some("core\\.comment(char|string)"))?
        .for_each(|entry| {
            if comment.is_ok() {
                comment = crate::guard::config_value(entry).map(|v| v.map(str::to_string));
            }
        })?;
    Ok(comment?
        .filter(|c| !c.is_empty() && c != "auto")
        .unwrap_or_else(|| "#".to_string()))
}
