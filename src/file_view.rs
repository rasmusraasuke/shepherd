use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use eframe::egui::{self, Sense, Vec2};

use crate::code::{CodeStyle, expand_tabs};

/// Files larger than this aren't shown, to keep the editor responsive.
const MAX_SIZE: u64 = 20 * 1024 * 1024;

/// The content of the files that are open in tabs.
pub struct OpenFiles {
    root: PathBuf,
    files: HashMap<String, OpenFile>,
}

struct OpenFile {
    modified: Option<SystemTime>,
    content: Result<Content, String>,
}

#[derive(Debug)]
struct Content {
    lines: Vec<String>,
    /// Length of the longest line, for the width of the horizontal scroll.
    max_chars: usize,
}

impl OpenFiles {
    pub fn new(root: PathBuf) -> Self {
        Self { root, files: HashMap::new() }
    }

    /// Forgets files that changed on disk, so they are read again when shown next.
    pub fn drop_changed(&mut self) {
        let root = &self.root;
        self.files.retain(|path, file| modified(&root.join(path)) == file.modified);
    }

    pub fn close(&mut self, path: &str) {
        self.files.remove(path);
    }

    /// Shows the file read-only.
    pub fn ui(&mut self, ui: &mut egui::Ui, path: &str) {
        let file = self.files.entry(path.to_string()).or_insert_with(|| OpenFile::read(&self.root.join(path)));
        let content = match &file.content {
            Ok(content) => content,
            Err(message) => {
                ui.centered_and_justified(|ui| ui.weak(message));
                return;
            }
        };
        let style = CodeStyle::new(ui);
        let number_width = style.digit_width * (content.lines.len().to_string().len().max(3) + 1) as f32;
        let code_x = number_width + style.digit_width * 2.0;
        let width = code_x + style.digit_width * (content.max_chars + 2) as f32;
        ui.spacing_mut().item_spacing.y = 0.0;
        egui::ScrollArea::both().id_salt(path).auto_shrink(false).show_rows(ui, style.row_height, content.lines.len(), |ui, range| {
            let width = width.max(ui.available_width());
            for index in range {
                let (rect, _) = ui.allocate_exact_size(Vec2::new(width, style.row_height), Sense::hover());
                let top = rect.top() + 1.0;
                style.number(ui.painter(), rect.left() + number_width, top, Some(index as u32 + 1));
                style.text(ui.painter(), rect.left() + code_x, top, &content.lines[index], style.text_color);
            }
        });
    }
}

impl OpenFile {
    fn read(path: &Path) -> Self {
        Self { modified: modified(path), content: Content::read(path) }
    }
}

impl Content {
    fn read(path: &Path) -> Result<Self, String> {
        let size = fs::metadata(path).map_err(|e| format!("Cannot open file: {e}"))?.len();
        if size > MAX_SIZE {
            return Err("File is too large to show".to_string());
        }
        let bytes = fs::read(path).map_err(|e| format!("Cannot open file: {e}"))?;
        if bytes.contains(&0) {
            return Err("Binary file".to_string());
        }
        let lines: Vec<String> = String::from_utf8_lossy(&bytes).lines().map(expand_tabs).collect();
        let max_chars = lines.iter().map(|line| line.chars().count()).max().unwrap_or(0);
        Ok(Self { lines, max_chars })
    }
}

fn modified(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|m| m.modified()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    #[test]
    fn reads_lines_with_tabs_expanded() {
        let dir = TempDir::new("lines");
        let content = Content::read(&dir.write("main.rs", "fn main() {\n\tlet x = 1;\r\n}\n")).unwrap();
        assert_eq!(content.lines, ["fn main() {", "    let x = 1;", "}"]);
        assert_eq!(content.max_chars, 14);
    }

    #[test]
    fn refuses_binary_and_missing_files() {
        let dir = TempDir::new("binary");
        assert_eq!(Content::read(&dir.write("image.png", [0x89, b'P', b'N', b'G', 0, 1])).unwrap_err(), "Binary file");
        assert!(Content::read(&dir.path().join("missing")).unwrap_err().starts_with("Cannot open file"));
    }

    #[test]
    fn rereads_a_file_after_it_changed_on_disk() {
        let dir = TempDir::new("changed");
        let path = dir.write("a.txt", "one");
        let mut files = OpenFiles::new(dir.path().to_path_buf());
        files.files.insert("a.txt".to_string(), OpenFile::read(&path));

        files.drop_changed();
        assert!(files.files.contains_key("a.txt"));

        fs::remove_file(&path).unwrap();
        files.drop_changed();
        assert!(files.files.is_empty());
    }
}
