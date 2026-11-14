//! The terminal application's merge editor (ADR-0024, G-15, G-18): a conflicted file's text in
//! `tui-textarea-2`, editable everywhere with undo. Its blocks are those the core found in the file
//! opened, each followed through every edit as the web interface follows its own: a block an edit
//! took a marker line from, or moved one out of order, is no block, and a run of marker lines typed
//! or pasted is none either. Alt-o, Alt-t, and Alt-b take ours, theirs, or both for the block under
//! the cursor, Alt-n and Alt-p move between blocks, Alt-a opens and closes the block's base, Ctrl-S
//! saves, and Esc leaves, asking first where the result is not saved (Q-7). Free of terminal I/O,
//! as [`App`](crate::app::App) is, so that tests drive it.
//!
//! An 80-column terminal holds one column, not three parts a block (ADR-0024): a base the file
//! holds between `|||||||` and `=======` is left out of the text the editor opens, which no
//! resolution keeps, and shown with a base found in memory beneath the text while it is open.

use ocelli_core::merge::{MergeBlock, MergeFile};
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

/// A block as the editor holds it from the file opened: where its three marker lines start in
/// the text as it is now, by byte, moved along with every edit, whether a side was taken for it,
/// and its base where it has one, in `\n` line endings.
#[derive(Clone)]
struct Held {
    markers: [usize; 3],
    taken: bool,
    base: Option<String>,
}

/// A block an edit left whole: its index as opened, and the rows of its markers, counted from 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Found {
    index: usize,
    rows: [usize; 3],
}

/// What each of a block's marker lines begins with, in their order.
const MARKERS: [&str; 3] = ["<<<<<<<", "=======", ">>>>>>>"];

/// The byte at which each line of `text` starts.
fn line_starts(text: &str) -> Vec<usize> {
    std::iter::once(0)
        .chain(text.match_indices('\n').map(|(i, _)| i + 1))
        .collect()
}

