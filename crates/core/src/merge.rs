//! The merge editor's file (ADR-0031, G-15): a conflicted file's text as the working directory
//! holds it, and its blocks, found here once for every front end. A block runs from a `<<<<<<<`
//! line to its `>>>>>>>` line, ours before `=======` and theirs after it, its base the `|||||||`
//! section where the file holds one, as the `merge`, `diff3`, and `zdiff3` styles write them,
//! else the base of the block holding the same ours and theirs in a merge of the index's three
//! stages in memory in the `diff3` style.

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

/// The blocks of `text`, each without a base of its own given the base of the block of
/// `merged`, the index's three stages merged in memory in the `diff3` style, that holds the same
/// ours and theirs, in the line endings of `text`; a block that none holds keeps none. Blocks
/// alike take the merged ones in their order, each one once.
pub fn blocks_with_bases(text: &str, merged: &str) -> Vec<MergeBlock> {
    let lf = |s: &str| s.replace("\r\n", "\n");
    let mut candidates = blocks(merged).into_iter();
    let mut found = blocks(text);
    for block in found.iter_mut().filter(|b| b.base.is_none()) {
        let (ours, theirs) = (lf(&block.ours), lf(&block.theirs));
        let mut ahead = candidates.clone();
        if let Some(base) = ahead
            .find(|m| lf(&m.ours) == ours && lf(&m.theirs) == theirs)
            .and_then(|m| m.base)
        {
            block.base = Some(with_line_endings(text, &base));
            candidates = ahead;
        }
    }
    found
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
