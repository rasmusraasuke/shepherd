/// A place in the text: a line and the number of characters before the place on that line.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Default)]
pub struct Pos {
    pub line: usize,
    pub col: usize,
}

impl Pos {
    pub fn new(line: usize, col: usize) -> Self {
        Self { line, col }
    }
}

/// Where the cursor can be sent.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Move {
    Left,
    Right,
    /// Down by this many rows, up when negative.
    Rows(isize),
    WordLeft,
    WordRight,
    /// The first character of the line that isn't whitespace, or the very start when already there.
    LineStart,
    LineEnd,
    Start,
    End,
}

/// One change of the text, kept to be undone: `removed` was replaced by `inserted` at `start`.
struct Edit {
    start: Pos,
    removed: String,
    inserted: String,
    /// The cursor and the anchor before the change.
    selection: (Pos, Pos),
    /// The revisions of the text before and after the change.
    revisions: (u64, u64),
}

/// The text of a file being edited, with a cursor, a selection and the history of changes.
pub struct Buffer {
    /// Never empty; a text that ends with a line break has an empty last line.
    lines: Vec<String>,
    /// Whether the lines of the file end with "\r\n".
    crlf: bool,
    cursor: Pos,
    /// The other end of the selection; where the cursor is when nothing is selected.
    anchor: Pos,
    /// The column the cursor returns to when moving over shorter lines.
    goal_col: Option<usize>,
    undo: Vec<Edit>,
    redo: Vec<Edit>,
    /// Identifies the current text; it changes with every edit and comes back with undo.
    revision: u64,
    last_revision: u64,
    saved_revision: u64,
    /// Where typing continues the previous edit, so that a word is undone at once.
    typing_at: Option<Pos>,
}

fn chars(line: &str) -> usize {
    line.chars().count()
}

/// The byte offset of a column.
fn byte(line: &str, col: usize) -> usize {
    line.char_indices().nth(col).map_or(line.len(), |(index, _)| index)
}

/// Where a text inserted at `start` ends.
fn end_of(start: Pos, text: &str) -> Pos {
    match text.rsplit_once('\n') {
        Some((before, last)) => Pos::new(start.line + before.matches('\n').count() + 1, chars(last)),
        None => Pos::new(start.line, start.col + chars(text)),
    }
}

fn indent_of(line: &str) -> usize {
    line.chars().take_while(|c| *c == ' ' || *c == '\t').count()
}

#[derive(PartialEq)]
enum CharKind {
    Space,
    Word,
    Other,
}

fn kind(c: char) -> CharKind {
    if c.is_whitespace() {
        CharKind::Space
    } else if c.is_alphanumeric() || c == '_' {
        CharKind::Word
    } else {
        CharKind::Other
    }
}

impl Buffer {
    pub fn new(text: &str) -> Self {
        let crlf = text.contains("\r\n");
        let lines = text.split('\n').map(|line| line.strip_suffix('\r').unwrap_or(line).to_string()).collect();
        Self {
            lines,
            crlf,
            cursor: Pos::default(),
            anchor: Pos::default(),
            goal_col: None,
            undo: Vec::new(),
            redo: Vec::new(),
            revision: 0,
            last_revision: 0,
            saved_revision: 0,
            typing_at: None,
        }
    }

    pub fn text(&self) -> String {
        self.lines.join(if self.crlf { "\r\n" } else { "\n" })
    }

    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    pub fn cursor(&self) -> Pos {
        self.cursor
    }

    /// The start and the end of the selection, if anything is selected.
    pub fn selection(&self) -> Option<(Pos, Pos)> {
        (self.cursor != self.anchor).then(|| (self.cursor.min(self.anchor), self.cursor.max(self.anchor)))
    }

    pub fn selected_text(&self) -> Option<String> {
        self.selection().map(|(start, end)| self.slice(start, end))
    }

    /// Whether the text differs from what was saved last.
    pub fn is_dirty(&self) -> bool {
        self.revision != self.saved_revision
    }

    pub fn mark_saved(&mut self) {
        self.saved_revision = self.revision;
    }

