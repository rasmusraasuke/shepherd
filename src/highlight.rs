use std::collections::HashSet;
use std::ops::Range;
use std::path::Path;
use std::sync::OnceLock;

use eframe::egui::text::{LayoutJob, TextFormat};
use eframe::egui::{Color32, FontId};
use tree_sitter::{Language, Parser, Query, QueryCursor, StreamingIterator, Tree};
use tree_sitter_language::LanguageFn;

use crate::buffer::Buffer;

/// Larger texts aren't highlighted, to keep typing in them responsive.
const MAX_SIZE: usize = 2 * 1024 * 1024;

/// A language the editor can highlight.
struct Definition {
    extensions: &'static [&'static str],
    language: LanguageFn,
    /// Highlight queries, joined in this order: when patterns capture the same node, the first one wins.
    queries: &'static [&'static str],
    /// Made on first use, as making the query takes a moment.
    grammar: OnceLock<Grammar>,
}

struct Grammar {
    language: Language,
    query: Query,
}

macro_rules! define {
    ($extensions:expr, $language:expr, $($query:expr),+) => {
        Definition { extensions: $extensions, language: $language, queries: &[$($query),+], grammar: OnceLock::new() }
    };
}

static DEFINITIONS: [Definition; 14] = [
    define!(&["kt", "kts"], tree_sitter_kotlin_ng::LANGUAGE, include_str!("../queries/kotlin.scm")),
    define!(&["java"], tree_sitter_java::LANGUAGE, tree_sitter_java::HIGHLIGHTS_QUERY),
    define!(&["rs"], tree_sitter_rust::LANGUAGE, tree_sitter_rust::HIGHLIGHTS_QUERY),
    define!(&["js", "mjs", "cjs", "jsx"], tree_sitter_javascript::LANGUAGE, tree_sitter_javascript::HIGHLIGHT_QUERY, tree_sitter_javascript::JSX_HIGHLIGHT_QUERY),
    define!(&["ts", "mts", "cts"], tree_sitter_typescript::LANGUAGE_TYPESCRIPT, tree_sitter_javascript::HIGHLIGHT_QUERY, tree_sitter_typescript::HIGHLIGHTS_QUERY),
    define!(
        &["tsx"],
        tree_sitter_typescript::LANGUAGE_TSX,
        tree_sitter_javascript::HIGHLIGHT_QUERY,
        tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
        tree_sitter_typescript::HIGHLIGHTS_QUERY
    ),
    define!(&["py"], tree_sitter_python::LANGUAGE, tree_sitter_python::HIGHLIGHTS_QUERY),
    define!(&["go"], tree_sitter_go::LANGUAGE, tree_sitter_go::HIGHLIGHTS_QUERY),
    define!(&["json"], tree_sitter_json::LANGUAGE, tree_sitter_json::HIGHLIGHTS_QUERY),
    define!(&["sh", "bash", "zsh"], tree_sitter_bash::LANGUAGE, tree_sitter_bash::HIGHLIGHT_QUERY),
    define!(&["toml"], tree_sitter_toml_ng::LANGUAGE, tree_sitter_toml_ng::HIGHLIGHTS_QUERY),
    define!(&["yml", "yaml"], tree_sitter_yaml::LANGUAGE, tree_sitter_yaml::HIGHLIGHTS_QUERY),
    define!(&["html", "htm"], tree_sitter_html::LANGUAGE, tree_sitter_html::HIGHLIGHTS_QUERY),
    define!(&["css"], tree_sitter_css::LANGUAGE, tree_sitter_css::HIGHLIGHTS_QUERY),
];

impl Definition {
    fn grammar(&self) -> &Grammar {
        self.grammar.get_or_init(|| {
            let language = Language::new(self.language);
            let query = Query::new(&language, &self.queries.concat()).unwrap_or_else(|e| panic!("highlight query for {:?}: {e}", self.extensions));
            Grammar { language, query }
        })
    }
}

