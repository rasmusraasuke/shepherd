use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::thread;

use eframe::egui::text::{LayoutJob, TextFormat};
use eframe::egui::{self, Align2, EventFilter, Key, Modifiers, Sense, TextStyle, Vec2};

use crate::buffer::{Pos, find_in_line};
use crate::file_view;

const MAX_FOUND: usize = 2000;

#[derive(Debug, PartialEq)]
pub struct FileFound {
    pub path: String,
    pub found: Vec<Found>,
}

#[derive(Debug, PartialEq)]
pub struct Found {
    pub line: usize,
    pub cols: Range<usize>,
    pub text: String,
}

pub fn search(
    root: &Path,
    files: &[String],
    query: &str,
    cancel: &AtomicBool,
) -> (Vec<FileFound>, bool) {
    let mut results = Vec::new();
    let mut count = 0;
    for path in files {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let Ok(text) = file_view::read(&root.join(path)) else {
            continue;
        };
        let found: Vec<Found> = text
            .lines()
            .enumerate()
            .flat_map(|(line, text)| {
                find_in_line(text, query)
                    .into_iter()
                    .map(move |cols| Found {
                        line,
                        cols,
                        text: text.to_string(),
                    })
            })
            .collect();
        if found.is_empty() {
            continue;
        }
        count += found.len();
        results.push(FileFound {
            path: path.clone(),
            found,
        });
        if count >= MAX_FOUND {
            return (results, true);
        }
    }
    (results, false)
}

#[derive(Debug, PartialEq)]
pub struct Reveal {
    pub path: String,
    pub start: Pos,
    pub end: Pos,
    pub focus: bool,
}

#[derive(Clone, Copy)]
enum Row {
    File(usize),
    Found(usize, usize),
}

#[derive(Default)]
pub struct ProjectSearch {
    query: String,
    focus: bool,
    stale: bool,
    results: (Vec<FileFound>, bool),
    rows: Vec<Row>,
    selected: Option<usize>,
    scroll_to_selected: bool,
    running: Option<(Receiver<(Vec<FileFound>, bool)>, Arc<AtomicBool>)>,
}

impl ProjectSearch {
    pub fn focus(&mut self, query: Option<String>) {
        self.focus = true;
        if let Some(query) = query {
            self.query = query;
            self.stale = true;
        }
    }

    pub fn files_changed(&mut self) {
        self.stale |= !self.query.is_empty();
    }

    fn start(&mut self, root: &Path, files: &[String], ctx: &egui::Context) {
        if let Some((_, cancel)) = self.running.take() {
            cancel.store(true, Ordering::Relaxed);
        }
        if self.query.is_empty() {
            self.set_results((Vec::new(), false));
            return;
        }
        let (sender, receiver) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let (root, files, query, stop, ctx) = (
            PathBuf::from(root),
            files.to_vec(),
            self.query.clone(),
            cancel.clone(),
            ctx.clone(),
        );
        thread::spawn(move || {
            let _ = sender.send(search(&root, &files, &query, &stop));
            ctx.request_repaint();
        });
        self.running = Some((receiver, cancel));
    }

    fn set_results(&mut self, results: (Vec<FileFound>, bool)) {
        let selected = self.selected.and_then(|row| self.place(row));
        self.rows = results
            .0
            .iter()
            .enumerate()
            .flat_map(|(file, found)| {
                [Row::File(file)]
                    .into_iter()
                    .chain((0..found.found.len()).map(move |index| Row::Found(file, index)))
            })
            .collect();
        self.results = results;
        self.selected = selected.and_then(|(path, line, col)| {
            (0..self.rows.len()).find(|&row| self.place(row) == Some((path.clone(), line, col)))
        });
    }

    fn place(&self, row: usize) -> Option<(String, usize, usize)> {
        match self.rows.get(row)? {
            Row::Found(file, index) => {
                let file = &self.results.0[*file];
                Some((
                    file.path.clone(),
                    file.found[*index].line,
                    file.found[*index].cols.start,
                ))
            }
            Row::File(_) => None,
        }
    }

    fn step(&mut self, forward: bool) {
        let found = |row: &usize| matches!(self.rows[*row], Row::Found(..));
        let next = match (self.selected, forward) {
            (None, _) => (0..self.rows.len()).find(found),
            (Some(row), true) => (row + 1..self.rows.len()).find(found),
            (Some(row), false) => (0..row).rev().find(found),
        };
        if next.is_some() {
            self.selected = next;
            self.scroll_to_selected = true;
        }
    }

