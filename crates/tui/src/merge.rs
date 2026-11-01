//! The terminal application's merge editor (ADR-0031, G-15, G-18): a conflicted file's text in
//! `tui-textarea-2`, editable everywhere with undo, its blocks found by the core in the text as
//! it is now. Alt-o, Alt-t, and Alt-b take ours, theirs, or both for the block under the cursor,
//! Alt-n and Alt-p move between blocks, Alt-a opens and closes the block's base, Ctrl-S saves,
//! and Esc leaves, asking first where the result is not saved (Q-7). Free of terminal I/O, as
//! [`App`](crate::app::App) is, so that tests drive it.
//!
//! An 80-column terminal holds one column, not three parts a block (ADR-0031): a base the file
//! holds between `|||||||` and `=======` is left out of the text the editor opens, which no
//! resolution keeps, and shown with a base found in memory beneath the text while it is open.

use ocelli_core::merge::{MergeBlock, MergeFile, blocks};
use ocelli_core::model::Conflict;
use ocelli_core::port::Write;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tui_textarea::{CursorMove, TextArea};

/// What taking a side replaces a block by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Ours,
    Theirs,
    Both,
}

/// What a line of a block is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Marker,
    Ours,
    Theirs,
}

/// What a key in the editor asks of the application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The editor stays open.
    Stay,
    /// The result is to be saved by this write; the editor stays open until it is answered.
    Save(Write),
    /// The editor closes, its result saved or discarded once the developer said so.
    Leave,
}

/// A block as opened: its ours and theirs, by which it is found again, and its base where it
/// has one, each in `\n` line endings.
struct Opened {
    ours: String,
    theirs: String,
    base: Option<String>,
}

pub struct MergeEditor {
    pub path: String,
    conflict: Conflict,
    /// The blob id of the working directory's file opened, which saving expects to find again.
    blob: String,
    /// The text as opened, its base sections left out: a result that differs is not saved.
    opened: String,
    /// The blocks as opened, in their order, with their bases.
    bases: Vec<Opened>,
    pub area: TextArea<'static>,
    /// Whether the base of the block under the cursor is shown.
    pub base_open: bool,
    /// Why the last save was refused, or what a key could not do, until the next key.
    pub said: Option<String>,
    /// Whether Esc asked to leave a result not saved, the answer awaited.
    pub leaving: bool,
    /// Whether a save runs: the text is not edited meanwhile, so no edit is lost when it ends.
    pub saving: bool,
    /// The text before and after each side taken, and the edits it took. `tui-textarea-2`
    /// records a take as the block cut and, but for an empty side, the side put in; undo and
    /// redo step over all of them where the text is one a take left, or the one it started from.
    takes: Vec<Take>,
}

/// The number of lines of a side's text, each ending in a line ending.
fn count(text: &str) -> usize {
    text.matches('\n').count()
}

/// A side taken, as undo and redo find it again.
struct Take {
    before: String,
    after: String,
    edits: usize,
}

fn lf(text: &str) -> String {
    text.replace("\r\n", "\n")
}

/// `text`'s lines without the base sections of `found`, the file's blocks.
fn without_bases(text: &str, found: &[MergeBlock]) -> Vec<String> {
    let mut lines: Vec<String> = text.split('\n').map(str::to_string).collect();
    // From the last block up, so that the lines of the blocks above stay where they are.
    for block in found.iter().rev().filter(|b| b.base_in_file) {
        let bar = block.first_line + count(&block.ours);
        let section = 1 + block.base.as_deref().map_or(0, count);
        lines.drain(bar..bar + section);
    }
    lines
}

impl MergeEditor {
    /// The editor on `file`, the conflict `conflict` as the stopped operation showed it.
    pub fn open(conflict: Conflict, file: MergeFile) -> Self {
        let text = lf(&file.text);
        let lines = without_bases(&text, &file.blocks);
        let bases = file
            .blocks
            .iter()
            .map(|b| Opened {
                ours: lf(&b.ours),
                theirs: lf(&b.theirs),
                base: b.base.as_deref().map(lf),
            })
            .collect();
        let mut area = TextArea::new(lines);
        // A tab goes in as a tab, as the file may need it to, a Makefile's.
        area.set_hard_tab_indent(true);
        area.move_cursor(CursorMove::Top);
        let mut editor = Self {
            path: conflict.path.clone(),
            conflict,
            blob: file.blob,
            opened: String::new(),
            bases,
            area,
            base_open: false,
            said: None,
            leaving: false,
            saving: false,
            takes: Vec::new(),
        };
        editor.opened = editor.text();
        if let Some(first) = editor.blocks().first() {
            editor.jump(first.first_line - 1);
        }
        editor
    }

