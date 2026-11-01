//! The merge editor's file (ADR-0031, G-15): a conflicted file's text as the working directory
//! holds it, and its blocks, found here once for every front end. A block runs from a `<<<<<<<`
//! line to its `>>>>>>>` line, ours before `=======` and theirs after it, its base the `|||||||`
//! section where the file holds one, as the `merge`, `diff3`, and `zdiff3` styles write them,
//! else the bases of the run of blocks holding the same ours and theirs in a merge of the
//! index's three stages in memory in the `diff3` style.

use serde::Serialize;

/// A conflicted file the merge editor opens: its text, the blob id of the working directory's
/// file it was read from, which saving expects to find again, and its blocks.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MergeFile {
    pub text: String,
    pub blob: String,
    pub blocks: Vec<MergeBlock>,
}

/// One block of a conflicted file: the lines of its first and last marker, counted from 1, and
/// the text of ours, theirs, and its base where it has one, each line with its line ending. The
/// base is the file's own `|||||||` section where `base_in_file`, else it was found in memory
/// and is in none of the file's lines.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MergeBlock {
    pub first_line: usize,
    pub last_line: usize,
    pub ours: String,
    pub theirs: String,
    pub base: Option<String>,
    pub base_in_file: bool,
}

/// Whether a line, its line ending left off, is one of the markers `git` writes into a
/// conflicted file.
pub fn is_marker(line: &str) -> bool {
    let line = line.strip_suffix('\r').unwrap_or(line);
    ["<<<<<<<", ">>>>>>>", "|||||||"]
        .iter()
        .any(|m| line == *m || line.strip_prefix(m).is_some_and(|r| r.starts_with(' ')))
        || line == "======="
}

/// Whether `text` still holds a marker line, which saving it as a resolution refuses.
pub fn has_marker(text: &str) -> bool {
    text.lines().any(is_marker)
}

/// The text of a file the merge editor opens: one without a NUL byte that is UTF-8 throughout;
/// `None` for any other.
pub fn as_text(bytes: &[u8]) -> Option<&str> {
    if bytes.contains(&0) {
        return None;
    }
    std::str::from_utf8(bytes).ok()
}

/// `result`, whose lines an editor ends with `\n`, with the line endings of `opened`, the file
/// it was opened from: `\r\n` where that file ended its lines so.
pub fn with_line_endings(opened: &str, result: &str) -> String {
    let result = result.replace("\r\n", "\n");
    if opened.contains("\r\n") {
        result.replace('\n', "\r\n")
    } else {
        result
    }
}

/// Where a block's lines go while it is read.
enum Part {
    Ours,
    Base,
    Theirs,
}

/// The blocks of `text`. A block that a marker line opens but none closes is no block.
pub fn blocks(text: &str) -> Vec<MergeBlock> {
    let mut found = Vec::new();
    let mut open: Option<(MergeBlock, Part)> = None;
    for (i, line) in text.split_inclusive('\n').enumerate() {
        let bare = line.trim_end_matches('\n').trim_end_matches('\r');
        let marker = is_marker(bare).then(|| &bare[..7]);
        match (marker, open.as_mut()) {
            (Some("<<<<<<<"), _) => {
                let block = MergeBlock {
                    first_line: i + 1,
                    last_line: i + 1,
                    ours: String::new(),
                    theirs: String::new(),
                    base: None,
                    base_in_file: false,
                };
                open = Some((block, Part::Ours));
            }
            (Some("|||||||"), Some((block, part @ Part::Ours))) => {
                block.base = Some(String::new());
                block.base_in_file = true;
                *part = Part::Base;
            }
            (Some("======="), Some((_, part @ (Part::Ours | Part::Base)))) => *part = Part::Theirs,
            (Some(">>>>>>>"), Some((_, Part::Theirs))) => {
                if let Some((mut block, _)) = open.take() {
                    block.last_line = i + 1;
                    found.push(block);
                }
            }
            (_, Some((block, part))) => match part {
                Part::Ours => block.ours.push_str(line),
                Part::Base => block.base.get_or_insert_default().push_str(line),
                Part::Theirs => block.theirs.push_str(line),
            },
            (_, None) => {}
        }
    }
    found
}

