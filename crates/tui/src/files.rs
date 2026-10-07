// Verifies: ADR-0020
//! The changed-files list in its two views (ADR-0020): flat, and a directory
//! tree with folders first and single-folder chains collapsed — ported from the
//! desktop's `fileTree.ts`. The tree is flattened into rows for a list widget,
//! skipping the contents of collapsed folders.

use ocelli_core::model::FileChange;
use std::collections::{BTreeMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileView {
    Flat,
    Tree,
}

/// One visible row of the files pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileRow {
    Dir {
        depth: usize,
        name: String,
        path: String,
        file_count: usize,
        open: bool,
    },
    File {
        depth: usize,
        /// The last path segment in the tree view, the full path in the flat one.
        name: String,
        /// Index into the commit's file list.
        index: usize,
    },
}

#[derive(Default)]
struct Dir {
    dirs: BTreeMap<String, Dir>,
    files: Vec<(String, usize)>,
}

/// The visible rows for `files` in `view`, with the folders in `collapsed`
/// (by path) closed.
pub fn rows(files: &[FileChange], view: FileView, collapsed: &HashSet<String>) -> Vec<FileRow> {
    match view {
        FileView::Flat => files
            .iter()
            .enumerate()
            .map(|(index, f)| FileRow::File {
                depth: 0,
                name: f.path.clone(),
                index,
            })
            .collect(),
        FileView::Tree => {
            let mut root = Dir::default();
            for (index, f) in files.iter().enumerate() {
                let mut parts: Vec<&str> = f.path.split('/').collect();
                let name = parts.pop().unwrap_or_default().to_string();
                let mut dir = &mut root;
                for part in parts {
                    dir = dir.dirs.entry(part.to_string()).or_default();
                }
                dir.files.push((name, index));
            }
            let mut out = Vec::new();
            emit(&root, "", 0, collapsed, &mut out);
            out
        }
    }
}

fn count(dir: &Dir) -> usize {
    dir.files.len() + dir.dirs.values().map(count).sum::<usize>()
}

/// Folders first, then files, each alphabetically. The root is a synthetic
/// container and takes no part in chain collapsing.
fn emit(
    dir: &Dir,
    prefix: &str,
    depth: usize,
    collapsed: &HashSet<String>,
    out: &mut Vec<FileRow>,
) {
    for (name, child) in &dir.dirs {
        // A folder holding exactly one folder and no files folds into it: `a/b`.
        let mut name = name.clone();
        let mut path = join(prefix, &name);
        let mut cur = child;
        while cur.files.is_empty() && cur.dirs.len() == 1 {
            let (only_name, only) = cur.dirs.iter().next().unwrap();
            name = format!("{name}/{only_name}");
            path = join(&path, only_name);
            cur = only;
        }
        let open = !collapsed.contains(&path);
        out.push(FileRow::Dir {
            depth,
            name,
            path: path.clone(),
            file_count: count(cur),
            open,
        });
        if open {
            emit(cur, &path, depth + 1, collapsed, out);
        }
    }
    let mut files = dir.files.clone();
    files.sort();
    for (name, index) in files {
        out.push(FileRow::File { depth, name, index });
    }
}

fn join(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}/{name}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocelli_core::model::FileStatus;

    fn files(paths: &[&str]) -> Vec<FileChange> {
        paths
            .iter()
            .map(|p| FileChange {
                path: p.to_string(),
                old_path: None,
                status: FileStatus::Modified,
                additions: 0,
                deletions: 0,
                staged: None,
                shown: None,
            })
            .collect()
    }

    fn show(rows: &[FileRow]) -> Vec<String> {
        rows.iter()
            .map(|r| match r {
                FileRow::Dir {
                    depth,
                    name,
                    file_count,
                    open,
                    ..
                } => format!(
                    "{}{}{name}/ ({file_count})",
                    "  ".repeat(*depth),
                    if *open { "v " } else { "> " }
                ),
                FileRow::File { depth, name, .. } => format!("{}{name}", "  ".repeat(*depth)),
            })
            .collect()
    }

    #[test]
    fn tree_puts_folders_first_and_collapses_single_folder_chains() {
        let f = files(&["README.md", "src/a/b/x.rs", "src/a/b/y.rs", "src/main.rs"]);
        assert_eq!(
            show(&rows(&f, FileView::Tree, &HashSet::new())),
            [
                "v src/ (3)",
                "  v a/b/ (2)",
                "    x.rs",
                "    y.rs",
                "  main.rs",
                "README.md"
            ]
        );
    }

    #[test]
    fn a_collapsed_folder_hides_its_contents() {
        let f = files(&["src/a/b/x.rs", "src/main.rs"]);
        let collapsed: HashSet<String> = ["src/a/b".to_string()].into();
        assert_eq!(
            show(&rows(&f, FileView::Tree, &collapsed)),
            ["v src/ (2)", "  > a/b/ (1)", "  main.rs"]
        );
    }

    #[test]
    fn flat_view_lists_full_paths_in_commit_order() {
        let f = files(&["z.rs", "a/b.rs"]);
        assert_eq!(
            show(&rows(&f, FileView::Flat, &HashSet::new())),
            ["z.rs", "a/b.rs"]
        );
    }
}
