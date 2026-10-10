//! One write per repository at a time across the server's tabs (ADR-0037): a write takes its
//! repository's turn, keyed by the canonical common directory, so that a worktree and the working
//! directory it belongs to count as one, and a write of another tab to it waits until the turn is
//! given back. The write checks what it expects only once its turn has come, so a write overtaken
//! is refused with nothing changed where the first changed it (ADR-0033).
//!
//! A waiting write holds a thread of the blocking pool, so a bounded number wait for each
//! repository, one past them refused at once, and a write whose tab has closed gives up its wait
//! without writing.
//!
//! Each tab with a repository open is kept here by the repository's common directory, so that a
//! write done, or refused after it moved a ref, tells every other tab on the repository to load
//! again before its turn is given back.

use ocelli_commands::HostError;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};

/// The repositories a write runs on now and how many writes wait for each, by common directory,
/// the signal that a write ended or a tab closed, how many may wait for one repository, and the
/// repository each tab has open.
#[derive(Clone)]
pub struct Turns(Arc<Shared>);

struct Shared {
    state: Mutex<State>,
    changed: Condvar,
    most_waiting: usize,
    /// Each tab's open repository, by the tab's number; as many as the server has tabs.
    open: Mutex<HashMap<u64, Open>>,
    next_tab: AtomicU64,
}

#[derive(Default)]
struct State {
    running: HashSet<PathBuf>,
    waiting: HashMap<PathBuf, usize>,
}

/// What a tell is: telling a tab that another tab wrote to the repository it has open.
pub type Tell = Arc<dyn Fn() + Send + Sync>;

/// A tab's open repository, by its common directory, and how the tab is told of another's write.
struct Open {
    common: PathBuf,
    tell: Tell,
}

/// A repository's turn, held until it is dropped.
pub struct Turn {
    turns: Turns,
    common: PathBuf,
    /// The write's tab, its repository's path, and the digest of its refs once the turn came,
    /// where the turn was taken for a tab's write.
    write: Option<(u64, PathBuf, Option<u64>)>,
}

/// The reason a write whose repository's common directory cannot be read fails with: it is git's
/// failure, which the browser reads only as such (ADR-0039).
const NO_COMMON_DIR: &str = "The repository's common directory cannot be read";

/// The reason a write is refused with where as many writes wait for its repository as may.
pub const TOO_MANY_WAITING: &str =
    "Too many writes wait for this repository: try again once they are done";

/// The reason a write whose tab closed while it waited ends with, unwritten; nobody reads it.
const GONE: &str = "The tab closed before the write's turn came";

impl Turns {
    /// Turns of which at most `most_waiting` writes wait for one repository.
    pub fn new(most_waiting: usize) -> Turns {
        Turns(Arc::new(Shared {
            state: Mutex::new(State::default()),
            changed: Condvar::new(),
            most_waiting,
            open: Mutex::new(HashMap::new()),
            next_tab: AtomicU64::new(0),
        }))
    }

    /// A number for a new tab, none other's.
    pub fn tab(&self) -> u64 {
        self.0.next_tab.fetch_add(1, Ordering::Relaxed)
    }

    /// Keeps that `tab` has the repository whose common directory is `common` open, none where
    /// it is `None`, and is told of another tab's write to it with `tell`; nothing once `gone`
    /// says the tab has closed, since it is forgotten then.
    pub fn watch(&self, tab: u64, common: Option<PathBuf>, tell: Tell, gone: &AtomicBool) {
        let mut open = self.0.open.lock().unwrap();
        // Read under the lock `forget` takes: a tab gone before is not kept again after.
        if gone.load(Ordering::Acquire) {
            return;
        }
        match common {
            Some(common) => open.insert(tab, Open { common, tell }),
            None => open.remove(&tab),
        };
    }

    /// Forgets `tab`, once it has closed and `gone` says so.
    pub fn forget(&self, tab: u64) {
        self.0.open.lock().unwrap().remove(&tab);
    }