/// The blocks of `text`, each without a base of its own given the bases of the run of blocks of
/// `merged`, the index's three stages merged in memory in the `diff3` style, that holds the same
/// ours and theirs, in the line endings of `text`; a block that none holds keeps none. A run is
/// one block or several in a row, joined by the lines between them, and without the lines both
/// sides share at its ends: `git`'s `merge` style moves those out of a block and joins
/// conflicts close together into one, the `diff3` style does neither. Its base is the runs'
/// bases joined by the same lines. Blocks take the runs in their order, each block once.
pub fn blocks_with_bases(text: &str, merged: &str) -> Vec<MergeBlock> {
    let lf = |s: &str| s.replace("\r\n", "\n");
    let merged = lf(merged);
    let runs = Runs::of(&merged);
    let mut from = 0;
    let mut found = blocks(text);
    for block in found.iter_mut().filter(|b| b.base.is_none()) {
        let (ours, theirs) = (lf(&block.ours), lf(&block.theirs));
        if let Some((base, next)) = runs.base_of(from, &ours, &theirs) {
            block.base = Some(with_line_endings(text, &base));
            from = next;
        }
    }
    found
}

/// The blocks of a merge in the `diff3` style, ours, base, and theirs each as lines, `None` for
/// one without a base, which no run holds, and the lines between each block and the next.
struct Runs {
    blocks: Vec<Option<[Vec<String>; 3]>>,
    between: Vec<Vec<String>>,
}

/// `text`'s lines, each with its line ending.
fn lines_of(text: &str) -> Vec<String> {
    text.split_inclusive('\n').map(str::to_string).collect()
}

impl Runs {
    fn of(merged: &str) -> Self {
        let lines = lines_of(merged);
        let found = blocks(merged);
        let between = found
            .windows(2)
            .map(|pair| lines[pair[0].last_line..pair[1].first_line - 1].to_vec())
            .collect();
        let blocks = found
            .into_iter()
            .map(|b| Some([lines_of(&b.ours), lines_of(&b.base?), lines_of(&b.theirs)]))
            .collect();
        Runs { blocks, between }
    }

    /// The base of the first run from block `from` on that holds `ours` and `theirs`, and the
    /// block after the run.
    fn base_of(&self, from: usize, ours: &str, theirs: &str) -> Option<(String, usize)> {
        let (ours, theirs) = (lines_of(ours), lines_of(theirs));
        for first in from..self.blocks.len() {
            // A run sheds only lines both sides share, never a side whole: the block's first
            // lines are among those of the run's first block.
            let Some([first_ours, _, first_theirs]) = &self.blocks[first] else {
                continue;
            };
            let starts = |side: &[String], run: &[String]| {
                side.first()
                    .is_none_or(|line| run.is_empty() || run.contains(line))
            };
            if !starts(&ours, first_ours) || !starts(&theirs, first_theirs) {
                continue;
            }
            let mut joined: [Vec<String>; 3] = Default::default();
            for last in first..self.blocks.len() {
                let Some(block) = &self.blocks[last] else {
                    break;
                };
                if last > first {
                    for part in &mut joined {
                        part.extend_from_slice(&self.between[last - 1]);
                    }
                }
                for (part, lines) in joined.iter_mut().zip(block) {
                    part.extend_from_slice(lines);
                }
                if holds(&joined[0], &joined[2], &ours, &theirs) {
                    return Some((joined[1].concat(), last + 1));
                }
                // The lines a run may shed are its first block's and its last's: past those, a
                // longer run holds more than the block.
                if last > first && joined[0].len() - first_ours.len() - block[0].len() > ours.len()
                {
                    break;
                }
            }
        }
        None
    }
}

