use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use eframe::egui::{self, Key, Modifiers};

use crate::editor::Editor;
use crate::editor::buffer::{Buffer, Pos};
use crate::editor::highlight::Highlighter;
use crate::editor::image_view::{ImageView, Source};
use crate::git::{Commit, Hunk, Repo};
use crate::lsp::install::Installer;
use crate::lsp::{Event, Lsp};
use crate::panels::search::Reveal;
use crate::review::line_diff;
use crate::ui::icons::FileType;

const MAX_SIZE: u64 = 20 * 1024 * 1024;

const SAVE_DELAY: Duration = Duration::from_millis(500);

pub struct OpenFiles {
    root: PathBuf,
    repo: Option<Repo>,
    head: String,
    files: HashMap<String, OpenFile>,
    last_active: Option<String>,
    commit_to_open: Option<String>,
    lsp: Lsp,
    reveal: Option<Reveal>,
}

pub struct Selection {
    pub path: String,
    pub start: Pos,
    pub end: Pos,
    pub text: String,
}

struct OpenFile {
    path: PathBuf,
    kind: FileType,
    preview: Option<ImageView>,
    modified: Option<SystemTime>,
    buffer: Result<Buffer, String>,
    highlighter: Option<Highlighter>,
    editor: Editor,
    edited: (u64, Instant),
    save_error: Option<String>,
    blame: Option<Result<Vec<Commit>, String>>,
    committed: Option<Vec<String>>,
    changes: Vec<Hunk>,
    changes_for: Option<u64>,
}

impl OpenFiles {
    pub fn new(root: PathBuf) -> Self {
        let repo = Repo::open(&root).ok();
        let head = repo.as_ref().map(Repo::head).unwrap_or_default();
        Self {
            lsp: Lsp::new(root.clone()),
            root,
            repo,
            head,
            files: HashMap::new(),
            last_active: None,
            commit_to_open: None,
            reveal: None,
        }
    }

    pub fn start_language_servers(&mut self, ctx: egui::Context, installer: Installer) {
        self.lsp.enable(ctx, installer);
    }

    #[cfg(test)]
    pub fn connect_language_server(&mut self, language: &'static str, server: crate::lsp::Server) {
        self.lsp.connect(language, server);
    }

    pub fn stop_language_servers(&mut self) {
        self.lsp.shutdown();
    }

    pub fn diagnostics(&self, path: Option<&str>) -> String {
        self.lsp.report(path)
    }

    pub fn problem_count(&self) -> usize {
        self.lsp.problem_count()
    }

    pub fn take_reveal(&mut self) -> Option<Reveal> {
        self.reveal.take()
    }

    pub fn poll_language_servers(&mut self) {
        for (path, file) in &self.files {
            if let Ok(buffer) = &file.buffer {
                self.lsp.sync(path, buffer);
            }
        }
        for event in self.lsp.poll() {
            match event {
                Event::Hover { path, pos, text } => {
                    if let Some(file) = self.files.get_mut(&path) {
                        file.editor.set_hover_text(pos, Path::new(&path), &text);
                    }
                }
                Event::Definition { path, start, .. } => {
                    self.reveal = Some(Reveal {
                        path,
                        start,
                        end: start,
                        focus: true,
                    });
                }
            }
        }
        for (path, file) in &mut self.files {
            file.editor
                .set_language_server(self.lsp.has_server(path), self.lsp.diagnostics(path));
            if let Some(pos) = file.editor.take_hover_request() {
                self.lsp.hover(path, pos);
            }
            if let Some(pos) = file.editor.take_definition_request() {
                self.lsp.definition(path, pos);
            }
        }
    }

    fn file(&mut self, path: &str) -> &mut OpenFile {
        let (root, repo) = (&self.root, self.repo.as_ref());
        self.files.entry(path.to_string()).or_insert_with(|| {
            let outside = Path::new(path).is_absolute();
            let committed = repo
                .filter(|repo| !outside && !repo.is_ignored(path))
                .map(|repo| repo.content_at("HEAD", path).unwrap_or_default());
            let mut file = OpenFile::open(root.join(path), committed);
            file.editor.set_read_only(outside);
            file
        })
    }