/// The colors of the parts of the code, by the names highlight queries give them. A name like
/// "function.method" takes the color of "function" when it has none of its own.
const COLORS: &[(&str, [u8; 3], [u8; 3])] = &[
    // name, dark, light
    ("keyword", [0xCC, 0x78, 0x32], [0x00, 0x33, 0xB3]),
    ("label", [0xCC, 0x78, 0x32], [0x00, 0x33, 0xB3]),
    ("variable.builtin", [0xCC, 0x78, 0x32], [0x00, 0x33, 0xB3]),
    ("constant.builtin", [0xCC, 0x78, 0x32], [0x00, 0x33, 0xB3]),
    ("boolean", [0xCC, 0x78, 0x32], [0x00, 0x33, 0xB3]),
    ("punctuation.special", [0xCC, 0x78, 0x32], [0x00, 0x33, 0xB3]),
    ("string", [0x6A, 0x87, 0x59], [0x06, 0x7D, 0x17]),
    ("string.escape", [0xCC, 0x78, 0x32], [0x00, 0x37, 0xA6]),
    ("escape", [0xCC, 0x78, 0x32], [0x00, 0x37, 0xA6]),
    ("string.special.key", [0x98, 0x76, 0xAA], [0x87, 0x10, 0x94]),
    ("comment", [0x80, 0x80, 0x80], [0x8C, 0x8C, 0x8C]),
    ("number", [0x68, 0x97, 0xBB], [0x17, 0x50, 0xEB]),
    ("constant", [0x98, 0x76, 0xAA], [0x87, 0x10, 0x94]),
    ("property", [0x98, 0x76, 0xAA], [0x87, 0x10, 0x94]),
    ("function", [0xFF, 0xC6, 0x6D], [0x00, 0x62, 0x7A]),
    ("function.call", [0xA9, 0xB7, 0xC6], [0x00, 0x00, 0x00]),
    ("function.macro", [0x90, 0x8B, 0x25], [0x9E, 0x88, 0x0D]),
    ("type", [0x6F, 0xAF, 0xBD], [0x00, 0x80, 0x80]),
    ("constructor", [0x6F, 0xAF, 0xBD], [0x00, 0x80, 0x80]),
    ("attribute", [0xBB, 0xB5, 0x29], [0x9E, 0x88, 0x0D]),
    ("tag", [0xE8, 0xBF, 0x6A], [0x00, 0x33, 0xB3]),
];

/// The color of a highlight name, if it has one. `None` leaves the text in the color of plain code.
pub fn color(name: &str, dark: bool) -> Option<Color32> {
    let mut name = name;
    loop {
        if let Some((_, on_dark, on_light)) = COLORS.iter().find(|(known, _, _)| *known == name) {
            let [r, g, b] = if dark { *on_dark } else { *on_light };
            return Some(Color32::from_rgb(r, g, b));
        }
        name = name.rsplit_once('.')?.0;
    }
}

