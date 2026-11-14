use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use eframe::egui;

use crate::buffer::Buffer;
use crate::editor::Editor;

/// Files larger than this aren't shown, to keep the editor responsive.
const MAX_SIZE: u64 = 20 * 1024 * 1024;

/// Edits are saved on their own once nothing was typed for this long, so that agents and build
/// tools always see what the editor shows.
const SAVE_DELAY: Duration = Duration::from_secs(1);

/// The files that are open in tabs.
pub struct OpenFiles {
    root: PathBuf,
    files: HashMap<String, OpenFile>,
}

struct OpenFile {
    path: PathBuf,
    /// When the file on disk was changed, as of the last time it was read or saved.
    modified: Option<SystemTime>,
    /// The text of the file, or why it can't be shown.
    buffer: Result<Buffer, String>,
    editor: Editor,
    /// The revision of the text when it was edited last and the time of that, to save it after a while.
    edited: (u64, Instant),
    save_error: Option<String>,
}

impl OpenFiles {
    pub fn new(root: PathBuf) -> Self {
        Self { root, files: HashMap::new() }
    }

    fn file(&mut self, path: &str) -> &mut OpenFile {
        self.files.entry(path.to_string()).or_insert_with(|| OpenFile::open(self.root.join(path)))
    }

    /// Makes the editor of the file take the keyboard when it is shown next.
    pub fn focus(&mut self, path: &str) {
        self.file(path).editor.focus();
    }

    /// Takes over what changed on disk, for example when an agent edited an open file.
    pub fn reload_changed(&mut self) {
        for file in self.files.values_mut() {
            file.reload_if_changed();
        }
    }

    /// Saves the files that were edited a while ago.
    pub fn save_edited(&mut self, ctx: &egui::Context) {
        for file in self.files.values_mut() {
            let Ok(buffer) = &file.buffer else { continue };
            if !buffer.is_dirty() {
                continue;
            }
            if file.edited.0 != buffer.revision() {
                file.edited = (buffer.revision(), Instant::now());
            }
            match SAVE_DELAY.checked_sub(file.edited.1.elapsed()) {
                Some(left) => ctx.request_repaint_after(left),
                None => file.save(),
            }
        }
    }

    /// Saves every file that has unsaved edits.
    pub fn save_all(&mut self) {
        for file in self.files.values_mut() {
            file.save();
        }
    }

    pub fn close(&mut self, path: &str) {
        if let Some(mut file) = self.files.remove(path) {
            file.save();
        }
    }

    /// Shows the file for editing.
    pub fn ui(&mut self, ui: &mut egui::Ui, path: &str) {
        let file = self.file(path);
        if let Some(error) = &file.save_error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        match &mut file.buffer {
            Ok(buffer) => file.editor.ui(ui, path, buffer),
            Err(message) => {
                ui.centered_and_justified(|ui| ui.weak(message.as_str()));
            }
        }
    }
}

impl OpenFile {
    fn open(path: PathBuf) -> Self {
        let buffer = read(&path).map(|text| Buffer::new(&text));
        Self { modified: modified(&path), buffer, editor: Editor::default(), edited: (0, Instant::now()), save_error: None, path }
    }

    fn save(&mut self) {
        let Ok(buffer) = &mut self.buffer else { return };
        if !buffer.is_dirty() {
            return;
        }
        self.edited = (buffer.revision(), Instant::now());
        match fs::write(&self.path, buffer.text()) {
            Ok(()) => {
                buffer.mark_saved();
                self.save_error = None;
                self.modified = modified(&self.path);
            }
            Err(e) => self.save_error = Some(format!("Cannot save file: {e}")),
        }
    }

    fn reload_if_changed(&mut self) {
        let on_disk = modified(&self.path);
        if on_disk == self.modified {
            return;
        }
        self.modified = on_disk;
        match (&mut self.buffer, read(&self.path)) {
            (Ok(buffer), Ok(text)) if buffer.text() == text => {}
            // Edits that aren't saved yet win over the change on disk: they were made in the
            // last moment and are about to be saved.
            (Ok(buffer), _) if buffer.is_dirty() => {}
            (Ok(buffer), Ok(text)) => {
                buffer.set_text(&text);
                buffer.mark_saved();
            }
            (_, read) => self.buffer = read.map(|text| Buffer::new(&text)),
        }
    }
}