    pub fn text(
        &mut self,
        path: &str,
        missing_is_empty: bool,
    ) -> Result<(&mut Buffer, Option<&mut Highlighter>), &str> {
        let file = self.file(path);
        if missing_is_empty && file.buffer.is_err() && !file.path.exists() {
            file.buffer = Ok(Buffer::new(""));
        }
        match &mut file.buffer {
            Ok(buffer) => Ok((buffer, file.highlighter.as_mut())),
            Err(message) => Err(message),
        }
    }

    pub fn focus(&mut self, path: &str) {
        self.file(path).editor.focus();
    }

    pub fn selected_text(&self) -> Option<String> {
        let file = self.files.values().find(|file| file.editor.is_active())?;
        file.buffer
            .as_ref()
            .ok()?
            .selected_text()
            .filter(|text| !text.contains('\n'))
    }

    pub fn selection(&self) -> Option<Selection> {
        let path = self.last_active.as_ref()?;
        let buffer = self.files.get(path)?.buffer.as_ref().ok()?;
        let (start, end) = buffer
            .selection()
            .unwrap_or((buffer.cursor(), buffer.cursor()));
        Some(Selection {
            path: path.clone(),
            start,
            end,
            text: buffer.selected_text().unwrap_or_default(),
        })
    }

    pub fn reveal(&mut self, path: &str, start: Pos, end: Pos, focus: bool) {
        let file = self.file(path);
        if let Ok(buffer) = &mut file.buffer {
            buffer.select(start, end);
            file.editor.reveal(focus);
        }
    }

    pub fn reload_changed(&mut self) {
        let repo = self.repo.as_ref();
        for file in self.files.values_mut() {
            file.reload_if_changed(repo);
        }
    }

    pub fn set_head(&mut self, head: &str) {
        if self.head == head {
            return;
        }
        self.head = head.to_string();
        let (root, repo) = (&self.root, self.repo.as_ref());
        for file in self
            .files
            .values_mut()
            .filter(|file| file.committed.is_some())
        {
            file.committed = repo.map(|repo| {
                let path = file
                    .path
                    .strip_prefix(root)
                    .unwrap_or(&file.path)
                    .to_string_lossy();
                repo.content_at("HEAD", &path)
                    .unwrap_or_default()
                    .lines()
                    .map(str::to_string)
                    .collect()
            });
            file.changes_for = None;
            file.update_changes();
            if file.blame.is_some() {
                file.refresh_blame(repo);
            }
        }
    }

    pub fn reload(&mut self, path: &str) {
        let Some(file) = self.files.get_mut(path) else {
            return;
        };
        file.buffer = read(&file.path).map(|text| Buffer::new(&text));
        file.modified = modified(&file.path);
        file.save_error = None;
        file.refresh_blame(self.repo.as_ref());
    }

    pub fn take_commit_to_open(&mut self) -> Option<String> {
        self.commit_to_open.take()
    }

    pub fn blame_on(&self, path: &str) -> bool {
        self.files
            .get(path)
            .is_some_and(|file| file.blame.is_some())
    }

    pub fn toggle_blame(&mut self, path: &str) {
        let on = self.blame_on(path);
        let blame = if on {
            None
        } else {
            Some(match &self.repo {
                Some(repo) => repo.blame_lines(path),
                None => Err("No git repository".to_string()),
            })
        };
        self.file(path).blame = blame;
    }

    pub fn save_edited(&mut self, ctx: &egui::Context) {
        let repo = self.repo.as_ref();
        for (path, file) in &mut self.files {
            let Ok(buffer) = &file.buffer else { continue };
            if !buffer.is_dirty() {
                continue;
            }
            if file.edited.0 != buffer.revision() {
                file.edited = (buffer.revision(), Instant::now());
            }
            match SAVE_DELAY.checked_sub(file.edited.1.elapsed()) {
                Some(left) => ctx.request_repaint_after(left),
                None => {
                    if file.save(repo) {
                        self.lsp.saved(path);
                    }
                }
            }
        }
    }

    pub fn save_all(&mut self) {
        let repo = self.repo.as_ref();
        for (path, file) in &mut self.files {
            if file.save(repo) {
                self.lsp.saved(path);
            }
        }
    }

