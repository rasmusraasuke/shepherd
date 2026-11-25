use std::path::Path;

use pulldown_cmark::{CodeBlockKind, Event, Parser, Tag, TagEnd};

use crate::editor::highlight::{Highlighter, Spans};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Style {
    pub strong: bool,
    pub emphasis: bool,
    pub code: bool,
    pub link: Option<String>,
    pub heading: bool,
}

#[derive(Debug, PartialEq)]
pub enum Block {
    Text(Vec<(String, Style)>),
    Code(Vec<(String, Spans)>),
    Rule,
}

pub fn parse(path: &Path, text: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut runs: Vec<(String, Style)> = Vec::new();
    let mut style = Style::default();
    let mut lists: Vec<Option<u64>> = Vec::new();
    let mut code: Option<(String, String)> = None;
    let flush = |runs: &mut Vec<(String, Style)>, blocks: &mut Vec<Block>| {
        if runs.iter().any(|(text, _)| !text.trim().is_empty()) {
            blocks.push(Block::Text(std::mem::take(runs)));
        }
        runs.clear();
    };
    for event in Parser::new(text) {
        match event {
            Event::Start(Tag::CodeBlock(kind)) => {
                flush(&mut runs, &mut blocks);
                let language = match kind {
                    CodeBlockKind::Fenced(info) => {
                        info.split([' ', ',']).next().unwrap_or("").to_string()
                    }
                    CodeBlockKind::Indented => String::new(),
                };
                code = Some((language, String::new()));
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some((language, text)) = code.take() {
                    let lines: Vec<String> = text.lines().map(str::to_string).collect();
                    let spans = Highlighter::spans_in(&language, path, &lines);
                    blocks.push(Block::Code(lines.into_iter().zip(spans).collect()));
                }
            }
            Event::Text(text) if code.is_some() => {
                if let Some((_, code)) = code.as_mut() {
                    code.push_str(&text);
                }
            }
            Event::Start(Tag::Heading { .. }) => {
                flush(&mut runs, &mut blocks);
                style.heading = true;
            }
            Event::End(TagEnd::Heading(_)) => {
                flush(&mut runs, &mut blocks);
                style.heading = false;
            }
            Event::Start(Tag::List(first)) => {
                flush(&mut runs, &mut blocks);
                lists.push(first);
            }
            Event::End(TagEnd::List(_)) => {
                flush(&mut runs, &mut blocks);
                lists.pop();
            }
            Event::Start(Tag::Item) => {
                flush(&mut runs, &mut blocks);
                let indent = "  ".repeat(lists.len().saturating_sub(1));
                let marker = match lists.last_mut() {
                    Some(Some(number)) => {
                        *number += 1;
                        format!("{}.", *number - 1)
                    }
                    _ => "•".to_string(),
                };
                runs.push((format!("{indent}{marker} "), Style::default()));
            }
            Event::End(TagEnd::Paragraph | TagEnd::Item | TagEnd::BlockQuote(_)) => {
                flush(&mut runs, &mut blocks)
            }
            Event::Start(Tag::Paragraph) if lists.is_empty() => flush(&mut runs, &mut blocks),
            Event::Start(Tag::Emphasis) => style.emphasis = true,
            Event::End(TagEnd::Emphasis) => style.emphasis = false,
            Event::Start(Tag::Strong) => style.strong = true,
            Event::End(TagEnd::Strong) => style.strong = false,
            Event::Start(Tag::Link { dest_url, .. }) => style.link = Some(dest_url.to_string()),
            Event::End(TagEnd::Link) => style.link = None,
            Event::Text(text) | Event::Html(text) | Event::InlineHtml(text) => {
                runs.push((text.to_string(), style.clone()))
            }
            Event::Code(text) => runs.push((
                text.to_string(),
                Style {
                    code: true,
                    ..style.clone()
                },
            )),
            Event::SoftBreak => runs.push((" ".to_string(), style.clone())),
            Event::HardBreak => runs.push(("\n".to_string(), style.clone())),
            Event::Rule => {
                flush(&mut runs, &mut blocks);
                blocks.push(Block::Rule);
            }
            _ => {}
        }
    }
    flush(&mut runs, &mut blocks);
    blocks
}