/// Whether `run_ours` and `run_theirs` are `ours` and `theirs` with the same lines before them and
/// the same lines after them.
fn holds(run_ours: &[String], run_theirs: &[String], ours: &[String], theirs: &[String]) -> bool {
    let Some(shed) = run_ours.len().checked_sub(ours.len()) else {
        return false;
    };
    if run_theirs.len().checked_sub(theirs.len()) != Some(shed) {
        return false;
    }
    (0..=shed).any(|before| {
        let after = shed - before;
        run_ours[..before] == run_theirs[..before]
            && run_ours[run_ours.len() - after..] == run_theirs[run_theirs.len() - after..]
            && run_ours[before..before + ours.len()] == *ours
            && run_theirs[before..before + theirs.len()] == *theirs
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MERGE: &str = "a\n<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> other\nz\n";

    #[test]
    fn a_block_in_the_merge_style_has_ours_and_theirs_and_no_base() {
        // Verifies: ADR-0031, G-15
        assert_eq!(
            blocks(MERGE),
            [MergeBlock {
                first_line: 2,
                last_line: 6,
                ours: "ours\n".into(),
                theirs: "theirs\n".into(),
                base: None,
                base_in_file: false,
            }]
        );
    }

    #[test]
    fn a_block_in_the_diff3_and_zdiff3_styles_has_its_base() {
        // Verifies: ADR-0031, G-15
        // zdiff3 writes the markers of diff3 and moves the lines both sides share out of them.
        for text in [
            "<<<<<<< HEAD\nours\n||||||| base\nbase\n=======\ntheirs\n>>>>>>> other\n",
            "shared\n<<<<<<< HEAD\nours\n||||||| 1234567\n=======\ntheirs\n>>>>>>> other\n",
        ] {
            let found = blocks(text);
            assert_eq!(found.len(), 1, "{text}");
            assert_eq!(found[0].ours, "ours\n");
            assert_eq!(found[0].theirs, "theirs\n");
            assert!(found[0].base.is_some(), "{text}");
            assert!(found[0].base_in_file, "{text}");
        }
        assert_eq!(
            blocks("<<<<<<< HEAD\nours\n||||||| base\nbase\n=======\ntheirs\n>>>>>>> other\n")[0]
                .base
                .as_deref(),
            Some("base\n")
        );
    }

    #[test]
    fn the_blocks_of_a_file_edited_since_are_those_still_marked() {
        // Verifies: ADR-0031, G-15
        let text = "kept\n<<<<<<< HEAD\nx\n=======\ny\n>>>>>>> o\n\
                    edited\n<<<<<<< HEAD\nunclosed\n=======\n";
        let found = blocks(text);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].first_line, found[0].last_line), (2, 6));
    }

    #[test]
    fn a_block_without_a_base_takes_that_of_the_merged_block_with_its_ours_and_theirs() {
        // Verifies: ADR-0031, G-15
        let text = "<<<<<<< HEAD\r\nx\r\n=======\r\ny\r\n>>>>>>> o\r\nmid\r\n\
                    <<<<<<< HEAD\r\nedited\r\n=======\r\nq\r\n>>>>>>> o\r\n\
                    <<<<<<< HEAD\r\nx\r\n=======\r\ny\r\n>>>>>>> o\r\n";
        let merged = "<<<<<<< ours\nx\n||||||| base\nb1\n=======\ny\n>>>>>>> theirs\nmid\n\
                      <<<<<<< ours\np\n||||||| base\nb2\n=======\nq\n>>>>>>> theirs\n\
                      <<<<<<< ours\nx\n||||||| base\nb3\n=======\ny\n>>>>>>> theirs\n";
        let found = blocks_with_bases(text, merged);
        let bases: Vec<_> = found.iter().map(|b| b.base.as_deref()).collect();
        assert_eq!(bases, [Some("b1\r\n"), None, Some("b3\r\n")]);
        assert!(found.iter().all(|b| !b.base_in_file));
    }

    #[test]
    fn a_block_git_moved_the_lines_both_sides_share_out_of_takes_its_base() {
        // Verifies: ADR-0031, G-15
        // `git merge-file` of a,orig,z with ours a,same,A,z and theirs a,same,B,z.
        let text = "a\nsame\n<<<<<<< o\nA\n=======\nB\n>>>>>>> t\nz\n";
        let merged = "a\n<<<<<<< o\nsame\nA\n||||||| b\norig\n=======\nsame\nB\n>>>>>>> t\nz\n";
        assert_eq!(
            blocks_with_bases(text, merged)[0].base.as_deref(),
            Some("orig\n")
        );
        // And the lines both share at its end.
        let text = "a\n<<<<<<< o\nA\n=======\nB\n>>>>>>> t\nsame\nz\n";
        let merged = "a\n<<<<<<< o\nA\nsame\n||||||| b\norig\n=======\nB\nsame\n>>>>>>> t\nz\n";
        assert_eq!(
            blocks_with_bases(text, merged)[0].base.as_deref(),
            Some("orig\n")
        );
    }

    #[test]
    fn a_block_git_joined_from_conflicts_close_together_takes_their_bases_and_the_lines_between() {
        // Verifies: ADR-0031, G-15
        let text = "a\n<<<<<<< o\no1\nmid\no2\n=======\nt1\nmid\nt2\n>>>>>>> t\nz\n\
                    <<<<<<< o\nx\n=======\ny\n>>>>>>> t\n";
        let merged = "a\n<<<<<<< o\no1\n||||||| b\nb1\n=======\nt1\n>>>>>>> t\nmid\n\
                      <<<<<<< o\no2\n||||||| b\nb2\n=======\nt2\n>>>>>>> t\nz\n\
                      <<<<<<< o\nx\n||||||| b\nbx\n=======\ny\n>>>>>>> t\n";
        let found = blocks_with_bases(text, merged);
        assert_eq!(found[0].base.as_deref(), Some("b1\nmid\nb2\n"));
        assert_eq!(found[1].base.as_deref(), Some("bx\n"), "the run after it");
    }

    #[test]
    fn a_merged_block_whose_side_holds_marker_lines_is_read_without_a_panic() {
        // Verifies: ADR-0031, G-15
        // Ours holds `=======` and `>>>>>>> q`: the merge reads as a block without a base,
        // which no run may hold.
        let merged = "a\n<<<<<<< o\n=======\n>>>>>>> q\n||||||| b\nx\n=======\ny\n>>>>>>> t\nz\n";
        let text = "a\n<<<<<<< o\nA\n=======\nB\n>>>>>>> t\nz\n";
        assert_eq!(blocks_with_bases(text, merged)[0].base, None);
    }

    #[test]
    fn a_run_of_three_blocks_in_crlf_takes_their_bases_in_the_file_s_line_endings() {
        // Verifies: ADR-0031, G-15
        let text = "<<<<<<< o\r\no1\r\nm\r\no2\r\nn\r\no3\r\n=======\r\n\
                    t1\r\nm\r\nt2\r\nn\r\nt3\r\n>>>>>>> t\r\n";
        let merged = "<<<<<<< o\no1\n||||||| b\nb1\n=======\nt1\n>>>>>>> t\nm\n\
                      <<<<<<< o\no2\n||||||| b\nb2\n=======\nt2\n>>>>>>> t\nn\n\
                      <<<<<<< o\no3\n||||||| b\nb3\n=======\nt3\n>>>>>>> t\n";
        assert_eq!(
            blocks_with_bases(text, merged)[0].base.as_deref(),
            Some("b1\r\nm\r\nb2\r\nn\r\nb3\r\n")
        );
    }

    #[test]
    fn a_run_that_holds_more_than_the_block_or_other_sides_gives_it_no_base() {
        // Verifies: ADR-0031, G-15
        let text = "<<<<<<< o\nA\n=======\nB\n>>>>>>> t\n";
        // The shared lines differ, before and after.
        for merged in [
            "<<<<<<< o\nsame\nA\n||||||| b\norig\n=======\nother\nB\n>>>>>>> t\n",
            "<<<<<<< o\nA\nsame\n||||||| b\norig\n=======\nB\nother\n>>>>>>> t\n",
            "<<<<<<< o\nC\n||||||| b\norig\n=======\nB\n>>>>>>> t\n",
        ] {
            assert_eq!(blocks_with_bases(text, merged)[0].base, None, "{merged}");
        }
    }

    #[test]
    fn a_base_of_the_file_s_own_is_kept() {
        // Verifies: ADR-0031, G-15
        let text = "<<<<<<< HEAD\nx\n||||||| b\nmine\n=======\ny\n>>>>>>> o\n";
        let merged = "<<<<<<< ours\nx\n||||||| base\nother\n=======\ny\n>>>>>>> theirs\n";
        let found = blocks_with_bases(text, merged);
        assert_eq!(found[0].base.as_deref(), Some("mine\n"));
        assert!(found[0].base_in_file);
    }

    #[test]
    fn crlf_lines_are_read_and_written_back_as_the_file_had_them() {
        // Verifies: ADR-0031, G-15
        let opened = "a\r\n<<<<<<< HEAD\r\nx\r\n=======\r\ny\r\n>>>>>>> o\r\n";
        assert_eq!(blocks(opened)[0].ours, "x\r\n");
        assert_eq!(with_line_endings(opened, "a\nx\n"), "a\r\nx\r\n");
        assert_eq!(with_line_endings("a\nb\n", "a\nx\n"), "a\nx\n");
    }

    #[test]
    fn a_marker_line_is_one_git_writes_and_no_line_that_only_begins_like_one() {
        // Verifies: ADR-0031, G-15
        for line in [
            "<<<<<<< HEAD",
            "=======",
            ">>>>>>> o",
            "||||||| b",
            "<<<<<<<",
            "=======\r",
        ] {
            assert!(is_marker(line), "{line:?}");
        }
        for line in ["<<<<<<<<", "========", "<<<<<<<x", " =======", "== ="] {
            assert!(!is_marker(line), "{line:?}");
        }
        assert!(has_marker(MERGE));
        assert!(!has_marker("a\nours\nz\n"));
    }

    #[test]
    fn only_utf8_without_a_nul_byte_is_text() {
        // Verifies: ADR-0031, G-15
        assert_eq!(as_text(b"a\n"), Some("a\n"));
        assert_eq!(as_text(b"a\0b"), None);
        assert_eq!(as_text(&[0xff, 0xfe]), None);
    }
}
