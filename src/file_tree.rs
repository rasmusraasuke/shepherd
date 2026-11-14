use std::cmp::Ordering;
use std::collections::BTreeSet;

use eframe::egui::{self, Align2, Color32, Sense, TextStyle, Vec2};

/// Files as a tree of directories, of which only the open ones show their content.
#[derive(Default)]
pub struct FileTree {
    /// Paths relative to the project root, ordered the way the tree shows them.
    files: Vec<String>,
    /// The rows currently visible, kept so that drawing costs nothing for closed directories.
    rows: Vec<Row>,
    /// Whether directories show their content until they are closed, rather than once opened.
    open_by_default: bool,
}

/// How a file is shown in the tree besides its name.
#[derive(Default)]
pub struct FileLook {
    pub color: Option<Color32>,
    /// Text at the right end of the row.
    pub detail: Option<String>,
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
    pub fn open_by_default() -> Self {
        Self { open_by_default: true, ..Self::default() }
    }

    /// Sets the files to show. `toggled` holds the directories that were opened, or closed
    /// when directories are open by default.
    pub fn set_files(&mut self, mut files: Vec<String>, toggled: &BTreeSet<String>) {
        files.sort_by(|a, b| tree_order(a, b));
        self.files = files;
        self.update_rows(toggled);
    }

    fn is_open(&self, dir: &str, toggled: &BTreeSet<String>) -> bool {
        toggled.contains(dir) != self.open_by_default
    }

    fn update_rows(&mut self, toggled: &BTreeSet<String>) {
        self.rows = rows(&self.files, |dir| self.is_open(dir, toggled));
    }

    /// Shows the tree with the selected file highlighted. Returns the file that was clicked, if any.
    pub fn ui(&mut self, ui: &mut egui::Ui, toggled: &mut BTreeSet<String>, selected: Option<&str>, look: impl Fn(&str) -> FileLook) -> Option<String> {
        let font = TextStyle::Body.resolve(ui.style());
        let row_height = ui.text_style_height(&TextStyle::Body) + 6.0;
        let indent = 14.0;
        let mut clicked = None;
        ui.spacing_mut().item_spacing.y = 0.0;
        egui::ScrollArea::vertical().auto_shrink(false).show_rows(ui, row_height, self.rows.len(), |ui, range| {
            for row in &self.rows[range] {
                let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), row_height), Sense::click());
                response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), row.name()));
                if selected == Some(row.path.as_str()) {
                    ui.painter().rect_filled(rect, 2.0, ui.visuals().selection.bg_fill);
                } else if response.hovered() {
                    ui.painter().rect_filled(rect, 2.0, ui.visuals().widgets.hovered.weak_bg_fill);
                }
                let x = rect.left() + 4.0 + indent * row.depth as f32;
                let y = rect.center().y;
                let painter = ui.painter().with_clip_rect(rect);
                if row.is_dir {
                    let (center, r) = (egui::pos2(x + 5.0, y), 3.5);
                    let arrow = if self.is_open(&row.path, toggled) {
                        vec![center + egui::vec2(-r, -r / 2.0), center + egui::vec2(r, -r / 2.0), center + egui::vec2(0.0, r)]
                    } else {
                        vec![center + egui::vec2(-r / 2.0, -r), center + egui::vec2(r, 0.0), center + egui::vec2(-r / 2.0, r)]
                    };
                    painter.add(egui::Shape::convex_polygon(arrow, ui.visuals().weak_text_color(), egui::Stroke::NONE));
                }
                let look = if row.is_dir { FileLook::default() } else { look(&row.path) };
                let color = look.color.unwrap_or(ui.visuals().text_color());
                painter.text(egui::pos2(x + indent, y), Align2::LEFT_CENTER, row.name(), font.clone(), color);
                if let Some(detail) = look.detail {
                    painter.text(egui::pos2(rect.right() - 6.0, y), Align2::RIGHT_CENTER, detail, font.clone(), ui.visuals().weak_text_color());
                }
                if response.clicked() {
                    clicked = Some((row.path.clone(), row.is_dir));
                }
            }
        });
        match clicked {
            Some((path, true)) => {
                if !toggled.remove(&path) {
                    toggled.insert(path);
                }
                self.update_rows(toggled);
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
fn rows(files: &[String], is_open: impl Fn(&str) -> bool) -> Vec<Row> {
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
            open.push(visible && is_open(path));
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
        shown(FileTree::default(), files, expanded)
    }

    fn shown(mut tree: FileTree, files: &[&str], toggled: &[&str]) -> Vec<String> {
        let toggled = toggled.iter().map(|s| s.to_string()).collect();
        tree.set_files(files.iter().map(|s| s.to_string()).collect(), &toggled);
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

    #[test]
    fn tree_open_by_default_shows_everything_but_closed_directories() {
        assert_eq!(shown(FileTree::open_by_default(), &["src/ui/tabs.rs", "src/app.rs", "README.md"], &["src/ui"]), ["src/", "  ui/", "  app.rs", "README.md"]);
    }
}