    fn reveal(&self, focus: bool) -> Option<Reveal> {
        let Row::Found(file, index) = self.rows[self.selected?] else {
            return None;
        };
        let (file, found) = (&self.results.0[file], &self.results.0[file].found[index]);
        Some(Reveal {
            path: file.path.clone(),
            start: Pos::new(found.line, found.cols.start),
            end: Pos::new(found.line, found.cols.end),
            focus,
        })
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, root: &Path, files: &[String]) -> Option<Reveal> {
        if let Some(results) = self
            .running
            .as_ref()
            .and_then(|(receiver, _)| receiver.try_recv().ok())
        {
            self.running = None;
            self.set_results(results);
        }

        let id = egui::Id::unique("project search query");
        let mut reveal = None;
        if ui.memory(|m| m.has_focus(id)) {
            for (key, forward) in [(Key::ArrowDown, true), (Key::ArrowUp, false)] {
                if ui.input_mut(|i| i.consume_key(Modifiers::NONE, key)) {
                    self.step(forward);
                    reveal = self.reveal(false);
                }
            }
        }
        let filter = EventFilter {
            vertical_arrows: true,
            ..Default::default()
        };
        let text_edit = egui::TextEdit::singleline(&mut self.query)
            .id(id)
            .event_filter(filter)
            .hint_text("Search in files");
        let query = ui.add(text_edit.desired_width(f32::INFINITY));
        if std::mem::take(&mut self.focus) {
            query.request_focus();
        }
        if query.changed() {
            self.selected = None;
        }
        if query.changed() || std::mem::take(&mut self.stale) {
            self.start(root, files, ui.ctx());
        }
        if query.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
            if self.selected.is_none() {
                self.step(true);
            }
            reveal = self.reveal(true);
        }
        let status = match &self.results {
            _ if self.running.is_some() => "Searching…".to_string(),
            _ if self.query.is_empty() => String::new(),
            (results, cut_short) => {
                let count: usize = results.iter().map(|file| file.found.len()).sum();
                let more = if *cut_short { "+" } else { "" };
                format!("{count}{more} found in {} files", results.len())
            }
        };
        ui.weak(status);
        ui.separator();

        let results = &self.results.0;
        let font = TextStyle::Monospace.resolve(ui.style());
        let row_height = ui.text_style_height(&TextStyle::Body) + 6.0;
        let mut clicked = None;
        ui.spacing_mut().item_spacing.y = 0.0;
        egui::ScrollArea::both().auto_shrink(false).show_rows(
            ui,
            row_height,
            self.rows.len(),
            |ui, range| {
                if let Some(selected) = self
                    .selected
                    .filter(|_| std::mem::take(&mut self.scroll_to_selected))
                {
                    let top =
                        ui.cursor().top() + row_height * (selected as f32 - range.start as f32);
                    let rect = egui::Rect::from_min_size(
                        egui::pos2(ui.cursor().left(), top),
                        Vec2::new(1.0, row_height),
                    );
                    ui.scroll_to_rect(rect, None);
                }
                for (offset, row) in self.rows[range.clone()].iter().enumerate() {
                    let (rect, response) = ui.allocate_exact_size(
                        Vec2::new(ui.available_width(), row_height),
                        Sense::click(),
                    );
                    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
                    match *row {
                        Row::File(file) => {
                            let path = &results[file].path;
                            response.widget_info(|| {
                                egui::WidgetInfo::labeled(egui::Role::Label, ui.is_enabled(), path)
                            });
                            let pos = egui::pos2(rect.left() + 4.0, rect.center().y);
                            painter.text(
                                pos,
                                Align2::LEFT_CENTER,
                                path,
                                TextStyle::Body.resolve(ui.style()),
                                ui.visuals().strong_text_color(),
                            );
                        }
                        Row::Found(file, index) => {
                            let found = &results[file].found[index];
                            let label = format!("{}:{}", results[file].path, found.line + 1);
                            response.widget_info(|| {
                                egui::WidgetInfo::labeled(
                                    egui::Role::Button,
                                    ui.is_enabled(),
                                    &label,
                                )
                            });
                            if self.selected == Some(range.start + offset) {
                                ui.painter()
                                    .rect_filled(rect, 2.0, ui.visuals().selection.bg_fill);
                            } else if response.hovered() {
                                ui.painter().rect_filled(
                                    rect,
                                    2.0,
                                    ui.visuals().widgets.hovered.weak_bg_fill,
                                );
                            }
                            let number = format!("{:>5}  ", found.line + 1);
                            let job = found_line(&number, found, &font, ui.visuals());
                            let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
                            painter.galley(
                                egui::pos2(
                                    rect.left() + 4.0,
                                    rect.center().y - galley.size().y / 2.0,
                                ),
                                galley,
                                ui.visuals().text_color(),
                            );
                            if response.clicked() {
                                clicked = Some(range.start + offset);
                            }
                        }
                    }
                }
            },
        );
        if let Some(row) = clicked {
            self.selected = Some(row);
            reveal = self.reveal(true);
        }
        reveal
    }
}