    /// The text as it is now, in `\n` line endings.
    pub fn text(&self) -> String {
        self.area.lines().join("\n")
    }

    /// Whether leaving now discards a result not saved.
    pub fn unsaved(&self) -> bool {
        self.text() != self.opened
    }

    /// The blocks of the text as it is now, as the core finds them.
    pub fn blocks(&self) -> Vec<MergeBlock> {
        blocks(&self.text())
    }

    /// What each line of the blocks is now, by row counted from 0.
    pub fn line_kinds(&self) -> Vec<(usize, LineKind)> {
        let mut kinds = Vec::new();
        for block in self.blocks() {
            let between = block.first_line + count(&block.ours);
            for row in block.first_line - 1..block.last_line {
                let kind = if row + 1 == block.first_line
                    || row + 1 == block.last_line
                    || row == between
                {
                    LineKind::Marker
                } else if row < between {
                    LineKind::Ours
                } else {
                    LineKind::Theirs
                };
                kinds.push((row, kind));
            }
        }
        kinds
    }

    /// The block under the cursor.
    fn block_here(&self) -> Option<MergeBlock> {
        let row = self.area.cursor().0;
        self.blocks()
            .into_iter()
            .find(|b| b.first_line - 1 <= row && row < b.last_line)
    }

    /// The base of the block under the cursor: that of the block in its place as opened while
    /// no block was taken or made, its sides edited or not, else that of the block opened with
    /// its ours and theirs.
    pub fn base_here(&self) -> Option<&str> {
        let row = self.area.cursor().0;
        let found = self.blocks();
        let index = found
            .iter()
            .position(|b| b.first_line - 1 <= row && row < b.last_line)?;
        let opened = if found.len() == self.bases.len() {
            self.bases.get(index)
        } else {
            let (ours, theirs) = (lf(&found[index].ours), lf(&found[index].theirs));
            self.bases
                .iter()
                .find(|b| b.ours == ours && b.theirs == theirs)
        };
        opened?.base.as_deref()
    }

    /// Moves the cursor to the start of `row`. A jump reaches row 65,535 at most; the rows past
    /// it are walked.
    fn jump(&mut self, row: usize) {
        let near = u16::try_from(row).unwrap_or(u16::MAX);
        self.area.move_cursor(CursorMove::Jump(near, 0));
        while self.area.cursor().0 < row {
            self.area.move_cursor(CursorMove::Down);
        }
        self.area.move_cursor(CursorMove::Head);
    }

    /// Replaces the block under the cursor, its markers included, by `side` as it holds it now,
    /// one change that undo puts back.
    fn take(&mut self, side: Side) {
        let Some(block) = self.block_here() else {
            self.said = Some("The cursor is in no block".into());
            return;
        };
        let mut insert = lf(&match side {
            Side::Ours => block.ours,
            Side::Theirs => block.theirs,
            Side::Both => block.ours + &block.theirs,
        });
        let before = self.text();
        let last = block.last_line - 1;
        self.jump(block.first_line - 1);
        self.area.start_selection();
        if last + 1 < self.area.lines().len() {
            self.jump(last + 1);
        } else {
            self.area.move_cursor(CursorMove::Bottom);
            self.area.move_cursor(CursorMove::End);
            insert.pop();
        }
        let edits = if insert.is_empty() { 1 } else { 2 };
        self.area.insert_str(insert);
        self.area.cancel_selection();
        self.takes.push(Take {
            before,
            after: self.text(),
            edits,
        });
    }

    /// Ctrl-U: undoes the last edit, a side taken whole.
    fn undo(&mut self) {
        let text = self.text();
        let take = self.takes.iter().rev().find(|t| t.after == text);
        for _ in 0..take.map_or(1, |t| t.edits) {
            self.area.undo();
        }
    }

