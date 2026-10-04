// Verifies: ADR-0020
//! Folds a unified diff into side-by-side rows — ported from the desktop's
//! `diffSplit.ts`. Within a change block (deletes followed by adds, as git
//! emits them) the i-th deleted line pairs with the i-th added one; the longer
//! side runs against empty cells. Hunk headers and meta lines become banners.

use ocelli_core::model::{DiffLine, DiffLineKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellKind {
    Add,
    Delete,
    Context,
    Empty,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    pub kind: CellKind,
    pub content: String,
    pub lineno: Option<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SplitRow {
    Banner { kind: DiffLineKind, content: String },
    Line { left: Cell, right: Cell },
}

const EMPTY: Cell = Cell {
    kind: CellKind::Empty,
    content: String::new(),
    lineno: None,
};

pub fn split_rows(lines: &[DiffLine]) -> Vec<SplitRow> {
    let mut rows = Vec::new();
    let mut dels: Vec<&DiffLine> = Vec::new();
    let mut adds: Vec<&DiffLine> = Vec::new();

    let flush = |rows: &mut Vec<SplitRow>, dels: &mut Vec<&DiffLine>, adds: &mut Vec<&DiffLine>| {
        for i in 0..dels.len().max(adds.len()) {
            let left = dels.get(i).map_or(EMPTY, |l| Cell {
                kind: CellKind::Delete,
                content: l.content.clone(),
                lineno: l.old_lineno,
            });
            let right = adds.get(i).map_or(EMPTY, |l| Cell {
                kind: CellKind::Add,
                content: l.content.clone(),
                lineno: l.new_lineno,
            });
            rows.push(SplitRow::Line { left, right });
        }
        dels.clear();
        adds.clear();
    };

    for line in lines {
        match line.kind {
            DiffLineKind::Delete => dels.push(line),
            DiffLineKind::Add => adds.push(line),
            DiffLineKind::Context => {
                flush(&mut rows, &mut dels, &mut adds);
                let cell = |lineno| Cell {
                    kind: CellKind::Context,
                    content: line.content.clone(),
                    lineno,
                };
                rows.push(SplitRow::Line {
                    left: cell(line.old_lineno),
                    right: cell(line.new_lineno),
                });
            }
            DiffLineKind::Hunk | DiffLineKind::Meta => {
                flush(&mut rows, &mut dels, &mut adds);
                rows.push(SplitRow::Banner {
                    kind: line.kind.clone(),
                    content: line.content.clone(),
                });
            }
        }
    }
    flush(&mut rows, &mut dels, &mut adds);
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(kind: DiffLineKind, content: &str, old: Option<u32>, new: Option<u32>) -> DiffLine {
        DiffLine {
            kind,
            content: content.into(),
            old_lineno: old,
            new_lineno: new,
        }
    }

    #[test]
    fn deletes_and_adds_pair_up_and_the_longer_side_runs_against_empty_cells() {
        use DiffLineKind::*;
        let rows = split_rows(&[
            line(Hunk, "@@ -1,3 +1,2 @@", None, None),
            line(Context, "keep", Some(1), Some(1)),
            line(Delete, "old a", Some(2), None),
            line(Delete, "old b", Some(3), None),
            line(Add, "new a", None, Some(2)),
        ]);
        assert_eq!(rows.len(), 4);
        assert!(matches!(&rows[0], SplitRow::Banner { content, .. } if content.starts_with("@@")));
        let SplitRow::Line { left, right } = &rows[2] else {
            panic!()
        };
        assert_eq!(
            (left.content.as_str(), right.content.as_str()),
            ("old a", "new a")
        );
        let SplitRow::Line { left, right } = &rows[3] else {
            panic!()
        };
        assert_eq!((left.kind, right.kind), (CellKind::Delete, CellKind::Empty));
        assert_eq!(left.lineno, Some(3));
    }
}
