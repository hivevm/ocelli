//! `ocelli-measure`: measures Ocelli's quality goals on a real repository (#11).
//!
//! `ocelli-measure crossings PATH` lays out the first chunk of the repository at `PATH` as every
//! front end does (ADR-0016), its main line in the leftmost lane (ADR-0014), hands git exactly the commits laid out, and prints the number of
//! commits and the edge crossings in Ocelli's layout and in `git log --graph` over them (Q-1).
//! The stash and working-directory nodes are left out, since git draws neither (ADR-0018).
//!
//! `ocelli-measure windows PATH` loads the whole history of the repository at `PATH` as the front
//! ends do and prints, in milliseconds, how long the first window of rows took and how long the
//! slowest window across the loaded history took to read (Q-2), with the lineage and the search
//! beside them.

mod crossings;
mod git_graph;
mod windows;

use anyhow::{Context, Result, bail};
use ocelli_core::graph::Layout;
use ocelli_core::model::{CommitData, CommitKind};
use ocelli_core::port::RepositoryReader;
use ocelli_git::GitReader;
use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const USAGE: &str = "Usage: ocelli-measure crossings PATH | ocelli-measure windows PATH";

/// The measured numbers of one repository.
struct Crossings {
    commits: usize,
    /// The main line's tip the layout kept the leftmost lane for (ADR-0014).
    main_line: Option<String>,
    ocelli: usize,
    git: usize,
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [command, path] if command == "crossings" => {
            let measured = measure_crossings(&PathBuf::from(path))?;
            println!("commits: {}", measured.commits);
            println!(
                "main line: {}",
                measured.main_line.as_deref().unwrap_or("none")
            );
            println!("ocelli crossings: {}", measured.ocelli);
            println!("git crossings: {}", measured.git);
            Ok(())
        }
        [command, path] if command == "windows" => {
            let waits = windows::measure(&PathBuf::from(path))?;
            let ms = |d: std::time::Duration| format!("{:.1}", d.as_secs_f64() * 1000.0);
            println!("commits: {}", waits.commits);
            println!("first window ms: {}", ms(waits.first_window));
            println!("load ms: {}", ms(waits.load));
            println!(
                "slowest while loading ms: {}",
                ms(waits.slowest_while_loading)
            );
            println!("windows: {}", waits.windows);
            println!("slowest window ms: {}", ms(waits.slowest_window));
            println!("lineage ms: {}", ms(waits.lineage));
            println!("search ms: {}", ms(waits.search));
            Ok(())
        }
        _ => bail!("{USAGE}"),
    }
}

fn measure_crossings(path: &Path) -> Result<Crossings> {
    let commits: Vec<CommitData> = GitReader
        .load_first_chunk(path, &mut |_| {})
        .with_context(|| format!("cannot read the repository at {}", path.display()))?
        .into_iter()
        .filter(|c| c.kind == CommitKind::Commit)
        .collect();
    // Laid out as the front ends lay it out, the main line in the leftmost lane (ADR-0014).
    let tip = GitReader
        .load_main_tip(path)
        .with_context(|| format!("cannot read the main line of {}", path.display()))?;
    let mut layout = tip.clone().map_or_else(Layout::new, Layout::with_main_line);
    let edges = layout.lay_out(commits.clone()).edges;
    let ascii = git_log_graph(path, &commits)?;
    let rows: Vec<&str> = ascii.iter().map(|(graph, _)| graph.as_str()).collect();
    let shown: HashSet<&str> = ascii.iter().filter_map(|(_, sha)| sha.as_deref()).collect();
    let laid_out: HashSet<&str> = commits.iter().map(|c| c.sha.as_str()).collect();
    if shown != laid_out {
        bail!(
            "git shows {} commits and Ocelli lays out {}; the comparison needs the same commits",
            shown.len(),
            laid_out.len()
        );
    }
    Ok(Crossings {
        commits: commits.len(),
        main_line: tip,
        ocelli: crossings::count_edges(&edges),
        git: git_graph::count(&rows)?,
    })
}

/// The rows of `git log --graph` over exactly `commits`: each row's graph, and the SHA on the rows
/// that carry a commit.
///
/// The commits Ocelli lays out are the newest of the walk in commit-date order, which yields a
/// commit after its children unless a date runs backwards (ADR-0016), so the set is what its heads
/// reach, short of the ancestors of the parents left outside. git is given exactly those heads and
/// that boundary; where a date running backwards makes the two sets differ, the measurement says
/// so and fails.
fn git_log_graph(path: &Path, commits: &[CommitData]) -> Result<Vec<(String, Option<String>)>> {
    let inside: HashSet<&str> = commits.iter().map(|c| c.sha.as_str()).collect();
    let parents: HashSet<&str> = commits
        .iter()
        .flat_map(|c| c.parent_shas.iter().map(String::as_str))
        .collect();
    let mut input = String::new();
    for head in commits.iter().filter(|c| !parents.contains(c.sha.as_str())) {
        input.push_str(&head.sha);
        input.push('\n');
    }
    input.push_str("--not\n");
    for outside in parents.iter().filter(|p| !inside.contains(*p)) {
        input.push_str(outside);
        input.push('\n');
    }

    // The repository's own configuration must not choose what git runs: no signature check, which
    // would start the configured signing program, no colour, and no system or global config.
    let mut git = Command::new("git")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .arg("-C")
        .arg(path)
        .args(["-c", "log.showSignature=false", "-c", "color.ui=never"])
        .args(["log", "--graph", "--no-show-signature", "--no-color"])
        .args(["--format=%x00%H", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("cannot start git")?;
    git.stdin
        .take()
        .context("git's input is not open")?
        .write_all(input.as_bytes())?;
    let output = git.wait_with_output()?;
    if !output.status.success() {
        bail!(
            "git log failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8(output.stdout)?
        .lines()
        .map(|line| match line.split_once('\0') {
            Some((graph, sha)) => (graph.trim_end().to_string(), Some(sha.to_string())),
            None => (line.trim_end().to_string(), None),
        })
        .collect())
}
