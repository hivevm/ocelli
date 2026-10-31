//! The registry of the server's tabs (ADR-0035): each tab with a repository open is kept by the
//! repository's common directory, so that a write done, or refused after it moved a ref, tells
//! every other tab on the repository to load again before its turn is given back.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// What a tell is: telling a tab that another tab wrote to the repository it has open.
pub type Tell = Arc<dyn Fn() + Send + Sync>;

/// A tab's open repository, by its common directory, and how the tab is told of another's write.
struct Open {
    common: PathBuf,
    tell: Tell,
}

/// The repository each tab has open and the number the next tab gets.
#[derive(Default)]
pub struct Tabs {
    /// Each tab's open repository, by the tab's number; as many as the server has tabs.
    open: Mutex<HashMap<u64, Open>>,
    next_tab: AtomicU64,
}

impl Tabs {
    /// A number for a new tab, none other's.
    pub fn tab(&self) -> u64 {
        self.next_tab.fetch_add(1, Ordering::Relaxed)
    }

    /// Keeps that `tab` has the repository whose common directory is `common` open, none where
    /// it is `None`, and is told of another tab's write to it with `tell`; nothing once `gone`
    /// says the tab has closed, since it is forgotten then.
    pub fn watch(&self, tab: u64, common: Option<PathBuf>, tell: Tell, gone: &AtomicBool) {
        let mut open = self.open.lock().unwrap();
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
        self.open.lock().unwrap().remove(&tab);
    }

    /// Tells every tab but `by` that has the repository whose common directory is `common` open
    /// that it was written to.
    pub(super) fn written(&self, common: &Path, by: u64) {
        let tells: Vec<Tell> = self
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    // Verifies: ADR-0035
    fn a_write_tells_the_other_tabs_on_its_repository_and_no_tab_that_closed() {
        let tabs = Tabs::default();
        let told = Arc::new(Mutex::new(Vec::new()));
        let tell = |name: &'static str| -> Tell {
            let told = Arc::clone(&told);
            Arc::new(move || told.lock().unwrap().push(name))
        };
        let (one, two) = (PathBuf::from("/one/.git"), PathBuf::from("/two/.git"));
        let open = AtomicBool::new(false);
        let (writer, beside, elsewhere, closed) = (tabs.tab(), tabs.tab(), tabs.tab(), tabs.tab());
        assert_eq!(
            [writer, beside, elsewhere, closed]
                .iter()
                .collect::<HashSet<_>>()
                .len(),
            4,
            "each tab its own number"
        );
        tabs.watch(writer, Some(one.clone()), tell("writer"), &open);
        tabs.watch(beside, Some(one.clone()), tell("beside"), &open);
        tabs.watch(elsewhere, Some(two.clone()), tell("elsewhere"), &open);
        tabs.watch(closed, Some(one.clone()), tell("closed"), &open);
        tabs.forget(closed);
        // A tab that closed while its repository opened is not kept again.
        tabs.watch(
            closed,
            Some(one.clone()),
            tell("closed"),
            &AtomicBool::new(true),
        );
        tabs.written(&one, writer);
        assert_eq!(*told.lock().unwrap(), ["beside"]);
        // A tab that opened another repository, or none, is told no longer.
        tabs.watch(beside, Some(two.clone()), tell("beside"), &open);
        tabs.written(&one, writer);
        tabs.watch(beside, None, tell("beside"), &open);
        tabs.written(&two, writer);
        assert_eq!(*told.lock().unwrap(), ["beside", "elsewhere"]);
        assert_eq!(tabs.open.lock().unwrap().len(), 2, "writer and elsewhere");
    }
}