pub fn link_at(runs: &[(String, Style)], char_index: usize) -> Option<&str> {
    let mut start = 0;
    for (text, style) in runs {
        let end = start + text.chars().count();
        if (start..end).contains(&char_index) {
            return style
                .link
                .as_deref()
                .filter(|url| url.starts_with("https://") || url.starts_with("http://"));
        }
        start = end;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(text: &str) -> (String, Style) {
        (text.to_string(), Style::default())
    }

    #[test]
    fn highlights_the_code_blocks() {
        let text = "```kotlin\nval x = 1\n```\n\n```\nfun f()\n```\n```rust\nfn g()\n```";
        assert_eq!(
            parse(Path::new("a.kt"), text),
            [
                Block::Code(vec![(
                    "val x = 1".to_string(),
                    vec![(0..3, "keyword"), (8..9, "number")]
                )]),
                Block::Code(vec![(
                    "fun f()".to_string(),
                    vec![(0..3, "keyword"), (4..5, "function")]
                )]),
                Block::Code(vec![(
                    "fn g()".to_string(),
                    vec![
                        (0..2, "keyword"),
                        (3..4, "function"),
                        (4..6, "punctuation.bracket")
                    ]
                )]),
            ]
        );
    }

    #[test]
    fn joins_lines_and_shows_links_by_their_text() {
        let text = "egui::response\n\n---\n\nThe result of adding a\n[`Ui`](https://docs.rs/ui.html) to **it**, _maybe_.";
        let code = Style {
            code: true,
            link: Some("https://docs.rs/ui.html".to_string()),
            ..Style::default()
        };
        assert_eq!(
            parse(Path::new("a.rs"), text),
            [
                Block::Text(vec![plain("egui::response")]),
                Block::Rule,
                Block::Text(vec![
                    plain("The result of adding a"),
                    plain(" "),
                    ("Ui".to_string(), code),
                    plain(" to "),
                    (
                        "it".to_string(),
                        Style {
                            strong: true,
                            ..Style::default()
                        }
                    ),
                    plain(", "),
                    (
                        "maybe".to_string(),
                        Style {
                            emphasis: true,
                            ..Style::default()
                        }
                    ),
                    plain("."),
                ]),
            ]
        );
    }

    #[test]
    fn shows_headings_and_lists() {
        let text = "# Panics\n\n- one\n- two\n\n1. first\n2. second";
        let heading = Style {
            heading: true,
            ..Style::default()
        };
        assert_eq!(
            parse(Path::new("a.rs"), text),
            [
                Block::Text(vec![("Panics".to_string(), heading)]),
                Block::Text(vec![plain("• "), plain("one")]),
                Block::Text(vec![plain("• "), plain("two")]),
                Block::Text(vec![plain("1. "), plain("first")]),
                Block::Text(vec![plain("2. "), plain("second")]),
            ]
        );
    }

    #[test]
    fn finds_the_web_link_at_a_character() {
        let blocks = parse(
            Path::new("a.rs"),
            "é [Ui](https://docs.rs/ui.html) and [Id](crate::Id)",
        );
        let Block::Text(runs) = &blocks[0] else {
            panic!("expected text");
        };
        assert_eq!(link_at(runs, 1), None);
        assert_eq!(link_at(runs, 2), Some("https://docs.rs/ui.html"));
        assert_eq!(link_at(runs, 3), Some("https://docs.rs/ui.html"));
        assert_eq!(link_at(runs, 4), None);
        assert_eq!(link_at(runs, 9), None);
        assert_eq!(link_at(runs, 99), None);
    }
}