    /// Ctrl-R: redoes the edit undone last, a side taken whole.
    fn redo(&mut self) {
        let text = self.text();
        let take = self.takes.iter().rev().find(|t| t.before == text);
        for _ in 0..take.map_or(1, |t| t.edits) {
            self.area.redo();
        }
    }

    /// Moves the cursor to the first line of the next block, or the previous one.
    fn step(&mut self, forward: bool) {
        let row = self.area.cursor().0;
        let found = self.blocks();
        let start = self.block_here().map_or(row, |b| b.first_line - 1);
        let to = if forward {
            found.iter().find(|b| b.first_line - 1 > row)
        } else {
            found.iter().rev().find(|b| b.first_line - 1 < start)
        };
        match to {
            Some(block) => self.jump(block.first_line - 1),
            None => self.said = Some("No block that way".into()),
        }
    }

    /// The save of the result as the file's resolution, expecting the conflict and the file it
    /// was opened on.
    fn save(&self) -> Write {
        Write::SaveMerge {
            path: self.path.clone(),
            conflict: self.conflict.clone(),
            opened: self.blob.clone(),
            result: self.text(),
        }
    }

    /// Handles a key; what the application is to do with it.
    pub fn key(&mut self, key: KeyEvent) -> Outcome {
        if self.leaving {
            self.leaving = false;
            return if key.code == KeyCode::Char('y') {
                Outcome::Leave
            } else {
                Outcome::Stay
            };
        }
        self.said = None;
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        match key.code {
            // The answer to a save is awaited before anything else is done.
            _ if self.saving => self.said = Some("Saving…".into()),
            // Ctrl-C, which quits elsewhere, leaves as Esc does here: it asks first.
            KeyCode::Esc => return self.leave(),
            KeyCode::Char('c') if ctrl && !alt => return self.leave(),
            KeyCode::Char('s') if ctrl && !alt => return Outcome::Save(self.save()),
            KeyCode::Char('u') if ctrl => self.undo(),
            KeyCode::Char('r') if ctrl => self.redo(),
            KeyCode::Char('o') if alt => self.take(Side::Ours),
            KeyCode::Char('t') if alt => self.take(Side::Theirs),
            KeyCode::Char('b') if alt => self.take(Side::Both),
            KeyCode::Char('n') if alt => self.step(true),
            KeyCode::Char('p') if alt => self.step(false),
            KeyCode::Char('a') if alt => {
                if self.base_open || self.base_here().is_some() {
                    self.base_open = !self.base_open;
                } else {
                    self.said = Some("This block has no base".into());
                }
            }
            // A character AltGr types, which some terminals report as Ctrl+Alt (crossterm on
            // Windows), as `{`, `[`, and `@` are on many European layouts.
            KeyCode::Char(c) if ctrl && alt && !c.is_ascii_alphanumeric() => {
                self.area.insert_char(c);
            }
            _ => {
                self.area.input(key);
            }
        }
        Outcome::Stay
    }