/// The parts of one line to color: byte ranges in the line and their highlight names.
pub type Spans = Vec<(Range<usize>, &'static str)>;

/// Keeps the syntax tree of a text up to date as it changes and finds the parts of it to color.
pub struct Highlighter {
    grammar: &'static Grammar,
    parser: Parser,
    tree: Option<Tree>,
    /// The text the tree is for: the lines joined by "\n", and where each line starts.
    text: String,
    line_starts: Vec<usize>,
    /// The spans found last and the lines they are for.
    found: Option<(Range<usize>, Vec<Spans>)>,
}

impl Highlighter {
    /// A highlighter for the language of the file, if it is one the editor knows.
    pub fn for_path(path: &Path) -> Option<Self> {
        let extension = path.extension()?.to_str()?;
        let definition = DEFINITIONS.iter().find(|definition| definition.extensions.contains(&extension))?;
        let grammar = definition.grammar();
        let mut parser = Parser::new();
        parser.set_language(&grammar.language).ok()?;
        Some(Self { grammar, parser, tree: None, text: String::new(), line_starts: Vec::new(), found: None })
    }

    /// Brings the tree up to date with the text of the buffer. Only what changed is parsed again.
    pub fn update(&mut self, buffer: &mut Buffer) {
        let changes = buffer.take_changes();
        if self.tree.is_some() && changes.is_empty() {
            return;
        }
        if let Some(tree) = &mut self.tree {
            for change in &changes {
                tree.edit(change);
            }
        }
        let lines = buffer.lines();
        self.text = lines.join("\n");
        self.line_starts = lines.iter().scan(0, |start, line| Some(std::mem::replace(start, *start + line.len() + 1))).collect();
        self.tree = if self.text.len() > MAX_SIZE { None } else { self.parser.parse(&self.text, self.tree.as_ref()) };
        self.found = None;
    }

    /// The parts to color on each line of a text that doesn't change, such as a side of a diff.
    pub fn spans_of(path: &Path, lines: &[String]) -> Vec<Spans> {
        let Some(mut highlighter) = Self::for_path(path) else { return vec![Vec::new(); lines.len()] };
        highlighter.update(&mut Buffer::new(&lines.join("\n")));
        highlighter.find(0..lines.len())
    }

    /// The parts to color on each of the given lines.
    pub fn spans(&mut self, lines: Range<usize>) -> &[Spans] {
        if self.found.as_ref().is_none_or(|(found_for, _)| *found_for != lines) {
            self.found = Some((lines.clone(), self.find(lines)));
        }
        &self.found.as_ref().unwrap().1
    }

    fn find(&self, lines: Range<usize>) -> Vec<Spans> {
        let Some(tree) = &self.tree else { return vec![Vec::new(); lines.len()] };
        let line_end = |line: usize| self.line_starts.get(line + 1).map_or(self.text.len(), |next| next - 1);
        let start = self.line_starts.get(lines.start).copied().unwrap_or(self.text.len());
        let end = lines.end.checked_sub(1).map_or(start, line_end).max(start);

        // the name for each byte; nodes inside others come later and paint over them
        let names = self.grammar.query.capture_names();
        let mut byte_names: Vec<Option<&'static str>> = vec![None; end - start];
        let mut painted = HashSet::new();
        let mut cursor = QueryCursor::new();
        cursor.set_byte_range(start..end);
        let mut captures = cursor.captures(&self.grammar.query, tree.root_node(), self.text.as_bytes());
        while let Some((found, index)) = captures.next() {
            let capture = found.captures()[*index];
            // the first pattern that captures a node decides its color
            if !painted.insert(capture.node.id()) {
                continue;
            }
            let range = capture.node.byte_range();
            let (from, to) = (range.start.clamp(start, end), range.end.clamp(start, end));
            byte_names[from - start..to - start].fill(Some(names[capture.index as usize]));
        }

        lines
            .map(|line| {
                let (line_start, line_end) = (self.line_starts[line], line_end(line));
                let mut spans: Spans = Vec::new();
                for (offset, name) in byte_names[line_start - start..line_end - start].iter().enumerate() {
                    let Some(name) = name else { continue };
                    match spans.last_mut() {
                        Some((range, last)) if range.end == offset && last == name => range.end += 1,
                        _ => spans.push((offset..offset + 1, name)),
                    }
                }
                spans
            })
            .collect()
    }
}

/// A line of code laid out in the colors of its spans, with the `marked` parts on a background
/// of the `mark` color.
pub fn layout_job(line: &str, spans: &[(Range<usize>, &str)], marked: &[Range<usize>], mark: Color32, font: &FontId, text_color: Color32, dark: bool) -> LayoutJob {
    let mut cuts: Vec<usize> = spans.iter().map(|(range, _)| range).chain(marked).flat_map(|range| [range.start, range.end]).chain([0, line.len()]).collect();
    cuts.sort_unstable();
    cuts.dedup();
    let mut job = LayoutJob::default();
    for cut in cuts.windows(2) {
        let range = cut[0]..cut[1];
        let color = spans.iter().find(|(span, _)| span.contains(&range.start)).and_then(|(_, name)| color(name, dark));
        let background = if marked.iter().any(|marked| marked.contains(&range.start)) { mark } else { Color32::TRANSPARENT };
        job.append(&line[range], 0.0, TextFormat { background, ..TextFormat::simple(font.clone(), color.unwrap_or(text_color)) });
    }
    job
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Pos;

    #[test]
    fn every_query_fits_its_grammar() {
        for definition in &DEFINITIONS {
            definition.grammar();
        }
    }

    /// The highlight name of each piece of the line, other than the ones without.
    fn names(highlighter: &mut Highlighter, lines: &[String], line: usize) -> Vec<(String, &'static str)> {
        let spans = highlighter.spans(line..line + 1)[0].clone();
        spans.into_iter().map(|(range, name)| (lines[line][range].to_string(), name)).collect()
    }

    fn highlighted(path: &str, text: &str) -> (Highlighter, Vec<String>) {
        let mut highlighter = Highlighter::for_path(Path::new(path)).unwrap();
        let mut buffer = Buffer::new(text);
        highlighter.update(&mut buffer);
        (highlighter, buffer.lines().to_vec())
    }

    #[test]
    fn highlights_kotlin() {
        let (mut highlighter, lines) = highlighted("Main.kt", "@Suppress(\"x\")\nfun main(args: Array<String>) {\n    val s = \"hi ${args.size}\\n\" // done\n    println(null)\n}");
        let line = |highlighter: &mut Highlighter, line| names(highlighter, &lines, line).into_iter().map(|(text, name)| format!("{text}={name}")).collect::<Vec<_>>();
        assert_eq!(line(&mut highlighter, 0), ["@Suppress=attribute", "\"x\"=string"]);
        assert_eq!(line(&mut highlighter, 1), ["fun=keyword", "main=function", "args=variable.parameter", "Array=type", "String=type"]);
        assert_eq!(line(&mut highlighter, 2), ["val=keyword", "\"hi =string", "${=punctuation.special", "args.=embedded", "size=property", "}=punctuation.special", "\\n=string.escape", "\"=string", "// done=comment"]);
        assert_eq!(line(&mut highlighter, 3), ["println=function.call", "null=constant.builtin"]);
    }

    #[test]
    fn highlights_other_languages() {
        let (mut highlighter, lines) = highlighted("main.rs", "fn main() { let x = 1; }");
        assert!(names(&mut highlighter, &lines, 0).contains(&("fn".to_string(), "keyword")));
        let (mut highlighter, lines) = highlighted("a.ts", "const a: number = 1;");
        assert!(names(&mut highlighter, &lines, 0).contains(&("const".to_string(), "keyword")));
        assert!(Highlighter::for_path(Path::new("notes.txt")).is_none());
        assert!(Highlighter::for_path(Path::new("Makefile")).is_none());
    }

    #[test]
    fn highlights_lines_inside_a_comment_that_started_above_them() {
        let (mut highlighter, lines) = highlighted("a.kt", "/*\none\ntwo\n*/\nval x = 1");
        assert_eq!(highlighter.spans(2..5).to_vec(), [vec![(0..3, "comment")], vec![(0..2, "comment")], vec![(0..3, "keyword"), (8..9, "number")]]);
        assert_eq!(lines.len(), 5);
    }

    #[test]
    fn follows_changes_of_the_text_like_parsing_it_anew() {
        let mut buffer = Buffer::new("fun main() {\n    println(\"hõla\")\n}\n");
        let mut highlighter = Highlighter::for_path(Path::new("a.kt")).unwrap();
        highlighter.update(&mut buffer);
        buffer.set_cursor(Pos::new(1, 13), false);
        buffer.type_text("ä\"); val s = \"");
        buffer.newline();
        buffer.insert("/* multi\nline */");
        buffer.select(Pos::new(0, 4), Pos::new(0, 8));
        buffer.type_text("start");
        buffer.undo();
        buffer.redo();
        buffer.set_text("// replaced\nfun other() = 1\n");
        buffer.undo();
        highlighter.update(&mut buffer);

        let (mut fresh, lines) = highlighted("a.kt", &buffer.text());
        let all = 0..lines.len();
        assert_eq!(highlighter.tree.as_ref().unwrap().root_node().to_sexp(), fresh.tree.as_ref().unwrap().root_node().to_sexp());
        assert_eq!(highlighter.spans(all.clone()).to_vec(), fresh.spans(all).to_vec());
    }

    #[test]
    fn colors_names_by_their_most_specific_known_part() {
        assert_eq!(color("function.call.generic", true), color("function.call", true));
        assert_eq!(color("function.method", false), color("function", false));
        assert_eq!(color("variable", true), None);
    }

    #[test]
    fn lays_out_spans_in_their_colors() {
        let font = FontId::monospace(12.0);
        let job = layout_job("val x = 1", &[(0..3, "keyword"), (8..9, "number")], &[], Color32::RED, &font, Color32::WHITE, true);
        let colors: Vec<(String, Color32)> = job.sections.iter().map(|section| (job.text[section.byte_range.start.0..section.byte_range.end.0].to_string(), section.format.color)).collect();
        assert_eq!(colors, [("val".to_string(), color("keyword", true).unwrap()), (" x = ".to_string(), Color32::WHITE), ("1".to_string(), color("number", true).unwrap())]);
    }

    #[test]
    fn lays_out_marked_parts_on_their_background() {
        let font = FontId::monospace(12.0);
        let job = layout_job("val x = 1", &[(0..3, "keyword")], &[2..5, 7..8], Color32::RED, &font, Color32::WHITE, true);
        let sections: Vec<(String, Color32, Color32)> =
            job.sections.iter().map(|section| (job.text[section.byte_range.start.0..section.byte_range.end.0].to_string(), section.format.color, section.format.background)).collect();
        let keyword = color("keyword", true).unwrap();
        let transparent = Color32::TRANSPARENT;
        assert_eq!(
            sections,
            [
                ("va".to_string(), keyword, transparent),
                ("l".to_string(), keyword, Color32::RED), (" x".to_string(), Color32::WHITE, Color32::RED),
                (" =".to_string(), Color32::WHITE, transparent),
                (" ".to_string(), Color32::WHITE, Color32::RED),
                ("1".to_string(), Color32::WHITE, transparent)
            ]
        );
    }

    #[test]
    fn highlights_a_text_as_a_whole() {
        let lines = ["/* a".to_string(), "b */ val x = 1".to_string()];
        assert_eq!(Highlighter::spans_of(Path::new("a.kt"), &lines), [vec![(0..4, "comment")], vec![(0..4, "comment"), (5..8, "keyword"), (13..14, "number")]]);
        assert_eq!(Highlighter::spans_of(Path::new("a.txt"), &lines), [vec![], vec![]]);
    }
}