    pub fn close(&mut self, path: &str) {
        if let Some(mut file) = self.files.remove(path) {
            if file.save(self.repo.as_ref()) {
                self.lsp.saved(path);
            }
            self.lsp.close(path);
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, path: &str) -> Option<usize> {
        let active = self
            .files
            .get(path)
            .is_some_and(|file| file.editor.is_active());
        if active && ui.input_mut(|i| i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::B))
        {
            self.toggle_blame(path);
        }
        let status = self.lsp.status(path);
        let view = ui.max_rect();
        let file = self.file(path);
        file.update_changes();
        if let Some(error) = &file.save_error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        if let Some(Err(error)) = &file.blame {
            ui.colored_label(ui.visuals().error_fg_color, error.as_str());
        }
        let annotations = file
            .blame
            .as_ref()
            .and_then(|blame| blame.as_ref().ok())
            .map(Vec::as_slice);
        let blame_on = file.blame.is_some();
        let mut toggle = false;
        let mut open_commit = None;
        let mut label = None;
        let (code, picture) = ui
            .available_rect_before_wrap()
            .split_left_right_at_fraction(0.5);
        let line = match &mut file.buffer {
            Ok(buffer) => {
                let review = file.editor.is_active()
                    && ui.input_mut(|i| {
                        i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::R)
                    });
                let mut show = |ui: &mut egui::Ui| {
                    file.editor.ui(
                        ui,
                        path,
                        buffer,
                        file.highlighter.as_mut(),
                        annotations,
                        &file.changes,
                        blame_on,
                        &mut toggle,
                        &mut open_commit,
                    )
                };
                match &mut file.preview {
                    Some(preview) => {
                        ui.scope_builder(egui::UiBuilder::new().max_rect(code), show);
                        ui.painter().vline(
                            picture.left(),
                            picture.y_range(),
                            ui.visuals().widgets.noninteractive.bg_stroke,
                        );
                        let dir = file.path.parent().map(Path::to_path_buf);
                        label = ui
                            .scope_builder(egui::UiBuilder::new().max_rect(picture), |ui| {
                                preview.ui(ui, buffer.revision(), || Source::Svg {
                                    text: buffer.text(),
                                    dir,
                                })
                            })
                            .inner;
                    }
                    None => show(ui),
                }
                let line = review.then(|| buffer.cursor().line);
                if review {
                    ui.memory_mut(|m| m.focused().map(|id| m.surrender_focus(id)));
                }
                if file.editor.is_active() {
                    self.last_active = Some(path.to_string());
                }
                line
            }
            Err(_) if file.kind == FileType::Image => {
                let key = file
                    .modified
                    .and_then(|time| time.duration_since(SystemTime::UNIX_EPOCH).ok())
                    .map_or(0, |time| time.as_nanos() as u64);
                let path = file.path.clone();
                label = file
                    .preview
                    .as_mut()
                    .and_then(|preview| preview.ui(ui, key, || Source::File(path)));
                None
            }
            Err(message) => {
                ui.centered_and_justified(|ui| ui.weak(message.as_str()));
                None
            }
        };
        self.commit_to_open = open_commit;
        if toggle {
            self.toggle_blame(path);
        }
        if let Some(status) = status.or(label) {
            status_ui(ui, view, &status);
        }
        line
    }
}

fn status_ui(ui: &egui::Ui, view: egui::Rect, status: &str) {
    let margin = egui::vec2(10.0, 8.0);
    let galley = ui.painter().layout(
        status.to_string(),
        egui::TextStyle::Small.resolve(ui.style()),
        ui.visuals().weak_text_color(),
        (view.width() * 0.8).min(520.0),
    );
    let size = galley.size() + margin * 2.0;
    let rect = egui::Rect::from_min_size(view.max - size - egui::vec2(12.0, 12.0), size);
    let painter = ui.painter().with_clip_rect(view);
    painter.rect(
        rect,
        6.0,
        ui.visuals().panel_fill,
        egui::Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color),
        egui::StrokeKind::Inside,
    );
    painter.galley(rect.min + margin, galley, ui.visuals().weak_text_color());
    ui.interact(
        rect,
        ui.scope_id().with("language-server-status"),
        egui::Sense::hover(),
    )
    .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, status));
}

impl OpenFile {
    fn open(path: PathBuf, committed: Option<String>) -> Self {
        let kind = FileType::of(&path.to_string_lossy());
        let buffer = match kind {
            FileType::Image => Err("An image has no text".to_string()),
            _ => read(&path).map(|text| Buffer::new(&text)),
        };
        let highlighter = Highlighter::for_path(&path);
        Self {
            kind,
            preview: matches!(kind, FileType::Svg | FileType::Image).then(ImageView::default),
            modified: modified(&path),
            buffer,
            highlighter,
            editor: Editor::default(),
            edited: (0, Instant::now()),
            save_error: None,
            blame: None,
            committed: committed.map(|text| text.lines().map(str::to_string).collect()),
            changes: Vec::new(),
            changes_for: None,
            path,
        }
    }