    /// Esc: leaves at once where nothing is unsaved, else asks first.
    fn leave(&mut self) -> Outcome {
        if self.unsaved() {
            self.leaving = true;
            Outcome::Stay
        } else {
            Outcome::Leave
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocelli_core::merge::blocks_with_bases;

    const TEXT: &str = "a\n<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> side\nm\n\
                        <<<<<<< HEAD\no2\n||||||| base\nb2\n=======\nt2\n>>>>>>> side\nz\n";

    fn conflict() -> Conflict {
        Conflict {
            path: "f".into(),
            ancestor: Some("a".into()),
            ours: Some("o".into()),
            theirs: Some("t".into()),
        }
    }

    /// The editor on `TEXT`, the first block's base found in memory.
    fn editor() -> MergeEditor {
        let merged = "a\n<<<<<<< ours\nours\n||||||| base\nb1\n=======\ntheirs\n>>>>>>> theirs\n";
        let file = MergeFile {
            text: TEXT.into(),
            blob: "b10b".into(),
            blocks: blocks_with_bases(TEXT, merged),
        };
        MergeEditor::open(conflict(), file)
    }

    fn alt(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::ALT)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn plain(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    // Verifies: ADR-0031, G-15
    fn the_editor_opens_on_the_text_its_bases_aside_and_the_cursor_on_the_first_block() {
        let e = editor();
        assert_eq!(
            e.text(),
            "a\n<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> side\nm\n\
             <<<<<<< HEAD\no2\n=======\nt2\n>>>>>>> side\nz\n"
        );
        assert!(!e.unsaved());
        assert_eq!(e.area.cursor(), (1, 0));
        assert_eq!(
            e.line_kinds()[..5],
            [
                (1, LineKind::Marker),
                (2, LineKind::Ours),
                (3, LineKind::Marker),
                (4, LineKind::Theirs),
                (5, LineKind::Marker),
            ]
        );
    }

    #[test]
    // Verifies: ADR-0031, G-15
    fn alt_o_t_and_b_take_ours_theirs_or_both_and_undo_puts_the_block_back() {
        for (key, result) in [
            ('o', "a\nours\nm\n"),
            ('t', "a\ntheirs\nm\n"),
            ('b', "a\nours\ntheirs\nm\n"),
        ] {
            let mut e = editor();
            assert_eq!(e.key(alt(key)), Outcome::Stay);
            assert!(e.text().starts_with(result), "{key}: {}", e.text());
            assert_eq!(e.blocks().len(), 1);
            let taken = e.text();
            e.key(ctrl('u'));
            assert!(
                !e.unsaved(),
                "{key}: undo puts the block back: {}",
                e.text()
            );
            assert_eq!(e.blocks().len(), 2);
            e.key(ctrl('r'));
            assert_eq!(e.text(), taken, "{key}: redo takes it again");
        }
    }

    #[test]
    // Verifies: ADR-0031, G-15
    fn a_side_edited_is_taken_as_edited_and_the_last_block_of_a_file_without_a_final_line_ending() {
        let mut e = editor();
        e.key(plain(KeyCode::Down));
        e.key(plain(KeyCode::End));
        e.key(plain(KeyCode::Char('!')));
        e.key(alt('b'));
        assert!(
            e.text().starts_with("a\nours!\ntheirs\nm\n"),
            "{}",
            e.text()
        );
        // Undoing the take, then the typing: the file as opened.
        e.key(ctrl('u'));
        assert!(e.text().contains("ours!\n=======\n"), "{}", e.text());
        e.key(ctrl('u'));
        assert!(!e.unsaved(), "{}", e.text());

        let text = "a\n<<<<<<< HEAD\nx\n=======\ny\n>>>>>>> side";
        let file = MergeFile {
            text: text.into(),
            blob: "b".into(),
            blocks: blocks(text),
        };
        let mut e = MergeEditor::open(conflict(), file);
        e.key(alt('t'));
        assert_eq!(e.text(), "a\ny");
    }

    #[test]
    // Verifies: ADR-0031, G-15
    fn a_take_of_an_empty_side_is_undone_alone_and_an_earlier_one_stays() {
        let text =
            "a\n<<<<<<< H\nx\n=======\ny\n>>>>>>> s\nm\n<<<<<<< H\n=======\nq\n>>>>>>> s\nz\n";
        let file = MergeFile {
            text: text.into(),
            blob: "b".into(),
            blocks: blocks(text),
        };
        let mut e = MergeEditor::open(conflict(), file);
        e.key(alt('o'));
        let first = e.text();
        assert_eq!(first, "a\nx\nm\n<<<<<<< H\n=======\nq\n>>>>>>> s\nz\n");
        e.key(alt('n'));
        e.key(alt('o'));
        assert_eq!(e.text(), "a\nx\nm\nz\n", "ours is empty");
        e.key(ctrl('u'));
        assert_eq!(e.text(), first, "the empty take alone is undone");
        e.key(ctrl('u'));
        assert!(!e.unsaved(), "{}", e.text());
        e.key(ctrl('r'));
        assert_eq!(e.text(), first);
        e.key(ctrl('r'));
        assert_eq!(e.text(), "a\nx\nm\nz\n");
    }

    #[test]
    // Verifies: ADR-0031, G-15
    fn a_block_past_row_65535_is_taken_where_it_is() {
        let mut text: String = (0..70_000).map(|i| format!("l{i}\n")).collect();
        text.push_str("<<<<<<< H\nx\n=======\ny\n>>>>>>> s\nz\n");
        let file = MergeFile {
            text: text.clone(),
            blob: "b".into(),
            blocks: blocks(&text),
        };
        let mut e = MergeEditor::open(conflict(), file);
        assert_eq!(e.area.cursor(), (70_000, 0), "opened on the block");
        e.key(alt('t'));
        assert!(e.text().ends_with("l69999\ny\nz\n"));
        assert!(e.text().starts_with("l0\nl1\n"));
    }

    #[test]
    // Verifies: ADR-0029, G-18
    fn a_character_altgr_types_as_ctrl_alt_is_typed_and_a_tab_stays_a_tab() {
        let mut e = editor();
        e.key(plain(KeyCode::Home));
        for c in ['{', '@'] {
            e.key(KeyEvent::new(
                KeyCode::Char(c),
                KeyModifiers::CONTROL | KeyModifiers::ALT,
            ));
        }
        e.key(plain(KeyCode::Tab));
        assert!(e.text().contains("{@\t<<<<<<< HEAD\n"), "{}", e.text());
    }

    #[test]
    // Verifies: ADR-0031, G-15
    fn a_file_s_own_base_is_found_again_once_its_sides_are_edited() {
        let mut e = editor();
        e.key(alt('n'));
        e.key(plain(KeyCode::Down));
        e.key(plain(KeyCode::End));
        e.key(plain(KeyCode::Char('!')));
        assert!(e.text().contains("o2!\n"));
        assert_eq!(e.base_here(), Some("b2\n"));
    }

    #[test]
    // Verifies: ADR-0031, G-15
    fn alt_n_and_p_move_between_blocks_and_alt_a_opens_and_closes_the_base() {
        let mut e = editor();
        assert_eq!(e.base_here(), Some("b1\n"));
        e.key(alt('a'));
        assert!(e.base_open);
        e.key(alt('a'));
        assert!(!e.base_open);
        e.key(alt('n'));
        assert_eq!(e.area.cursor().0, 7);
        assert_eq!(
            e.base_here(),
            Some("b2\n"),
            "the file's own base, set aside"
        );
        e.key(alt('n'));
        assert_eq!(e.said.as_deref(), Some("No block that way"));
        e.key(plain(KeyCode::Down));
        e.key(alt('p'));
        assert_eq!(e.area.cursor().0, 1);
        e.key(plain(KeyCode::Up));
        e.key(alt('a'));
        assert_eq!(e.said.as_deref(), Some("This block has no base"));
    }

    #[test]
    // Verifies: ADR-0031, G-15
    fn ctrl_s_saves_the_result_expecting_the_conflict_and_the_file_opened() {
        let mut e = editor();
        e.key(alt('o'));
        let Outcome::Save(Write::SaveMerge {
            path,
            conflict: shown,
            opened,
            result,
        }) = e.key(ctrl('s'))
        else {
            panic!("no save");
        };
        assert_eq!((path.as_str(), opened.as_str()), ("f", "b10b"));
        assert_eq!(shown, conflict());
        assert!(result.starts_with("a\nours\nm\n"));
        e.saving = true;
        assert_eq!(e.key(ctrl('s')), Outcome::Stay, "one save at a time");
        let before = e.text();
        e.key(plain(KeyCode::Char('x')));
        assert_eq!(e.text(), before, "the text waits while a save runs");
        assert_eq!(
            e.key(plain(KeyCode::Esc)),
            Outcome::Stay,
            "and so does leaving"
        );
        assert!(!e.leaving);
        assert_eq!(e.said.as_deref(), Some("Saving…"));
    }

    #[test]
    // Verifies: ADR-0031, ADR-0033, Q-7
    fn esc_with_a_result_not_saved_asks_and_only_y_leaves() {
        let mut e = editor();
        assert_eq!(
            e.key(plain(KeyCode::Esc)),
            Outcome::Leave,
            "nothing unsaved"
        );
        let mut e = editor();
        e.key(alt('o'));
        assert_eq!(e.key(plain(KeyCode::Esc)), Outcome::Stay);
        assert!(e.leaving);
        assert_eq!(e.key(plain(KeyCode::Char('n'))), Outcome::Stay);
        assert!(!e.leaving, "the editor stays open");
        assert_eq!(e.key(ctrl('c')), Outcome::Stay);
        assert_eq!(e.key(plain(KeyCode::Char('y'))), Outcome::Leave);
    }
}
