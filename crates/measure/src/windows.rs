//! `ocelli-measure windows PATH`: how long a developer waits on a large history (Q-2).
//!
//! The history is loaded as the front ends load it, through the core's `load` into a
//! `LoadSession` (ADR-0016, ADR-0019). Measured are the time from opening the repository until the
//! first window of rows is read, and, once the whole history has loaded, the time to read each
//! window of rows across it, top to bottom, on another thread than the one that loaded, as in the
//! front ends. The time to compute the lineage of the newest commit and to search the whole
//! history is reported beside them.

use anyhow::{Context, Result, anyhow};
use ocelli_core::load::{self, Event, LoadSession};
use ocelli_git::GitReader;
use std::path::Path;
use std::time::{Duration, Instant};

/// The rows of one window: a tall screen of the desktop application.
pub const WINDOW_ROWS: usize = 60;

/// What a load of one repository took.
pub struct Waits {
    pub commits: usize,
    /// From opening the repository until the first window is read.
    pub first_window: Duration,
    /// The whole load, until the walk ends.
    pub load: Duration,
    /// The slowest chunk taken in while the history loads, as a front end takes it in: appended,
    /// the lineage of a commit selected meanwhile grown by it, and the window in view read again.
    pub slowest_while_loading: Duration,
    /// The windows read across the loaded history, and the slowest of them.
    pub windows: usize,
    pub slowest_window: Duration,
    /// The lineage of the newest commit, which reaches nearly the whole history.
    pub lineage: Duration,
    /// A search through the whole history.
    pub search: Duration,
}

pub fn measure(path: &Path) -> Result<Waits> {
    let start = Instant::now();
    let mut session = LoadSession::new();
    let mut first_window = None;
    let mut slowest_while_loading = Duration::ZERO;
    let mut selected = None;
    let mut ended = Ok(());
    // The load runs on a thread of its own, as in both front ends, and the windows are read on
    // this one: the memory the load frees as it ends is then no cost of the first window read.
    std::thread::scope(|scope| {
        scope.spawn(|| {
            load::load(&GitReader, path, &mut |event| {
                match event {
                    Event::Progress(_) | Event::Refs(_) => {}
                    Event::Chunk(chunk) => {
                        let taken = Instant::now();
                        let from = session.append(chunk);
                        let history = session.history();
                        match &mut selected {
                            // The newest commit, selected as the first chunk shows: its lineage
                            // reaches into every chunk after.
                            None => selected = Some(history.lineage(0)),
                            Some(lineage) => history.extend_lineage(lineage, from),
                        }
                        let _ = history.window(0, WINDOW_ROWS);
                        if first_window.is_none() {
                            first_window = Some(start.elapsed());
                        } else {
                            slowest_while_loading = slowest_while_loading.max(taken.elapsed());
                        }
                    }
                    Event::Ended(result) => ended = result,
                }
                true
            });
        });
    });
    ended
        .map_err(|e| anyhow!(e))
        .with_context(|| format!("cannot read the repository at {}", path.display()))?;
    let load = start.elapsed();
    let first_window = first_window.context("the repository has no commits")?;
    let history = session.history();

    let mut slowest_window = Duration::ZERO;
    let mut windows = 0;
    for top in (0..history.rows()).step_by(WINDOW_ROWS) {
        let read = Instant::now();
        let _ = history.window(top, WINDOW_ROWS);
        slowest_window = slowest_window.max(read.elapsed());
        windows += 1;
    }
    let read = Instant::now();
    let _ = history.lineage(0);
    let lineage = read.elapsed();
    let read = Instant::now();
    let _ = history.search("ocelli-measure");
    let search = read.elapsed();

    Ok(Waits {
        commits: history.rows(),
        first_window,
        slowest_while_loading,
        load,
        windows,
        slowest_window,
        lineage,
        search,
    })
}