    fn update_changes(&mut self) {
        let (Some(committed), Ok(buffer)) = (&self.committed, &self.buffer) else {
            self.changes.clear();
            self.changes_for = None;
            return;
        };
        let revision = buffer.revision();
        if self.changes_for == Some(revision) {
            return;
        }
        let lines = buffer.lines();
        let work = &lines[..lines.len() - lines.last().is_some_and(String::is_empty) as usize];
        self.changes = line_diff::diff(committed, work);
        self.changes_for = Some(revision);
    }

    fn save(&mut self, repo: Option<&Repo>) -> bool {
        let Ok(buffer) = &mut self.buffer else {
            return false;
        };
        if !buffer.is_dirty() {
            return false;
        }
        self.edited = (buffer.revision(), Instant::now());
        match fs::write(&self.path, buffer.text()) {
            Ok(()) => {
                buffer.mark_saved();
                self.save_error = None;
                self.modified = modified(&self.path);
                self.refresh_blame(repo);
                true
            }
            Err(e) => {
                self.save_error = Some(format!("Cannot save file: {e}"));
                false
            }
        }
    }

    fn reload_if_changed(&mut self, repo: Option<&Repo>) {
        let on_disk = modified(&self.path);
        if on_disk == self.modified {
            return;
        }
        self.modified = on_disk;
        if self.kind == FileType::Image {
            return;
        }
        let mut reloaded = false;
        match (&mut self.buffer, read(&self.path)) {
            (Ok(buffer), Ok(text)) if buffer.text() == text => {}
            (Ok(buffer), _) if buffer.is_dirty() => {}
            (Ok(buffer), Ok(text)) => {
                buffer.set_text(&text);
                buffer.mark_saved();
                reloaded = true;
            }
            (_, read) => {
                self.buffer = read.map(|text| Buffer::new(&text));
                reloaded = true;
            }
        }
        if reloaded {
            self.refresh_blame(repo);
        }
    }

    fn refresh_blame(&mut self, repo: Option<&Repo>) {
        if self.blame.is_none() {
            return;
        }
        self.blame = Some(match repo {
            Some(repo) => {
                let path = self
                    .path
                    .strip_prefix(&repo.root)
                    .unwrap_or(&self.path)
                    .to_string_lossy()
                    .into_owned();
                repo.blame_lines(&path)
            }
            None => Err("No git repository".to_string()),
        });
    }
}