fn found_line(
    number: &str,
    found: &Found,
    font: &egui::FontId,
    visuals: &egui::Visuals,
) -> LayoutJob {
    let indent = found
        .text
        .chars()
        .take_while(|c| c.is_whitespace())
        .count()
        .min(found.cols.start);
    let chars: Vec<char> = found.text.chars().collect();
    let part = |range: Range<usize>| chars[range].iter().collect::<String>();
    let format = |color| TextFormat::simple(font.clone(), color);
    let mut job = LayoutJob::default();
    job.append(number, 0.0, format(visuals.weak_text_color()));
    job.append(
        &part(indent..found.cols.start),
        0.0,
        format(visuals.text_color()),
    );
    let marked = TextFormat {
        background: visuals.warn_fg_color.gamma_multiply(0.35),
        ..format(visuals.strong_text_color())
    };
    job.append(&part(found.cols.clone()), 0.0, marked);
    job.append(
        &part(found.cols.end..chars.len()),
        0.0,
        format(visuals.text_color()),
    );
    job
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    fn files(dir: &TempDir, files: &[(&str, &[u8])]) -> Vec<String> {
        files
            .iter()
            .map(|(name, content)| {
                dir.write(name, content);
                name.to_string()
            })
            .collect()
    }

    #[test]
    fn finds_the_query_in_every_file_ignoring_case_unless_it_has_capitals() {
        let dir = TempDir::new("search");
        let files = files(
            &dir,
            &[
                ("a.kt", b"fun main() {\n    Main.run(main)\n}"),
                ("b.kt", b"no match"),
                ("c.png", b"main\0"),
            ],
        );
        let (results, cut_short) = search(dir.path(), &files, "main", &AtomicBool::new(false));
        assert!(!cut_short);
        assert_eq!(results.len(), 1);
        let places: Vec<(usize, Range<usize>)> = results[0]
            .found
            .iter()
            .map(|found| (found.line, found.cols.clone()))
            .collect();
        assert_eq!(places, [(0, 4..8), (1, 4..8), (1, 13..17)]);
        assert_eq!(results[0].found[1].text, "    Main.run(main)");

        let (results, _) = search(dir.path(), &files, "Main", &AtomicBool::new(false));
        assert_eq!(results[0].found.len(), 1);
    }

    #[test]
    fn stops_when_too_much_is_found_or_the_search_is_no_longer_wanted() {
        let dir = TempDir::new("search-stop");
        let line = "x".repeat(MAX_FOUND / 2);
        let files = files(
            &dir,
            &[
                ("a", line.as_bytes()),
                ("b", line.as_bytes()),
                ("c", line.as_bytes()),
            ],
        );
        let (results, cut_short) = search(dir.path(), &files, "x", &AtomicBool::new(false));
        assert_eq!((results.len(), cut_short), (2, true));
        let (results, _) = search(dir.path(), &files, "x", &AtomicBool::new(true));
        assert!(results.is_empty());
    }

    fn chosen(search: &ProjectSearch) -> Option<(String, usize, usize)> {
        search.selected.and_then(|row| search.place(row))
    }

    #[test]
    fn steps_through_the_places_found_over_the_names_of_the_files() {
        let dir = TempDir::new("search-step");
        let files = files(&dir, &[("a", b"x\nx"), ("b", b"x")]);
        let mut search = ProjectSearch::default();
        search.set_results(search_for(&dir, &files, "x"));
        search.step(false);
        assert_eq!(chosen(&search), Some(("a".to_string(), 0, 0)));
        search.step(true);
        search.step(true);
        assert_eq!(chosen(&search), Some(("b".to_string(), 0, 0)));
        search.step(true);
        assert_eq!(chosen(&search), Some(("b".to_string(), 0, 0)));
        search.step(false);
        assert_eq!(
            search.reveal(false),
            Some(Reveal {
                path: "a".to_string(),
                start: Pos::new(1, 0),
                end: Pos::new(1, 1),
                focus: false
            })
        );

        dir.write("b", "y\nx");
        search.set_results(search_for(&dir, &files, "x"));
        assert_eq!(chosen(&search), Some(("a".to_string(), 1, 0)));
        dir.write("a", "x\ny");
        search.set_results(search_for(&dir, &files, "x"));
        assert_eq!(chosen(&search), None);
    }

    fn search_for(dir: &TempDir, files: &[String], query: &str) -> (Vec<FileFound>, bool) {
        search(dir.path(), files, query, &AtomicBool::new(false))
    }

    #[test]
    fn marks_the_place_on_the_line_without_its_indentation() {
        let found = Found {
            line: 4,
            cols: 8..12,
            text: "\t   õun main()".to_string(),
        };
        let job = found_line(
            "    5  ",
            &found,
            &egui::FontId::monospace(12.0),
            &egui::Visuals::dark(),
        );
        let parts: Vec<&str> = job
            .sections
            .iter()
            .map(|section| &job.text[section.byte_range.start.0..section.byte_range.end.0])
            .collect();
        assert_eq!(parts, ["    5  ", "õun ", "main", "()"]);
    }
}
