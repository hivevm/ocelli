//! The terminal application's side of a write (ADR-0033), following the web interface's
//! `writes.ts`: the requests built from what the developer typed, and the one write that runs at
//! a time. [`App`](crate::app::App) keeps the keys that start a write and hands each to
//! [`OneWrite::queue`], the only way into the slot the event loop takes it from.

use ocelli_core::model::FileChange;
use ocelli_core::port::{Confirmation, Write};

/// The tag the prompt describes on `sha`, or `None` while it has no name. A blank message makes
/// a lightweight tag.
pub fn tag_request(sha: &str, name: &str, message: String) -> Option<Write> {
    let name = name.trim();
    (!name.is_empty()).then(|| Write::CreateTag {
        name: name.to_string(),
        commit: sha.to_string(),
        message: blank_is_none(message),
    })
}

/// The stash the prompt describes.
pub fn stash_request(message: String, untracked: bool) -> Write {
    Write::CreateStash {
        message: blank_is_none(message),
        untracked,
    }
}

/// What `a` or `u` on a working-directory file asks for: staged or unstaged whole, expecting the
/// content shown; nothing for a file that does not stage, a commit's.
pub fn stage_request(file: &FileChange, stage: bool) -> Option<Write> {
    let path = file.path.clone();
    let shown = &file.staging.as_ref()?.shown;
    Some(if stage {
        Write::StageFile {
            path,
            shown: shown.workdir.clone(),
        }
    } else {
        Write::UnstageFile {
            path,
            shown: shown.index.clone(),
        }
    })
}

/// The branch the prompt describes at `commit`, or `None` while it has no name.
pub fn branch_request(commit: &str, name: &str, switch: bool) -> Option<Write> {
    let name = name.trim();
    (!name.is_empty()).then(|| Write::CreateBranch {
        name: name.to_string(),
        commit: commit.to_string(),
        switch,
    })
}

/// The commit the prompt describes on `head`, or `None` while it has no message.
pub fn commit_request(head: &str, message: &str) -> Option<Write> {
    (!message.trim().is_empty()).then(|| Write::Commit {
        message: message.to_string(),
        head: Some(head.to_string()),
    })
}

fn blank_is_none(text: String) -> Option<String> {
    Some(text).filter(|t| !t.trim().is_empty())
}

/// The one write the terminal application runs at a time: a second is not queued while one
/// runs, from whatever key, so a second press writes nothing. The slot and the flag guarding it
/// change only here.
#[derive(Debug, Default)]
pub struct OneWrite {
    /// The write for the event loop to run; its result comes back as `Msg::Written`.
    slot: Option<(Write, Confirmation)>,
    /// Whether a write runs: no second one starts until its result is in.
    running: bool,
    /// Whether the write that runs is a transfer, after which the repository is read again
    /// even where it is refused (ADR-0035).
    transferring: bool,
    /// Whether the write that runs can be stopped, as the core says (ADR-0035).
    cancellable: bool,
    /// Asks the event loop to stop the write that runs.
    cancel: bool,
}

impl OneWrite {
    /// Hands `write` to the event loop, unless one runs still; whether it was queued.
    pub fn queue(&mut self, write: Write, confirmation: Confirmation) -> bool {
        if self.running {
            return false;
        }
        self.running = true;
        self.transferring = write.is_transfer();
        self.cancellable = write.can_stop();
        self.slot = Some((write, confirmation));
        true
    }

    /// The write queued, taken by the event loop to run it.
    pub fn take(&mut self) -> Option<(Write, Confirmation)> {
        self.slot.take()
    }

    /// The write's result is in: the next one may start. Whether the write reached a remote.
    pub fn answered(&mut self) -> bool {
        self.running = false;
        self.cancellable = false;
        std::mem::take(&mut self.transferring)
    }

    /// Whether a write runs.
    pub fn running(&self) -> bool {
        self.running
    }

    /// Whether the write that runs can be stopped.
    #[cfg(test)]
    pub fn cancellable(&self) -> bool {
        self.cancellable
    }

    /// Asks to stop the write that runs, where it can be stopped; whether it was asked.
    pub fn cancel(&mut self) -> bool {
        self.cancel |= self.cancellable;
        self.cancellable
    }

    /// Whether the write that runs is to stop, taken by the event loop.
    pub fn take_cancel(&mut self) -> bool {
        std::mem::take(&mut self.cancel)
    }

    /// The write queued, which the tests look into without taking it.
    #[cfg(test)]
    pub fn queued(&self) -> Option<(Write, Confirmation)> {
        self.slot.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detach(commit: &str) -> Write {
        Write::Detach {
            commit: commit.into(),
        }
    }

    #[test]
    fn a_second_write_is_not_queued_until_the_first_is_answered() {
        let mut one = OneWrite::default();
        assert!(one.queue(detach("a"), Confirmation::Absent));
        assert_eq!(one.take(), Some((detach("a"), Confirmation::Absent)));
        assert!(!one.queue(detach("b"), Confirmation::Absent));
        assert_eq!(one.take(), None, "the second press writes nothing");
        assert!(!one.answered());
        assert!(one.queue(detach("b"), Confirmation::Absent));
        assert_eq!(one.take(), Some((detach("b"), Confirmation::Absent)));
    }

    #[test]
    fn a_fetch_and_a_pull_can_be_stopped_and_a_push_cannot() {
        let mut one = OneWrite::default();
        assert!(!one.cancel(), "nothing runs to stop");
        assert!(!one.take_cancel());
        let pull = Write::Pull {
            branch: "main".into(),
            tip: "a".into(),
        };
        one.queue(pull, Confirmation::Absent);
        assert!(one.cancel());
        assert!(one.take_cancel());
        assert!(one.answered(), "a pull reaches a remote");
        let push = Write::Push {
            branch: "main".into(),
            tip: "a".into(),
        };
        one.queue(push, Confirmation::Absent);
        assert!(!one.cancel());
        assert!(!one.take_cancel());
        assert!(one.answered(), "a push reaches a remote");
    }

    #[test]
    fn a_name_or_message_of_blanks_builds_no_request() {
        assert_eq!(tag_request("a", "  ", String::new()), None);
        assert_eq!(branch_request("a", " ", true), None);
        assert_eq!(commit_request("a", " \n"), None);
        assert_eq!(
            tag_request("a", " v1 ", " ".into()),
            Some(Write::CreateTag {
                name: "v1".into(),
                commit: "a".into(),
                message: None,
            }),
            "the name is trimmed and a blank message makes a lightweight tag"
        );
    }
}