pub fn read(path: &Path) -> Result<String, String> {
    let size = fs::metadata(path)
        .map_err(|e| format!("Cannot open file: {e}"))?
        .len();
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

    fn write_later(dir: &TempDir, name: &str, content: &str) {
        let path = dir.write(name, content);
        let later = SystemTime::now() + Duration::from_secs(5);
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(later)
            .unwrap();
    }

    #[test]
    fn finds_the_lines_that_changed_since_the_commit() {
        use crate::git::LineKind;

        let (repo, dir) = crate::git::tests::temp_repo("file-changes", &[("a.txt", "one\ntwo\n")]);
        crate::git::tests::commit_all(&repo);
        let mut files = OpenFiles::new(dir.path().to_path_buf());
        let file = files.file("a.txt");
        file.update_changes();
        assert!(file.changes.is_empty());

        file.buffer.as_mut().unwrap().set_text("one\nTWO\nnew\n");
        file.update_changes();
        let kinds: Vec<LineKind> = file
            .changes
            .iter()
            .flat_map(|hunk| &hunk.lines)
            .map(|line| line.kind)
            .collect();
        assert_eq!(kinds, [LineKind::Removed, LineKind::Added, LineKind::Added]);
    }

    #[test]
    fn takes_a_commit_as_the_new_baseline_for_open_files() {
        let (repo, dir) = crate::git::tests::temp_repo("file-commit", &[("a.txt", "one\ntwo\n")]);
        crate::git::tests::commit_all(&repo);
        let mut files = OpenFiles::new(dir.path().to_path_buf());
        let file = files.file("a.txt");
        file.update_changes();
        assert!(file.changes.is_empty());

        file.buffer.as_mut().unwrap().set_text("one\nTWO\nnew\n");
        file.update_changes();
        assert!(!file.changes.is_empty());

        dir.write("a.txt", "one\nTWO\nnew\n");
        repo.stage_file("a.txt", true).unwrap();
        repo.commit("change").unwrap();
        files.set_head(&repo.head());
        assert!(files.file("a.txt").changes.is_empty());
    }

    #[test]
    fn turns_git_blame_on_and_off_for_a_file() {
        let (repo, dir) = crate::git::tests::temp_repo("blame-toggle", &[("a.txt", "one\n")]);
        crate::git::tests::commit_all(&repo);
        let mut files = OpenFiles::new(dir.path().to_path_buf());
        assert!(!files.blame_on("a.txt"));
        files.toggle_blame("a.txt");
        assert!(files.blame_on("a.txt"));
        files.toggle_blame("a.txt");
        assert!(!files.blame_on("a.txt"));
    }

    #[test]
    fn keeps_ignored_files_unmarked_also_after_a_commit() {
        let (repo, dir) = crate::git::tests::temp_repo(
            "ignored",
            &[
                (".gitignore", "node_modules\n"),
                ("node_modules/lib.ts", "x\n"),
            ],
        );
        crate::git::tests::commit_all(&repo);
        let outside = TempDir::new("ignored-outside");
        let library = outside.write("Lib.kt", "class Lib\n");
        let library = library.to_string_lossy();
        let mut files = OpenFiles::new(dir.path().to_path_buf());
        files.file("node_modules/lib.ts").update_changes();
        files.file(&library);
        files.set_head("other");
        for path in ["node_modules/lib.ts", &library] {
            let file = files.file(path);
            assert!(file.committed.is_none());
            assert!(file.changes.is_empty());
        }
    }

    #[test]
    fn keeps_files_outside_the_project_unmarked() {
        let (repo, dir) = crate::git::tests::temp_repo("outside", &[("a.txt", "one\n")]);
        crate::git::tests::commit_all(&repo);
        let outside = TempDir::new("outside-file");
        let path = outside.write("Lib.kt", "class Lib\n");
        let mut files = OpenFiles::new(dir.path().to_path_buf());
        let file = files.file(&path.to_string_lossy());
        file.update_changes();
        assert!(file.committed.is_none());
        assert!(file.changes.is_empty());
        assert_eq!(file.buffer.as_ref().unwrap().text(), "class Lib\n");
    }

    #[test]
    fn refuses_binary_and_missing_files() {
        let dir = TempDir::new("binary");
        assert_eq!(
            read(&dir.write("image.png", [0x89, b'P', b'N', b'G', 0, 1])).unwrap_err(),
            "Binary file"
        );
        assert!(
            read(&dir.path().join("missing"))
                .unwrap_err()
                .starts_with("Cannot open file")
        );
    }

    #[test]
    fn saves_edits_as_they_are_in_the_editor() {
        let dir = TempDir::new("save");
        let mut files = open(&dir, "a.txt", "one\r\n\ttwo\r\n");
        buffer(&mut files, "a.txt").type_text("zero ");
        files.save_all();
        assert_eq!(
            fs::read_to_string(dir.path().join("a.txt")).unwrap(),
            "zero one\r\n\ttwo\r\n"
        );
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
        assert!(
            files
                .file("a.txt")
                .buffer
                .as_ref()
                .is_err_and(|message| message.starts_with("Cannot open file"))
        );
    }

    #[test]
    fn reload_takes_the_file_from_disk_even_over_unsaved_edits() {
        let dir = TempDir::new("reload");
        let mut files = open(&dir, "a.txt", "one");
        buffer(&mut files, "a.txt").type_text("my ");
        write_later(&dir, "a.txt", "theirs");
        files.reload("a.txt");
        assert_eq!(buffer(&mut files, "a.txt").text(), "theirs");
        assert!(!buffer(&mut files, "a.txt").is_dirty());
        files.save_all();
        assert_eq!(
            fs::read_to_string(dir.path().join("a.txt")).unwrap(),
            "theirs"
        );
    }

    #[test]
    fn unsaved_edits_win_over_a_change_on_disk() {
        let dir = TempDir::new("conflict");
        let mut files = open(&dir, "a.txt", "one");
        buffer(&mut files, "a.txt").type_text("my ");
        write_later(&dir, "a.txt", "theirs");
        files.reload_changed();
        files.save_all();
        assert_eq!(
            fs::read_to_string(dir.path().join("a.txt")).unwrap(),
            "my one"
        );
    }
}