    /// Tells every tab but `by` that has the repository whose common directory is `common` open
    /// that it was written to.
    fn written(&self, common: &Path, by: u64) {
        let tells: Vec<Tell> = self
            .0
            .open
            .lock()
            .unwrap()
            .iter()
            .filter(|(tab, open)| **tab != by && open.common == common)
            .map(|(_, open)| Arc::clone(&open.tell))
            .collect();
        for tell in tells {
            tell();
        }
    }

    /// Waits until no other write runs on the repository whose common directory is `common`, and
    /// takes its turn; refused at once where as many writes wait for it as may, and without its
    /// turn once `gone` says its tab has closed and `wake` is called. It blocks: it runs on the
    /// blocking pool.
    pub fn take(&self, common: PathBuf, gone: &AtomicBool) -> Result<Turn, HostError> {
        let shared = &self.0;
        let mut state = shared.state.lock().unwrap();
        if state.running.contains(&common) {
            let waiting = state.waiting.entry(common.clone()).or_default();
            if *waiting >= shared.most_waiting {
                return Err(HostError::Refused(TOO_MANY_WAITING.into()));
            }
            *waiting += 1;
            state = shared
                .changed
                .wait_while(state, |state| {
                    state.running.contains(&common) && !gone.load(Ordering::Acquire)
                })
                .unwrap();
            let waiting = state.waiting.get_mut(&common).expect("counted above");
            *waiting -= 1;
            if *waiting == 0 {
                state.waiting.remove(&common);
            }
        }
        if gone.load(Ordering::Acquire) {
            return Err(HostError::Refused(GONE.into()));
        }
        state.running.insert(common.clone());
        Ok(Turn {
            turns: self.clone(),
            common,
            write: None,
        })
    }

    /// The turn of the repository at `path`, one the root has let through, for the shared `Host`
    /// to hold while `tab` writes; none once `gone`. Where its refs stood once the turn came is
    /// kept, to tell the other tabs after a write refused that moved one.
    pub fn of(
        &self,
        path: &Path,
        gone: &AtomicBool,
        tab: u64,
    ) -> Result<Box<dyn ocelli_commands::Turn>, HostError> {
        let common = ocelli_git::confine::common_dir(path)
            .ok_or_else(|| HostError::Git(NO_COMMON_DIR.into()))?;
        let mut turn = self.take(common, gone)?;
        let before = ocelli_git::confine::refs_digest(path);
        turn.write = Some((tab, path.to_path_buf(), before));
        Ok(Box::new(turn))
    }

    /// Wakes every waiting write to look at its tab's `gone` again, after a tab closed.
    pub fn wake(&self) {
        // Taken, so that a write between reading `gone` and waiting is not missed.
        drop(self.0.state.lock().unwrap());
        self.0.changed.notify_all();
    }

    /// How many writes wait for the repository whose common directory is `common`.
    pub fn waiting(&self, common: &Path) -> usize {
        let state = self.0.state.lock().unwrap();
        state.waiting.get(common).copied().unwrap_or(0)
    }
}

impl ocelli_commands::Turn for Turn {
    /// A write done, or refused or failed after it moved a ref or `HEAD`, tells the other tabs
    /// on the repository to load again, while the turn is still held (ADR-0037).
    fn ended(&mut self, done: bool) {
        let Some((tab, path, before)) = &self.write else {
            return;
        };
        if done || ocelli_git::confine::refs_digest(path) != *before {
            self.turns.written(&self.common, *tab);
        }
    }
}

