use std::cmp::Ordering;
use std::collections::BTreeSet;

use eframe::egui::{self, Align2, Sense, TextStyle, Vec2};

/// The project files as a tree of directories, of which only the expanded ones show their content.
#[derive(Default)]
pub struct FileTree {
    /// Paths relative to the project root, ordered the way the tree shows them.
    files: Vec<String>,
    /// The rows currently visible, kept so that drawing costs nothing for collapsed directories.
    rows: Vec<Row>,
}

#[derive(Debug, PartialEq)]
struct Row {
    depth: usize,
    path: String,
    is_dir: bool,
}

impl Row {
    fn name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }
}

impl FileTree {
    pub fn set_files(&mut self, mut files: Vec<String>, expanded: &BTreeSet<String>) {
        files.sort_by(|a, b| tree_order(a, b));
        self.files = files;
        self.rows = rows(&self.files, expanded);
    }

    /// Shows the tree. Returns the file that was clicked, if any.
    pub fn ui(&mut self, ui: &mut egui::Ui, expanded: &mut BTreeSet<String>) -> Option<String> {
        let font = TextStyle::Body.resolve(ui.style());
        let row_height = ui.text_style_height(&TextStyle::Body) + 6.0;
        let indent = 14.0;
        let mut clicked = None;
        ui.spacing_mut().item_spacing.y = 0.0;
        egui::ScrollArea::vertical().auto_shrink(false).show_rows(ui, row_height, self.rows.len(), |ui, range| {
            for row in &self.rows[range] {
                let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), row_height), Sense::click());
                response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), row.name()));
                if response.hovered() {
                    ui.painter().rect_filled(rect, 2.0, ui.visuals().widgets.hovered.weak_bg_fill);
                }
                let x = rect.left() + 4.0 + indent * row.depth as f32;
                let y = rect.center().y;
                let painter = ui.painter().with_clip_rect(rect);
                if row.is_dir {
                    let (center, r) = (egui::pos2(x + 5.0, y), 3.5);
                    let arrow = if expanded.contains(&row.path) {
                        vec![center + egui::vec2(-r, -r / 2.0), center + egui::vec2(r, -r / 2.0), center + egui::vec2(0.0, r)]
                    } else {
                        vec![center + egui::vec2(-r / 2.0, -r), center + egui::vec2(r, 0.0), center + egui::vec2(-r / 2.0, r)]
                    };
                    painter.add(egui::Shape::convex_polygon(arrow, ui.visuals().weak_text_color(), egui::Stroke::NONE));
                }
                painter.text(egui::pos2(x + indent, y), Align2::LEFT_CENTER, row.name(), font.clone(), ui.visuals().text_color());
                if response.clicked() {
                    clicked = Some((row.path.clone(), row.is_dir));
                }
            }
        });
        match clicked {
            Some((path, true)) => {
                if !expanded.remove(&path) {
                    expanded.insert(path);
                }
                self.rows = rows(&self.files, expanded);
                None
            }
            Some((path, false)) => Some(path),
            None => None,
        }
    }
}

/// Orders paths so that in every directory the subdirectories come before the files.
fn tree_order(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.split('/').peekable(), b.split('/').peekable());
    loop {
        let (Some(x), Some(y)) = (a.next(), b.next()) else { return Ordering::Equal };
        let (x_is_dir, y_is_dir) = (a.peek().is_some(), b.peek().is_some());
        let order = y_is_dir.cmp(&x_is_dir).then_with(|| x.cmp(y));
        if order != Ordering::Equal {
            return order;
        }
    }
}

/// The visible rows for files given in tree order.
fn rows(files: &[String], expanded: &BTreeSet<String>) -> Vec<Row> {
    fn dirs(path: &str) -> impl Iterator<Item = &str> {
        path.rsplit_once('/').into_iter().flat_map(|(dir, _)| dir.split('/'))
    }

    let mut rows = Vec::new();
    // for each directory leading to the previous file: is its content visible?
    let mut open: Vec<bool> = Vec::new();
    let mut previous = "";
    for file in files {
        let common = dirs(previous).zip(dirs(file)).take_while(|(a, b)| a == b).count();
        open.truncate(common);
        for (end, _) in file.match_indices('/').skip(common) {
            let visible = open.last().copied().unwrap_or(true);
            let path = &file[..end];
            if visible {
                rows.push(Row { depth: open.len(), path: path.to_string(), is_dir: true });
            }
            open.push(visible && expanded.contains(path));
        }
        if open.last().copied().unwrap_or(true) {
            rows.push(Row { depth: open.len(), path: file.clone(), is_dir: false });
        }
        previous = file;
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn visible(files: &[&str], expanded: &[&str]) -> Vec<String> {
        let mut tree = FileTree::default();
        let expanded = expanded.iter().map(|s| s.to_string()).collect();
        tree.set_files(files.iter().map(|s| s.to_string()).collect(), &expanded);
        tree.rows.iter().map(|row| format!("{}{}{}", "  ".repeat(row.depth), row.name(), if row.is_dir { "/" } else { "" })).collect()
    }

    const FILES: &[&str] = &["README.md", "src/main.rs", "src/ui/tabs.rs", "src/app.rs", "docs/plan.md", "srcs.txt"];

    #[test]
    fn collapsed_tree_shows_top_level_with_directories_first() {
        assert_eq!(visible(FILES, &[]), ["docs/", "src/", "README.md", "srcs.txt"]);
    }

    #[test]
    fn expanded_directories_show_their_content() {
        assert_eq!(visible(FILES, &["src"]), ["docs/", "src/", "  ui/", "  app.rs", "  main.rs", "README.md", "srcs.txt"]);
        assert_eq!(
            visible(FILES, &["src", "src/ui", "docs"]),
            ["docs/", "  plan.md", "src/", "  ui/", "    tabs.rs", "  app.rs", "  main.rs", "README.md", "srcs.txt"]
        );
    }

    #[test]
    fn expanded_directory_inside_a_collapsed_one_stays_hidden() {
        assert_eq!(visible(FILES, &["src/ui"]), ["docs/", "src/", "README.md", "srcs.txt"]);
    }

    #[test]
    fn directories_with_the_same_name_in_different_parents_are_separate() {
        assert_eq!(visible(&["a/x/1", "b/x/2"], &["a", "b", "b/x"]), ["a/", "  x/", "b/", "  x/", "    2"]);
    }
}