fn read(path: &Path) -> Result<String, String> {
    let size = fs::metadata(path).map_err(|e| format!("Cannot open file: {e}"))?.len();
    if size > MAX_SIZE {
        return Err("File is too large to show".to_string());
    }
    let bytes = fs::read(path).map_err(|e| format!("Cannot open file: {e}"))?;
    if bytes.contains(&0) {
        return Err("Binary file".to_string());
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn modified(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|m| m.modified()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    fn open(dir: &TempDir, name: &str, content: &str) -> OpenFiles {
        dir.write(name, content);
        let mut files = OpenFiles::new(dir.path().to_path_buf());
        files.file(name);
        files
    }

    fn buffer<'a>(files: &'a mut OpenFiles, name: &str) -> &'a mut Buffer {
        files.file(name).buffer.as_mut().unwrap()
    }

    /// Writes the file as another program would: at a later time than the editor read it.
    fn write_later(dir: &TempDir, name: &str, content: &str) {
        let path = dir.write(name, content);
        let later = SystemTime::now() + Duration::from_secs(5);
        fs::File::options().write(true).open(path).unwrap().set_modified(later).unwrap();
    }

    #[test]
    fn refuses_binary_and_missing_files() {
        let dir = TempDir::new("binary");
        assert_eq!(read(&dir.write("image.png", [0x89, b'P', b'N', b'G', 0, 1])).unwrap_err(), "Binary file");
        assert!(read(&dir.path().join("missing")).unwrap_err().starts_with("Cannot open file"));
    }

    #[test]
    fn saves_edits_as_they_are_in_the_editor() {
        let dir = TempDir::new("save");
        let mut files = open(&dir, "a.txt", "one\r\n\ttwo\r\n");
        buffer(&mut files, "a.txt").type_text("zero ");
        files.save_all();
        assert_eq!(fs::read_to_string(dir.path().join("a.txt")).unwrap(), "zero one\r\n\ttwo\r\n");
        assert!(!buffer(&mut files, "a.txt").is_dirty());
    }

    #[test]
    fn saves_edits_a_while_after_the_last_one_and_when_the_file_is_closed() {
        let dir = TempDir::new("save-later");
        let mut files = open(&dir, "a.txt", "one");
        let on_disk = || fs::read_to_string(dir.path().join("a.txt")).unwrap();
        let ctx = egui::Context::default();
        buffer(&mut files, "a.txt").type_text("1");
        files.save_edited(&ctx);
        assert_eq!(on_disk(), "one");
        files.file("a.txt").edited.1 -= SAVE_DELAY;
        files.save_edited(&ctx);
        assert_eq!(on_disk(), "1one");

        buffer(&mut files, "a.txt").type_text("2");
        files.close("a.txt");
        assert_eq!(on_disk(), "12one");
    }

    #[test]
    fn takes_over_a_file_changed_on_disk() {
        let dir = TempDir::new("changed");
        let mut files = open(&dir, "a.txt", "one\ntwo");
        files.reload_changed();
        assert_eq!(buffer(&mut files, "a.txt").text(), "one\ntwo");

        write_later(&dir, "a.txt", "one\n2");
        files.reload_changed();
        assert_eq!(buffer(&mut files, "a.txt").text(), "one\n2");
        assert!(!buffer(&mut files, "a.txt").is_dirty());

        fs::remove_file(dir.path().join("a.txt")).unwrap();
        files.reload_changed();
        assert!(files.file("a.txt").buffer.as_ref().is_err_and(|message| message.starts_with("Cannot open file")));
    }

    #[test]
    fn unsaved_edits_win_over_a_change_on_disk() {
        let dir = TempDir::new("conflict");
        let mut files = open(&dir, "a.txt", "one");
        buffer(&mut files, "a.txt").type_text("my ");
        write_later(&dir, "a.txt", "theirs");
        files.reload_changed();
        files.save_all();
        assert_eq!(fs::read_to_string(dir.path().join("a.txt")).unwrap(), "my one");
    }
}