    /// Identifies the current text; it changes with every edit.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    fn line_len(&self, line: usize) -> usize {
        chars(&self.lines[line])
    }

    fn last_pos(&self) -> Pos {
        Pos::new(self.lines.len() - 1, self.line_len(self.lines.len() - 1))
    }

    /// The nearest place that exists in the text.
    pub fn clamp(&self, pos: Pos) -> Pos {
        let line = pos.line.min(self.lines.len() - 1);
        Pos::new(line, pos.col.min(self.line_len(line)))
    }

    fn slice(&self, start: Pos, end: Pos) -> String {
        if start.line == end.line {
            let line = &self.lines[start.line];
            return line[byte(line, start.col)..byte(line, end.col)].to_string();
        }
        let (first, last) = (&self.lines[start.line], &self.lines[end.line]);
        let mut text = first[byte(first, start.col)..].to_string();
        for line in &self.lines[start.line + 1..end.line] {
            text.push('\n');
            text.push_str(line);
        }
        text.push('\n');
        text.push_str(&last[..byte(last, end.col)]);
        text
    }

    /// Replaces the text between two places without recording it. Returns what was there.
    fn splice(&mut self, start: Pos, end: Pos, text: &str) -> String {
        let removed = self.slice(start, end);
        let (first, last) = (&self.lines[start.line], &self.lines[end.line]);
        let replaced = format!("{}{text}{}", &first[..byte(first, start.col)], &last[byte(last, end.col)..]);
        self.lines.splice(start.line..=end.line, replaced.split('\n').map(str::to_string));
        removed
    }

    /// Replaces the text between two places, leaving the cursor after the new text.
    fn edit(&mut self, start: Pos, end: Pos, text: &str) {
        if start == end && text.is_empty() {
            return;
        }
        let selection = (self.cursor, self.anchor);
        let removed = self.splice(start, end, text);
        self.last_revision += 1;
        self.undo.push(Edit { start, removed, inserted: text.to_string(), selection, revisions: (self.revision, self.last_revision) });
        self.redo.clear();
        self.revision = self.last_revision;
        self.place_cursor(end_of(start, text));
    }

    fn place_cursor(&mut self, pos: Pos) {
        self.cursor = pos;
        self.anchor = pos;
        self.goal_col = None;
        self.typing_at = None;
    }

    pub fn undo(&mut self) {
        let Some(edit) = self.undo.pop() else { return };
        self.splice(edit.start, end_of(edit.start, &edit.inserted), &edit.removed);
        self.place_cursor(edit.selection.0);
        self.anchor = edit.selection.1;
        self.revision = edit.revisions.0;
        self.redo.push(edit);
    }

    pub fn redo(&mut self) {
        let Some(edit) = self.redo.pop() else { return };
        self.splice(edit.start, end_of(edit.start, &edit.removed), &edit.inserted);
        self.place_cursor(end_of(edit.start, &edit.inserted));
        self.revision = edit.revisions.1;
        self.undo.push(edit);
    }

    /// Replaces the selection with the text, or inserts it at the cursor.
    pub fn insert(&mut self, text: &str) {
        let (start, end) = self.selection().unwrap_or((self.cursor, self.cursor));
        self.edit(start, end, &text.replace("\r\n", "\n"));
    }

    /// Inserts what was typed. Characters typed one after another are undone together, a word at a time.
    pub fn type_text(&mut self, text: &str) {
        let continues = self.typing_at == Some(self.cursor) && self.selection().is_none() && !text.starts_with(char::is_whitespace);
        if continues && !self.undo.is_empty() {
            let cursor = self.cursor;
            self.splice(cursor, cursor, text);
            self.last_revision += 1;
            self.revision = self.last_revision;
            self.redo.clear();
            if let Some(edit) = self.undo.last_mut() {
                edit.inserted.push_str(text);
                edit.revisions.1 = self.revision;
            }
            self.place_cursor(end_of(cursor, text));
        } else {
            self.insert(text);
        }
        self.typing_at = Some(self.cursor);
    }

    /// Starts a new line with the indentation of the current one.
    pub fn newline(&mut self) {
        let line = &self.lines[self.selection().map_or(self.cursor, |(start, _)| start).line];
        let indent: String = line.chars().take(indent_of(line).min(self.cursor.col)).collect();
        self.insert(&format!("\n{indent}"));
    }

    /// Deletes the selection, or what is between the cursor and the place the move takes it to.
    pub fn delete(&mut self, to: Move) {
        let (selection, edits) = ((self.cursor, self.anchor), self.undo.len());
        if self.selection().is_none() {
            self.move_cursor(to, true);
        }
        self.insert("");
        // undoing puts the cursor back where it was, with nothing selected
        if let Some(edit) = self.undo.get_mut(edits) {
            edit.selection = selection;
        }
    }

    pub fn set_cursor(&mut self, pos: Pos, extend: bool) {
        let anchor = self.anchor;
        self.place_cursor(self.clamp(pos));
        if extend {
            self.anchor = anchor;
        }
    }

    pub fn select(&mut self, start: Pos, end: Pos) {
        self.set_cursor(start, false);
        self.set_cursor(end, true);
    }

    pub fn select_all(&mut self) {
        self.select(Pos::default(), self.last_pos());
    }

    /// Selects the word, or the run of other characters, the place is in.
    pub fn select_word(&mut self, pos: Pos) {
        let pos = self.clamp(pos);
        let line: Vec<char> = self.lines[pos.line].chars().collect();
        let Some(at) = line.get(pos.col).or(line.last()).map(|c| kind(*c)) else { return };
        let col = pos.col.min(line.len() - 1);
        let start = line[..col].iter().rposition(|c| kind(*c) != at).map_or(0, |index| index + 1);
        let end = line[col..].iter().position(|c| kind(*c) != at).map_or(line.len(), |index| col + index);
        self.select(Pos::new(pos.line, start), Pos::new(pos.line, end));
    }

    /// Selects the whole line with its line break.
    pub fn select_line(&mut self, line: usize) {
        let line = line.min(self.lines.len() - 1);
        let end = if line + 1 < self.lines.len() { Pos::new(line + 1, 0) } else { self.last_pos() };
        self.select(Pos::new(line, 0), end);
    }

    /// Moves the cursor; the selection is extended to it or dropped.
    pub fn move_cursor(&mut self, to: Move, extend: bool) {
        let cursor = self.cursor;
        let goal_col = self.goal_col;
        let pos = match (to, self.selection()) {
            // leaving a selection puts the cursor at its end in the direction of the move
            (Move::Left, Some((start, _))) if !extend => start,
            (Move::Right, Some((_, end))) if !extend => end,
            (Move::Left, _) if cursor.col > 0 => Pos::new(cursor.line, cursor.col - 1),
            (Move::Left, _) if cursor.line > 0 => Pos::new(cursor.line - 1, self.line_len(cursor.line - 1)),
            (Move::Right, _) if cursor.col < self.line_len(cursor.line) => Pos::new(cursor.line, cursor.col + 1),
            (Move::Right, _) if cursor.line + 1 < self.lines.len() => Pos::new(cursor.line + 1, 0),
            (Move::Left | Move::Right, _) => cursor,
            (Move::Rows(rows), _) => match cursor.line.checked_add_signed(rows) {
                None => Pos::default(),
                Some(line) if line >= self.lines.len() => self.last_pos(),
                Some(line) => Pos::new(line, goal_col.unwrap_or(cursor.col)),
            },
            (Move::WordLeft, _) => self.word_left(cursor),
            (Move::WordRight, _) => self.word_right(cursor),
            (Move::LineStart, _) => {
                let indent = indent_of(&self.lines[cursor.line]);
                Pos::new(cursor.line, if cursor.col == indent { 0 } else { indent })
            }
            (Move::LineEnd, _) => Pos::new(cursor.line, self.line_len(cursor.line)),
            (Move::Start, _) => Pos::default(),
            (Move::End, _) => self.last_pos(),
        };
        self.set_cursor(pos, extend);
        if matches!(to, Move::Rows(_)) {
            self.goal_col = Some(goal_col.unwrap_or(cursor.col));
        }
    }

    fn word_left(&self, from: Pos) -> Pos {
        if from.col == 0 {
            return if from.line > 0 { Pos::new(from.line - 1, self.line_len(from.line - 1)) } else { from };
        }
        let line: Vec<char> = self.lines[from.line].chars().take(from.col).collect();
        let end = line.iter().rposition(|c| kind(*c) != CharKind::Space).map_or(0, |index| index + 1);
        let start = match end.checked_sub(1) {
            Some(last) => line[..last].iter().rposition(|c| kind(*c) != kind(line[last])).map_or(0, |index| index + 1),
            None => 0,
        };
        Pos::new(from.line, start)
    }

    fn word_right(&self, from: Pos) -> Pos {
        let line: Vec<char> = self.lines[from.line].chars().collect();
        if from.col >= line.len() {
            return if from.line + 1 < self.lines.len() { Pos::new(from.line + 1, 0) } else { from };
        }
        let rest = &line[from.col..];
        let start = rest.iter().position(|c| kind(*c) != CharKind::Space).unwrap_or(rest.len());
        let end = match rest.get(start) {
            Some(first) => rest[start..].iter().position(|c| kind(*c) != kind(*first)).map_or(rest.len(), |index| start + index),
            None => rest.len(),
        };
        Pos::new(from.line, from.col + end)
    }

    /// What one step of indentation is in this file.
    fn indent_unit(&self) -> &'static str {
        if self.lines.iter().any(|line| line.starts_with('\t')) { "\t" } else { "    " }
    }

    /// Indents the selected lines, or moves the rest of the line to the next step of indentation.
    pub fn indent(&mut self) {
        let unit = self.indent_unit();
        match self.selection().filter(|(start, end)| start.line != end.line) {
            Some(_) => self.change_lines(|line| if line.is_empty() { String::new() } else { format!("{unit}{line}") }),
            None if unit == "\t" => self.insert(unit),
            None => {
                let col = self.selection().map_or(self.cursor, |(start, _)| start).col;
                self.insert(&unit[col % unit.len()..]);
            }
        }
    }

    /// Takes one step of indentation off the selected lines, or the line of the cursor.
    pub fn outdent(&mut self) {
        let width = self.indent_unit().len();
        self.change_lines(|line| {
            let remove = if line.starts_with('\t') { 1 } else { line.chars().take(width).take_while(|c| *c == ' ').count() };
            line[remove..].to_string()
        });
    }

    /// Changes each line the selection touches, as one edit that keeps the selection on the same text.
    fn change_lines(&mut self, change: impl Fn(&str) -> String) {
        let (cursor, anchor) = (self.cursor, self.anchor);
        let (start, end) = (cursor.min(anchor), cursor.max(anchor));
        // a selection that ends at the start of a line doesn't include that line
        let last = if end.line > start.line && end.col == 0 { end.line - 1 } else { end.line };
        let changed: Vec<String> = self.lines[start.line..=last].iter().map(|line| change(line)).collect();
        let moved = |pos: Pos| match pos.line.checked_sub(start.line).and_then(|index| changed.get(index)) {
            Some(line) => Pos::new(pos.line, (pos.col + chars(line)).saturating_sub(self.line_len(pos.line))),
            None => pos,
        };
        let (cursor, anchor) = (moved(cursor), moved(anchor));
        self.edit(Pos::new(start.line, 0), Pos::new(last, self.line_len(last)), &changed.join("\n"));
        self.select(anchor, cursor);
    }

    /// Every place the query is found, in the order of the text. Letter case matters only
    /// when the query has capital letters.
    pub fn find(&self, query: &str) -> Vec<(Pos, Pos)> {
        let exact = query.chars().any(char::is_uppercase);
        let same = |a: &char, b: &char| if exact { a == b } else { a.to_lowercase().eq(b.to_lowercase()) };
        let query: Vec<char> = query.chars().collect();
        let mut found = Vec::new();
        for (index, line) in self.lines.iter().enumerate().filter(|(_, line)| !query.is_empty() && line.len() >= query.len()) {
            let line: Vec<char> = line.chars().collect();
            let mut col = 0;
            while col + query.len() <= line.len() {
                if line[col..col + query.len()].iter().zip(&query).all(|(a, b)| same(a, b)) {
                    found.push((Pos::new(index, col), Pos::new(index, col + query.len())));
                    col += query.len();
                } else {
                    col += 1;
                }
            }
        }
        found
    }

    /// Replaces everything the query finds, as one edit. Returns how many places were replaced.
    pub fn replace_all(&mut self, query: &str, replacement: &str) -> usize {
        let found = self.find(query);
        let (Some(first), Some(last)) = (found.first(), found.last()) else { return 0 };
        let mut text = String::new();
        for (index, (start, _)) in found.iter().enumerate() {
            if index > 0 {
                text.push_str(&self.slice(found[index - 1].1, *start));
            }
            text.push_str(replacement);
        }
        self.edit(first.0, last.1, &text);
        found.len()
    }

    /// Replaces the whole text, as it changed on disk, keeping the cursor where it was.
    pub fn set_text(&mut self, text: &str) {
        let cursor = self.cursor;
        let new = Buffer::new(text);
        self.crlf = new.crlf;
        self.edit(Pos::default(), self.last_pos(), &new.lines.join("\n"));
        self.set_cursor(cursor, false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A buffer with the cursor where the text has a `|`.
    fn buffer(text: &str) -> Buffer {
        let mut buffer = Buffer::new(&text.replace('|', ""));
        let before = text.split('|').next().unwrap();
        buffer.set_cursor(end_of(Pos::default(), before), false);
        buffer
    }

    /// The text with a `|` at the cursor and, when something is selected, a `^` at the other end.
    fn shown(buffer: &Buffer) -> String {
        let mut marks = vec![(buffer.cursor, '|')];
        if buffer.selection().is_some() {
            marks.push((buffer.anchor, '^'));
        }
        marks.sort();
        let mut lines = buffer.lines.clone();
        for (pos, mark) in marks.into_iter().rev() {
            let at = byte(&lines[pos.line], pos.col);
            lines[pos.line].insert(at, mark);
        }
        lines.join("\n")
    }

    #[test]
    fn types_and_deletes_at_the_cursor() {
        let mut buffer = buffer("fn |main() {}");
        buffer.type_text("x");
        assert_eq!(shown(&buffer), "fn x|main() {}");
        buffer.delete(Move::Left);
        buffer.delete(Move::Right);
        assert_eq!(shown(&buffer), "fn |ain() {}");
        buffer.delete(Move::WordRight);
        assert_eq!(shown(&buffer), "fn |() {}");
        buffer.delete(Move::WordLeft);
        assert_eq!(shown(&buffer), "|() {}");
    }

    #[test]
    fn deleting_at_the_edge_of_a_line_joins_it_with_its_neighbour() {
        let mut buffer = buffer("one\n|two\nthree");
        buffer.delete(Move::Left);
        assert_eq!(shown(&buffer), "one|two\nthree");
        buffer.move_cursor(Move::LineEnd, false);
        buffer.delete(Move::Right);
        assert_eq!(shown(&buffer), "onetwo|three");
    }

    #[test]
    fn typing_replaces_the_selection() {
        let mut buffer = buffer("one |two three");
        buffer.move_cursor(Move::WordRight, true);
        assert_eq!(shown(&buffer), "one ^two| three");
        assert_eq!(buffer.selected_text().unwrap(), "two");
        buffer.type_text("2");
        assert_eq!(shown(&buffer), "one 2| three");
    }

    #[test]
    fn inserts_and_removes_several_lines() {
        let mut buffer = buffer("a|d");
        buffer.insert("b\r\nc\n");
        assert_eq!(shown(&buffer), "ab\nc\n|d");
        buffer.select(Pos::new(0, 1), Pos::new(2, 0));
        assert_eq!(buffer.selected_text().unwrap(), "b\nc\n");
        buffer.insert("");
        assert_eq!(shown(&buffer), "a|d");
    }

    #[test]
    fn a_new_line_keeps_the_indentation() {
        let mut buffer = buffer("fn main() {\n    let x = 1;|\n}");
        buffer.newline();
        assert_eq!(shown(&buffer), "fn main() {\n    let x = 1;\n    |\n}");
        let mut buffer = self::buffer("  |  x");
        buffer.newline();
        assert_eq!(shown(&buffer), "  \n  |  x");
    }

    #[test]
    fn moves_by_characters_words_and_lines() {
        let mut buffer = buffer("let x = foo.bar();\n    second|");
        buffer.move_cursor(Move::LineStart, false);
        assert_eq!(shown(&buffer), "let x = foo.bar();\n    |second");
        buffer.move_cursor(Move::LineStart, false);
        assert_eq!(shown(&buffer), "let x = foo.bar();\n|    second");
        buffer.move_cursor(Move::Left, false);
        assert_eq!(shown(&buffer), "let x = foo.bar();|\n    second");
        for expected in ["let x = foo.bar|();", "let x = foo.|bar();", "let x = foo|.bar();", "let x = |foo.bar();"] {
            buffer.move_cursor(Move::WordLeft, false);
            assert_eq!(shown(&buffer), format!("{expected}\n    second"));
        }
        buffer.move_cursor(Move::WordRight, false);
        assert_eq!(shown(&buffer), "let x = foo|.bar();\n    second");
        buffer.move_cursor(Move::End, false);
        buffer.move_cursor(Move::Right, false);
        assert_eq!(shown(&buffer), "let x = foo.bar();\n    second|");
        buffer.move_cursor(Move::Start, true);
        assert_eq!(shown(&buffer), "|let x = foo.bar();\n    second^");
        buffer.move_cursor(Move::Right, false);
        assert_eq!(shown(&buffer), "let x = foo.bar();\n    second|");
    }

    #[test]
    fn moving_over_short_lines_returns_to_the_column() {
        let mut buffer = buffer("a long line|\nshort\n\nanother long line");
        buffer.move_cursor(Move::Rows(1), false);
        assert_eq!(buffer.cursor(), Pos::new(1, 5));
        buffer.move_cursor(Move::Rows(2), false);
        assert_eq!(buffer.cursor(), Pos::new(3, 11));
        buffer.move_cursor(Move::Rows(-10), false);
        assert_eq!(buffer.cursor(), Pos::new(0, 0));
        buffer.move_cursor(Move::Rows(10), true);
        assert_eq!(buffer.selected_text().unwrap(), buffer.text());
    }

    #[test]
    fn selects_words_and_lines() {
        let mut buffer = buffer("foo.bar_baz(1)\nnext");
        buffer.select_word(Pos::new(0, 6));
        assert_eq!(buffer.selected_text().unwrap(), "bar_baz");
        buffer.select_word(Pos::new(0, 3));
        assert_eq!(buffer.selected_text().unwrap(), ".");
        buffer.select_word(Pos::new(0, 14));
        assert_eq!(buffer.selected_text().unwrap(), ")");
        buffer.select_line(0);
        assert_eq!(buffer.selected_text().unwrap(), "foo.bar_baz(1)\n");
        buffer.select_line(1);
        assert_eq!(buffer.selected_text().unwrap(), "next");
        buffer.select_all();
        assert_eq!(buffer.selected_text().unwrap(), buffer.text());
    }

    #[test]
    fn undoes_and_redoes_edits_with_the_cursor() {
        let mut buffer = buffer("one |two");
        buffer.move_cursor(Move::WordRight, true);
        buffer.insert("2\n3");
        buffer.delete(Move::Left);
        assert_eq!(shown(&buffer), "one 2\n|");
        buffer.undo();
        assert_eq!(shown(&buffer), "one 2\n3|");
        buffer.undo();
        assert_eq!(shown(&buffer), "one ^two|");
        buffer.undo();
        assert_eq!(shown(&buffer), "one ^two|");
        buffer.redo();
        buffer.redo();
        assert_eq!(shown(&buffer), "one 2\n|");

        // a new edit drops what could be redone
        buffer.undo();
        buffer.type_text("x");
        buffer.redo();
        assert_eq!(shown(&buffer), "one 2\n3x|");
    }

    #[test]
    fn typing_is_undone_a_word_at_a_time() {
        let mut buffer = buffer("|");
        for c in "let it be".chars() {
            buffer.type_text(&c.to_string());
        }
        buffer.undo();
        assert_eq!(shown(&buffer), "let it|");
        buffer.undo();
        assert_eq!(shown(&buffer), "let|");
        buffer.redo();
        assert_eq!(shown(&buffer), "let it|");

        // moving the cursor away and back starts a new edit
        buffer.type_text("s");
        buffer.move_cursor(Move::Left, false);
        buffer.move_cursor(Move::Right, false);
        buffer.type_text("!");
        buffer.undo();
        assert_eq!(shown(&buffer), "let its|");
    }

    #[test]
    fn is_dirty_until_saved_or_undone_back_to_what_was_saved() {
        let mut buffer = buffer("|");
        assert!(!buffer.is_dirty());
        buffer.type_text("a");
        assert!(buffer.is_dirty());
        buffer.mark_saved();
        assert!(!buffer.is_dirty());
        buffer.type_text(" b");
        buffer.undo();
        assert!(!buffer.is_dirty());
        buffer.undo();
        assert!(buffer.is_dirty());
        buffer.redo();
        assert!(!buffer.is_dirty());
    }

    #[test]
    fn indents_and_outdents_the_selected_lines() {
        let mut buffer = buffer("fn main() {\n|let x = 1;\n\nlet y = 2;\n}");
        buffer.set_cursor(Pos::new(3, 3), true);
        buffer.indent();
        assert_eq!(shown(&buffer), "fn main() {\n    ^let x = 1;\n\n    let| y = 2;\n}");
        buffer.outdent();
        buffer.outdent();
        assert_eq!(shown(&buffer), "fn main() {\n^let x = 1;\n\nlet| y = 2;\n}");
        buffer.undo();
        buffer.undo();
        assert_eq!(shown(&buffer), "fn main() {\n    ^let x = 1;\n\n    let| y = 2;\n}");
    }

    #[test]
    fn tab_moves_to_the_next_step_of_indentation() {
        let mut buffer = buffer("ab|c");
        buffer.indent();
        assert_eq!(shown(&buffer), "ab  |c");
        buffer.outdent();
        assert_eq!(shown(&buffer), "ab  |c");

        let mut buffer = self::buffer("\tx\n|y");
        buffer.indent();
        assert_eq!(shown(&buffer), "\tx\n\t|y");
        buffer.outdent();
        assert_eq!(shown(&buffer), "\tx\n|y");
    }

    #[test]
    fn keeps_line_endings_and_the_last_line_break() {
        let buffer = Buffer::new("one\r\ntwo\r\n");
        assert_eq!(buffer.lines(), ["one", "two", ""]);
        assert_eq!(buffer.text(), "one\r\ntwo\r\n");
        assert_eq!(Buffer::new("one\ntwo").text(), "one\ntwo");
    }

    #[test]
    fn works_with_characters_of_several_bytes() {
        let mut buffer = buffer("õun |jääb");
        buffer.move_cursor(Move::WordRight, true);
        assert_eq!(buffer.selected_text().unwrap(), "jääb");
        buffer.type_text("ö");
        buffer.delete(Move::WordLeft);
        buffer.delete(Move::Left);
        assert_eq!(shown(&buffer), "õun|");
    }

    #[test]
    fn finds_text_ignoring_case_unless_the_query_has_capitals() {
        let buffer = buffer("Foo foo\nfoofoo õFOO");
        let found = |query| buffer.find(query).into_iter().map(|(start, end)| (start.line, start.col, end.col)).collect::<Vec<_>>();
        assert_eq!(found("foo"), [(0, 0, 3), (0, 4, 7), (1, 0, 3), (1, 3, 6), (1, 8, 11)]);
        assert_eq!(found("Foo"), [(0, 0, 3)]);
        assert_eq!(found("o f"), [(0, 2, 5)]);
        assert!(found("").is_empty());
        assert!(found("bar").is_empty());
    }

    #[test]
    fn replaces_everything_found_as_one_edit() {
        let mut buffer = buffer("let a = a + b;\nreturn a;|");
        assert_eq!(buffer.replace_all("a", "xy"), 3);
        assert_eq!(buffer.text(), "let xy = xy + b;\nreturn xy;");
        assert_eq!(buffer.replace_all("missing", "x"), 0);
        buffer.undo();
        assert_eq!(shown(&buffer), "let a = a + b;\nreturn a;|");
    }

    #[test]
    fn takes_the_text_changed_on_disk_keeping_the_cursor() {
        let mut buffer = buffer("one\ntw|o\nthree");
        buffer.set_text("zero\none\ntwo");
        assert_eq!(shown(&buffer), "zero\non|e\ntwo");
        buffer.set_text("x");
        assert_eq!(shown(&buffer), "x|");
        buffer.undo();
        assert_eq!(buffer.text(), "zero\none\ntwo");
    }
}