pub struct MergeEditor {
    pub path: String,
    conflict: Conflict,
    /// The blob id of the working directory's file opened, which saving expects to find again.
    blob: String,
    /// The text as opened, its base sections left out: a result that differs is not saved.
    opened: String,
    /// The blocks as opened, in their order, where they are in `last`.
    held: Vec<Held>,
    /// The text the blocks' markers were last moved along to.
    last: String,
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

/// A side taken, as undo and redo find it again, with the blocks before and after it.
struct Take {
    before: String,
    after: String,
    edits: usize,
    held_before: Vec<Held>,
    held_after: Vec<Held>,
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
        let opened = lines.join("\n");
        let starts = line_starts(&opened);
        // The rows of each block's markers, less the base sections set aside above it.
        let mut aside = 0;
        let held = file
            .blocks
            .iter()
            .map(|b| {
                let first = b.first_line - 1 - aside;
                let between = first + 1 + count(&b.ours);
                let last = between + 1 + count(&b.theirs);
                if b.base_in_file {
                    aside += 1 + b.base.as_deref().map_or(0, count);
                }
                Held {
                    markers: [starts[first], starts[between], starts[last]],
                    taken: false,
                    base: b.base.as_deref().map(lf),
                }
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
            opened: opened.clone(),
            held,
            last: opened,
            area,
            base_open: false,
            said: None,
            leaving: false,
            saving: false,
            takes: Vec::new(),
        };
        if let Some(first) = editor.blocks().first() {
            editor.jump(first.rows[0]);
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

    /// The cursor's place in the text, by byte.
    fn offset(&self) -> usize {
        let (row, col) = self.area.cursor();
        let lines = self.area.lines();
        let above: usize = lines[..row].iter().map(|line| line.len() + 1).sum();
        let line = &lines[row];
        above + line.char_indices().nth(col).map_or(line.len(), |(i, _)| i)
    }

    /// Moves the blocks' markers along with the edit that made the text as it is now: the one
    /// change between the span the two texts share at their start, up to `at` where the edit was
    /// made there, and the one they share at their end. Where text put in repeats what follows it,
    /// the change is so found where the developer typed it, not past the repeat. A marker before
    /// the change stays, one after it moves by what the change adds, one at the start of what it
    /// removes stays there, and one where it only puts text in, or inside or at the end of what it
    /// replaces, goes past what it puts in, as CodeMirror maps a position kept with what follows
    /// it in the web interface.
    fn follow(&mut self, at: Option<usize>) {
        let now = self.text();
        if now == self.last {
            return;
        }
        let (old, new) = (self.last.as_bytes(), now.as_bytes());
        let start = old.iter().zip(new).take_while(|(a, b)| a == b).count();
        let start = at.map_or(start, |at| start.min(at));
        let room = old.len().min(new.len()) - start;
        let end = old
            .iter()
            .rev()
            .zip(new.iter().rev())
            .take(room)
            .take_while(|(a, b)| a == b)
            .count();
        let (to, put) = (old.len() - end, new.len() - end - start);
        for held in &mut self.held {
            for marker in &mut held.markers {
                *marker = if *marker < start || (*marker == start && to > start) {
                    *marker
                } else if *marker > to {
                    *marker - to + start + put
                } else {
                    start + put
                };
            }
        }
        self.last = now;
    }

    /// The blocks an edit left whole, in their order: no side taken, and each marker still at the
    /// start of a line that begins as it did, below the one before it.
    fn blocks(&self) -> Vec<Found> {
        let starts = line_starts(&self.last);
        self.held
            .iter()
            .enumerate()
            .filter(|(_, held)| !held.taken)
            .filter_map(|(index, held)| {
                let mut rows = [0; 3];
                for (i, &marker) in held.markers.iter().enumerate() {
                    let row = starts.binary_search(&marker).ok()?;
                    if !self.last[marker..].starts_with(MARKERS[i]) || (i > 0 && row <= rows[i - 1])
                    {
                        return None;
                    }
                    rows[i] = row;
                }
                Some(Found { index, rows })
            })
            .collect()
    }

    /// What each line of the blocks is now, by row counted from 0.
    pub fn line_kinds(&self) -> Vec<(usize, LineKind)> {
        let mut kinds = Vec::new();
        for Found { rows, .. } in self.blocks() {
            for row in rows[0]..=rows[2] {
                let kind = if rows.contains(&row) {
                    LineKind::Marker
                } else if row < rows[1] {
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
    fn block_here(&self) -> Option<Found> {
        let row = self.area.cursor().0;
        self.blocks()
            .into_iter()
            .find(|b| b.rows[0] <= row && row <= b.rows[2])
    }

    /// The base of the block under the cursor.
    pub fn base_here(&self) -> Option<&str> {
        self.held[self.block_here()?.index].base.as_deref()
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
        let Some(Found { index, rows }) = self.block_here() else {
            self.said = Some("The cursor is in no block".into());
            return;
        };
        let lines = |from: usize, to: usize| -> String {
            self.area.lines()[from..to]
                .iter()
                .map(|line| format!("{line}\n"))
                .collect()
        };
        let (ours, theirs) = (lines(rows[0] + 1, rows[1]), lines(rows[1] + 1, rows[2]));
        let mut insert = match side {
            Side::Ours => ours,
            Side::Theirs => theirs,
            Side::Both => ours + &theirs,
        };
        let before = self.text();
        let held_before = self.held.clone();
        let last = rows[2];
        self.jump(rows[0]);
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
        self.follow(None);
        self.held[index].taken = true;
        self.takes.push(Take {
            before,
            after: self.text(),
            edits,
            held_before,
            held_after: self.held.clone(),
        });
    }

    /// Ctrl-U: undoes the last edit, a side taken whole.
    fn undo(&mut self) {
        let text = self.text();
        let take = self.takes.iter().rev().find(|t| t.after == text);
        let (edits, back) = take.map_or((1, None), |t| {
            (t.edits, Some((t.before.clone(), t.held_before.clone())))
        });
        for _ in 0..edits {
            self.area.undo();
        }
        self.restore(back);
    }

    /// Ctrl-R: redoes the edit undone last, a side taken whole.
    fn redo(&mut self) {
        let text = self.text();
        let take = self.takes.iter().rev().find(|t| t.before == text);
        let (edits, back) = take.map_or((1, None), |t| {
            (t.edits, Some((t.after.clone(), t.held_after.clone())))
        });
        for _ in 0..edits {
            self.area.redo();
        }
        self.restore(back);
    }

    /// The blocks as a side taken left them, where undo or redo made the text that take left or
    /// started from; else moved along with the edit undone or redone. The take is found by its
    /// text, which an edit since and its own undo can make again without the take being next.
    fn restore(&mut self, back: Option<(String, Vec<Held>)>) {
        match back {
            Some((text, held)) if text == self.text() => {
                self.held = held;
                self.last = text;
            }
            _ => self.follow(None),
        }
    }

    /// Moves the cursor to the first line of the next block, or the previous one.
    fn step(&mut self, forward: bool) {
        let row = self.area.cursor().0;
        let found = self.blocks();
        let start = self.block_here().map_or(row, |b| b.rows[0]);
        let to = if forward {
            found.iter().find(|b| b.rows[0] > row)
        } else {
            found.iter().rev().find(|b| b.rows[0] < start)
        };
        match to {
            Some(block) => self.jump(block.rows[0]),
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
        let before = self.offset();
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
        // An edit a key makes, it makes where the cursor was or is now.
        self.follow(Some(before.min(self.offset())));
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
    use ocelli_core::merge::{blocks, blocks_with_bases};

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
    // Verifies: ADR-0024, G-15
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
    // Verifies: ADR-0024, G-15
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
    // Verifies: ADR-0024, G-15
    fn a_take_s_blocks_come_back_only_with_the_text_it_left_or_started_from() {
        let mut e = editor();
        e.key(alt('o'));
        e.key(ctrl('u'));
        assert_eq!(e.blocks().len(), 2);
        // Typed and taken back: the text the take started from, its redo gone with the typing.
        e.key(plain(KeyCode::Char('x')));
        e.key(plain(KeyCode::Backspace));
        e.key(ctrl('r'));
        assert!(!e.unsaved(), "{}", e.text());
        assert_eq!(
            e.blocks().len(),
            2,
            "no block is taken that the text still holds"
        );
    }

    #[test]
    // Verifies: ADR-0024, G-15
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
    // Verifies: ADR-0024, G-15
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
    // Verifies: ADR-0024, G-15
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
    // Verifies: ADR-0030, G-18
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
    // Verifies: ADR-0024, G-15
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
    // Verifies: ADR-0024, G-15
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
    // Verifies: ADR-0024, G-15
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
    // Verifies: ADR-0024, ADR-0025, Q-7
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

    /// The cases of `crates/core/tests/fixtures/merge.json`, which the core writes and the web
    /// interface's `mergeBlocks.test.ts` reads too.
    fn fixture() -> serde_json::Value {
        serde_json::from_str(include_str!("../../core/tests/fixtures/merge.json"))
            .expect("the fixture is JSON")
    }

    /// A file of the fixture, as the core handed it over.
    fn file_of(file: &serde_json::Value) -> MergeFile {
        let text = |v: &serde_json::Value| v.as_str().expect("a text").to_string();
        let line = |v: &serde_json::Value| v.as_u64().expect("a line") as usize;
        MergeFile {
            text: text(&file["text"]),
            blob: text(&file["blob"]),
            blocks: file["blocks"]
                .as_array()
                .expect("blocks")
                .iter()
                .map(|b| MergeBlock {
                    first_line: line(&b["firstLine"]),
                    last_line: line(&b["lastLine"]),
                    ours: text(&b["ours"]),
                    theirs: text(&b["theirs"]),
                    base: b["base"].as_str().map(str::to_string),
                    base_in_file: b["baseInFile"].as_bool().expect("baseInFile"),
                })
                .collect(),
        }
    }

    #[test]
    // Verifies: ADR-0024, G-15
    fn a_side_taken_leaves_the_text_the_core_says() {
        for case in fixture()["takes"].as_array().expect("takes") {
            let mut e = MergeEditor::open(conflict(), file_of(&case["file"]));
            for _ in 0..case["block"].as_u64().expect("a block") {
                e.key(alt('n'));
            }
            let side = case["side"].as_str().expect("a side");
            e.key(alt(side.chars().next().expect("ours, theirs, or both")));
            assert_eq!(
                e.text(),
                case["result"].as_str().expect("a result"),
                "{side}"
            );
        }
    }

    #[test]
    // Verifies: ADR-0024, G-15
    fn a_run_of_marker_lines_typed_is_no_block_and_the_blocks_opened_stay_whole() {
        for case in fixture()["pasted"].as_array().expect("pasted") {
            let mut e = MergeEditor::open(conflict(), file_of(&case["file"]));
            e.jump(case["line"].as_u64().expect("a line") as usize - 1);
            for c in case["paste"].as_str().expect("a paste").chars() {
                e.key(plain(if c == '\n' {
                    KeyCode::Enter
                } else {
                    KeyCode::Char(c)
                }));
            }
            let drawn: Vec<u64> = e.blocks().iter().map(|b| b.rows[0] as u64 + 1).collect();
            let want: Vec<u64> = case["blocks"]
                .as_array()
                .expect("blocks")
                .iter()
                .map(|line| line.as_u64().expect("a line"))
                .collect();
            assert_eq!(drawn, want, "{}", e.text());
        }
    }
}
