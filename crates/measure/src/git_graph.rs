//! Counting the *edge crossings* (glossary) in the ASCII graph of `git log --graph` (Q-1).
//!
//! `|` and `*` are vertical pieces in an even text column. The pieces with which a line moves span
//! the two even columns beside them: `/` from the upper right to the lower left, `\` from the
//! upper left to the lower right, and `_` along the lower boundary of its row. A moving line can
//! therefore meet another only at a point on a row boundary in an even column, the points the
//! glossary's reading of git's graph names.

use anyhow::{Result, bail};
use std::collections::HashMap;

/// A point on the grid, in half rows: row boundary `2 * row + 1` lies below text row `row`.
type Point = (i64, i64);

/// The non-vertical pieces that end at each point, as the column of their other end.
fn moving_ends(lines: &[&str]) -> Result<HashMap<Point, Vec<i64>>> {
    let mut ends: HashMap<Point, Vec<i64>> = HashMap::new();
    let mut add = |a: Point, b: Point| {
        ends.entry(a).or_default().push(b.1);
        ends.entry(b).or_default().push(a.1);
    };
    for (row, line) in lines.iter().enumerate() {
        let (top, bottom) = (2 * row as i64 - 1, 2 * row as i64 + 1);
        for (x, glyph) in line.chars().enumerate() {
            let x = x as i64;
            match glyph {
                '/' => add((top, x + 1), (bottom, x - 1)),
                '\\' => add((top, x - 1), (bottom, x + 1)),
                '_' => add((bottom, x - 1), (bottom, x + 1)),
                '|' | '*' | ' ' => {}
                other => bail!("unsupported glyph {other:?} in row {row} of git's graph"),
            }
        }
    }
    Ok(ends)
}

/// Whether a vertical piece (`|` or `*`) sits in column `x` of text row `row`.
fn vertical(lines: &[&str], row: i64, x: i64) -> bool {
    row >= 0
        && x >= 0
        && lines
            .get(row as usize)
            .and_then(|line| line.chars().nth(x as usize))
            .is_some_and(|c| c == '|' || c == '*')
}

/// The number of edge crossings in an ASCII graph of `git log --graph`, one text row per line,
/// the commit's text after the graph already cut off.
pub fn count(lines: &[&str]) -> Result<usize> {
    let ends = moving_ends(lines)?;
    let mut crossings = 0;
    for (&(boundary, x), others) in &ends {
        // Text rows above and below the boundary; a vertical line has to run through both.
        let (above, below) = ((boundary - 1) / 2, (boundary + 1) / 2);
        if !vertical(lines, above, x) || !vertical(lines, below, x) {
            continue;
        }
        let left = others.iter().filter(|&&o| o < x).count();
        let right = others.iter().filter(|&&o| o > x).count();
        if left > 0 && right > 0 {
            crossings += 1;
        }
    }
    Ok(crossings)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(graph: &str) -> Vec<&str> {
        graph.lines().collect()
    }

    #[test]
    fn straight_lines_do_not_cross() {
        assert_eq!(count(&rows("* |\n| *\n* |\n")).unwrap(), 0);
    }

    #[test]
    fn a_fork_and_a_merge_into_a_running_column_do_not_cross() {
        let graph = "*   \n|\\  \n| * \n|/  \n*   \n";
        assert_eq!(count(&rows(graph)).unwrap(), 0);
    }

    #[test]
    fn a_line_moving_past_one_running_column_crosses_it_once() {
        // The line in column 4 moves to column 0 past column 2, which runs on.
        let graph = "| | *\n| |/ \n|/|  \n* |  \n";
        assert_eq!(count(&rows(graph)).unwrap(), 1);
    }

    #[test]
    fn a_line_moving_past_two_running_columns_crosses_twice() {
        // Taken from git's graph of rust-lang/cargo: the line from column 6 moves to column 0
        // along the boundary under `_`, past columns 4 and 2, which both run on.
        let graph = "| | | *\n| |_|/ \n|/| |  \n| | |  \n";
        assert_eq!(count(&rows(graph)).unwrap(), 2);
    }

    #[test]
    fn a_line_that_ends_in_a_column_it_reaches_does_not_cross_it() {
        // The line from column 4 joins column 2 and stops there.
        let graph = "| | *\n| |/ \n| *  \n";
        assert_eq!(count(&rows(graph)).unwrap(), 0);
    }

    #[test]
    fn a_line_moving_along_a_boundary_crosses_each_running_column_it_passes() {
        // The line from column 6 moves under `_` to column 0, past columns 4 and 2, which run on,
        // and joins column 0.
        let graph = "| | | *\n|_|_|/ \n| | |  \n";
        assert_eq!(count(&rows(graph)).unwrap(), 2);
    }

    #[test]
    fn a_half_step_between_columns_crosses_nothing() {
        // A `/` in an even column ends between columns, where no line runs.
        let graph = "|  *\n| / \n|/  \n";
        assert_eq!(count(&rows(graph)).unwrap(), 0);
    }

    #[test]
    fn an_unknown_glyph_is_refused_rather_than_misread() {
        assert!(count(&rows("*-.\n")).is_err());
    }
}