impl Drop for Turn {
    fn drop(&mut self) {
        let shared = &self.turns.0;
        shared.state.lock().unwrap().running.remove(&self.common);
        shared.changed.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    // Verifies: ADR-0037
    fn a_write_waits_for_the_turn_of_its_repository_and_not_for_another_s() {
        let turns = Turns::new(4);
        let (one, two) = (PathBuf::from("/one/.git"), PathBuf::from("/two/.git"));
        let first = turns.take(one.clone(), &AtomicBool::new(false)).unwrap();
        let (taken, told) = mpsc::channel();
        let waiting = {
            let turns = turns.clone();
            std::thread::spawn(move || {
                let _turn = turns.take(one, &AtomicBool::new(false)).unwrap();
                taken.send(()).unwrap();
            })
        };
        // Another repository's write does not wait.
        drop(turns.take(two, &AtomicBool::new(false)).unwrap());
        assert!(
            told.recv_timeout(Duration::from_millis(200)).is_err(),
            "the second write to one waits while the first runs"
        );
        drop(first);
        told.recv_timeout(Duration::from_secs(20))
            .expect("the second write takes its turn once the first ends");
        waiting.join().unwrap();
        let state = turns.0.state.lock().unwrap();
        assert!(state.running.is_empty(), "every turn given back");
        assert!(state.waiting.is_empty(), "no write counted as waiting");
    }

    /// Waits until `n` writes wait for `common`.
    fn until_waiting(turns: &Turns, common: &Path, n: usize) {
        let since = std::time::Instant::now();
        while turns.waiting(common) != n {
            assert!(
                since.elapsed() < Duration::from_secs(20),
                "{n} never waited"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    // Verifies: ADR-0037
    fn a_write_tells_the_other_tabs_on_its_repository_and_no_tab_that_closed() {
        let turns = Turns::new(4);
        let told = Arc::new(Mutex::new(Vec::new()));
        let tell = |name: &'static str| -> Tell {
            let told = Arc::clone(&told);
            Arc::new(move || told.lock().unwrap().push(name))
        };
        let (one, two) = (PathBuf::from("/one/.git"), PathBuf::from("/two/.git"));
        let open = AtomicBool::new(false);
        let (writer, beside, elsewhere, closed) =
            (turns.tab(), turns.tab(), turns.tab(), turns.tab());
        assert_eq!(
            [writer, beside, elsewhere, closed]
                .iter()
                .collect::<HashSet<_>>()
                .len(),
            4,
            "each tab its own number"
        );
        turns.watch(writer, Some(one.clone()), tell("writer"), &open);
        turns.watch(beside, Some(one.clone()), tell("beside"), &open);
        turns.watch(elsewhere, Some(two.clone()), tell("elsewhere"), &open);
        turns.watch(closed, Some(one.clone()), tell("closed"), &open);
        turns.forget(closed);
        // A tab that closed while its repository opened is not kept again.
        turns.watch(
            closed,
            Some(one.clone()),
            tell("closed"),
            &AtomicBool::new(true),
        );
        turns.written(&one, writer);
        assert_eq!(*told.lock().unwrap(), ["beside"]);
        // A tab that opened another repository, or none, is told no longer.
        turns.watch(beside, Some(two.clone()), tell("beside"), &open);
        turns.written(&one, writer);
        turns.watch(beside, None, tell("beside"), &open);
        turns.written(&two, writer);
        assert_eq!(*told.lock().unwrap(), ["beside", "elsewhere"]);
        assert_eq!(
            turns.0.open.lock().unwrap().len(),
            2,
            "writer and elsewhere"
        );
    }

    #[test]
    // Verifies: ADR-0037
    fn writes_past_the_bound_are_refused_at_once_and_a_closed_tab_s_write_gives_up_unwritten() {
        let turns = Turns::new(1);
        let one = PathBuf::from("/one/.git");
        let first = turns.take(one.clone(), &AtomicBool::new(false)).unwrap();
        let gone = Arc::new(AtomicBool::new(false));
        let waiting = {
            let (turns, one, gone) = (turns.clone(), one.clone(), Arc::clone(&gone));
            std::thread::spawn(move || turns.take(one, &gone).map(drop))
        };
        until_waiting(&turns, &one, 1);
        // One waits, as many as may: the next is refused without waiting.
        assert_eq!(
            turns.take(one.clone(), &AtomicBool::new(false)).map(drop),
            Err(HostError::Refused(TOO_MANY_WAITING.into()))
        );
        // Its tab closes while the first write still runs: it gives up, without the turn.
        gone.store(true, Ordering::Release);
        turns.wake();
        assert_eq!(
            waiting.join().unwrap(),
            Err(HostError::Refused(GONE.into()))
        );
        assert_eq!(turns.waiting(&one), 0);
        // A closed tab's write that finds the repository free does not start either.
        drop(first);
        assert!(turns.take(one.clone(), &gone).is_err());
        assert!(turns.0.state.lock().unwrap().running.is_empty());
    }
}
