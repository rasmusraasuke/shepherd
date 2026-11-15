use std::collections::{BTreeSet, HashMap, HashSet};
use std::hash::{DefaultHasher, Hash as _, Hasher as _};
use std::ops::Range;
use std::path::Path;

use eframe::egui::{self, Align, Color32, Key, Modifiers, RichText, Sense, Stroke, Vec2};

use crate::code::{CodeStyle, expand_tabs};
use crate::comments::{self, Comment};
use crate::file_tree::{FileLook, FileTree};
use crate::file_view;
use crate::git::{self, Changes, DiffLine, FileDiff, FileStatus, Hunk, LineKind, Repo, Staged};
use crate::highlight::{self, Highlighter, Spans};
use crate::word_diff::{self, Ranges};

/// The color that marks new changes: the ones the user hasn't looked at yet.
pub const NEW_COLOR: Color32 = Color32::from_rgb(90, 150, 230);

/// How many unchanged lines are shown around each hunk.
const CONTEXT: usize = 3;

/// Longer lines are cut to this many characters, as drawing very long lines takes long.
const MAX_LINE_CHARS: usize = 500;

/// The working-tree changes, reviewed one file at a time.
pub struct Review {
    repo: Repo,
    files: Vec<FileReview>,
    by_path: HashMap<String, usize>,
    /// The changed files, in the same form as the project file tree.
    tree: FileTree,
    /// How each file is shown, kept while other files are reviewed.
    views: HashMap<String, View>,
    /// The lines of the files shown, ready to be drawn.
    texts: HashMap<String, FileText>,
    /// The number of reviewed lines and of all changed lines of every file and directory, and
    /// of all files together under "". A file without lines counts as one.
    progress: HashMap<String, (usize, usize)>,
    /// Why what is staged couldn't be changed the last time.
    error: Option<String>,
    /// Whether what is staged changed since the working tree was looked at.
    staged_changed: bool,
    comments: Vec<Comment>,
    /// The hunks the user has looked at, by their fingerprints; the ones there were when the
    /// editor started count as looked at.
    seen: Option<HashSet<u64>>,
    /// The files with hunks not looked at yet, and the directories they are in.
    unseen: HashSet<String>,
}

/// A terminal that text can be sent to, by its id and title; the one used last comes first.
pub type Agent = (u64, String);

/// What the review asks the editor to do.
#[derive(Debug, PartialEq)]
pub enum Outcome {
    /// Show the changes of another file.
    Show(String),
    /// Send text to the agent in a terminal.
    Send { terminal: u64, text: String },
}

/// A changed file. Its changes are reviewed when they are staged.
struct FileReview {
    diff: FileDiff,
    /// For each line of each hunk, whether it is staged. Unchanged lines count as staged.
    staged: Vec<Vec<bool>>,
    /// Whether the whole file is staged, for changes without lines such as of a binary file.
    file_staged: bool,
    /// For each hunk, what it changes, to tell whether it was looked at.
    fingerprints: Vec<u64>,
    /// For each hunk, whether it is new: changed since the user looked at the file.
    new: Vec<bool>,
}

/// What a hunk changes in a file, wherever it is in the file.
fn fingerprint(path: &str, hunk: &Hunk) -> u64 {
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    for line in &hunk.lines {
        (line.kind, &line.text).hash(&mut hasher);
    }
    hasher.finish()
}

impl FileReview {
    /// The number of staged lines and of all changed lines of the hunks given.
    fn lines_progress(&self, hunks: Range<usize>) -> (usize, usize) {
        let lines = hunks.flat_map(|h| self.diff.hunks[h].lines.iter().zip(&self.staged[h]));
        let changed = lines.filter(|(line, _)| line.kind != LineKind::Context);
        changed.fold((0, 0), |(done, total), (_, staged)| (done + *staged as usize, total + 1))
    }

    fn progress(&self) -> (usize, usize) {
        match self.diff.hunks.len() {
            0 => (self.file_staged as usize, 1),
            hunks => self.lines_progress(0..hunks),
        }
    }

    /// The number of staged lines and of all changed lines of a hunk.
    fn hunk_progress(&self, hunk: usize) -> (usize, usize) {
        self.lines_progress(hunk..hunk + 1)
    }

    fn is_reviewed(&self, hunk: usize) -> bool {
        let (done, total) = self.hunk_progress(hunk);
        done == total
    }
}

#[derive(Default)]
struct View {
    current: usize,
    scroll_to_current: bool,
    /// For each run of unchanged lines (one before each hunk and one after the last): whether
    /// it is shown rather than folded away.
    expanded: Vec<bool>,
    /// The rows selected to comment on, from where the selection started to where it ends.
    selection: Option<(usize, usize)>,
    /// Whether the selection is being dragged.
    selecting: bool,
    /// The comment being written on the selection.
    draft: String,
    /// Give the keyboard to the comment being written, as rows were just selected.
    focus_draft: bool,
}

/// A run of unchanged lines between the changes, by their numbers in the new file.
#[derive(Clone, Debug, PartialEq)]
struct Gap {
    lines: Range<usize>,
    /// What to add to the number of a line for its number in the old file.
    old_offset: isize,
}

/// The lines of both versions of a changed file as they are drawn.
struct FileText {
    /// The lines of the file on disk.
    content: Vec<String>,
    new: Side,
    old: Side,
    /// For each hunk and each of its lines, the parts that changed within the line.
    changed: Vec<Vec<Ranges>>,
    /// The number of characters of the longest line, for the width of the rows.
    longest: usize,
}

/// The lines of one version of a file, with tabs expanded and very long lines cut, and the
/// parts of them to color.
struct Side {
    lines: Vec<String>,
    spans: Vec<Spans>,
}

impl Review {
    pub fn new(repo: Repo) -> Self {
        Self {
            repo,
            files: Vec::new(),
            by_path: HashMap::new(),
            tree: FileTree::open_by_default(),
            views: HashMap::new(),
            texts: HashMap::new(),
            progress: HashMap::new(),
            error: None,
            staged_changed: false,
            comments: Vec::new(),
            seen: None,
            unseen: HashSet::new(),
        }
    }

    pub fn set_changes(&mut self, changes: &Changes, collapsed: &BTreeSet<String>) {
        let staged = Staged::new(changes);
        let before: HashMap<String, FileReview> = self.files.drain(..).map(|file| (file.diff.path.clone(), file)).collect();
        self.files = git::parse_diff(&changes.diff)
            .into_iter()
            .map(|diff| {
                let lines = diff.hunks.iter().map(|hunk| hunk.lines.iter().map(|line| staged.is_staged(&diff.path, line)).collect()).collect();
                let fingerprints: Vec<u64> = diff.hunks.iter().map(|hunk| fingerprint(&diff.path, hunk)).collect();
                FileReview { staged: lines, file_staged: staged.is_file_staged(&diff.path), new: vec![false; fingerprints.len()], fingerprints, diff }
            })
            .collect();
        self.by_path = self.files.iter().enumerate().map(|(index, file)| (file.diff.path.clone(), index)).collect();
        self.tree.set_files(self.files.iter().map(|file| file.diff.path.clone()).collect(), collapsed);
        self.texts.clear();
        self.error = None;
        self.update_progress();
        let fingerprints: HashSet<u64> = self.files.iter().flat_map(|file| file.fingerprints.iter().copied()).collect();
        match &mut self.seen {
            Some(seen) => seen.retain(|fingerprint| fingerprints.contains(fingerprint)),
            None => self.seen = Some(fingerprints),
        }
        self.update_new();
        let (files, by_path) = (&self.files, &self.by_path);
        self.views.retain(|path, view| {
            let Some(&index) = by_path.get(path) else { return false };
            let hunks = &files[index].diff.hunks;
            if view.expanded.len() != hunks.len() + 1 {
                view.expanded.clear();
            }
            // the rows selected are others once the file changed
            if before.get(path).is_none_or(|file| file.diff.hunks != *hunks) {
                view.selection = None;
            }
            view.current = view.current.min(hunks.len().saturating_sub(1));
            true
        });
        let mut lines = HashMap::new();
        self.comments.retain_mut(|comment| {
            let Some(&index) = by_path.get(&comment.path) else { return false };
            let diff = &files[index].diff;
            comment.follow(lines.entry(&diff.path).or_insert_with(|| file_lines(&read_lines(&self.repo.root, diff), diff)))
        });
    }

    /// The changed files in the order of the tree, with the number of reviewed and of all changed
    /// lines of each.
    pub fn files_progress(&self) -> Vec<(&str, (usize, usize))> {
        self.tree.files().iter().map(|path| (path.as_str(), self.progress_of(path))).collect()
    }

    /// The first file in the tree with changes left to review.
    pub fn first_unreviewed(&self) -> Option<String> {
        self.files_progress().into_iter().find(|(_, (done, total))| done < total).map(|(path, _)| path.to_string())
    }

    /// The comments on the changes, in the order they were made.
    pub fn comments(&self) -> &[Comment] {
        &self.comments
    }

    /// Whether what is staged changed since the last call, so the working tree should be looked at again.
    pub fn take_staged_changed(&mut self) -> bool {
        std::mem::take(&mut self.staged_changed)
    }

    fn update_progress(&mut self) {
        self.progress.clear();
        for file in &self.files {
            let (done, total) = file.progress();
            let path = file.diff.path.as_str();
            let dirs = path.match_indices('/').map(|(end, _)| &path[..end]);
            for key in ["", path].into_iter().chain(dirs) {
                let progress = self.progress.entry(key.to_string()).or_default();
                *progress = (progress.0 + done, progress.1 + total);
            }
        }
    }

    /// Takes note of which hunks are new, by file and directory.
    fn update_new(&mut self) {
        self.unseen.clear();
        let seen = self.seen.get_or_insert_default();
        for file in &mut self.files {
            file.new = file.fingerprints.iter().map(|fingerprint| !seen.contains(fingerprint)).collect();
            if file.new.contains(&true) {
                let path = file.diff.path.as_str();
                self.unseen.extend(path.match_indices('/').map(|(end, _)| path[..end].to_string()).chain([path.to_string()]));
            }
        }
    }

    /// Takes note that the user looked at a hunk.
    fn see(&mut self, file: usize, hunk: usize) {
        if let Some(&fingerprint) = self.files[file].fingerprints.get(hunk) {
            self.seen.get_or_insert_default().insert(fingerprint);
            self.update_new();
        }
    }

    /// Makes a hunk the current one, which the user looks at then.
    fn make_current(&mut self, file: usize, hunk: usize) {
        self.view(file).current = hunk;
        self.see(file, hunk);
    }

    /// The number of reviewed lines and of all changed lines.
    pub fn progress(&self) -> (usize, usize) {
        self.progress_of("")
    }

    /// The number of reviewed lines and of all changed lines of a file, or of the files in a directory.
    fn progress_of(&self, path: &str) -> (usize, usize) {
        self.progress.get(path).copied().unwrap_or_default()
    }

    fn next_unreviewed(&self, file: usize, from: usize) -> Option<usize> {
        let file = &self.files[file];
        let n = file.diff.hunks.len();
        (1..=n).map(|i| (from + i) % n).find(|&i| !file.is_reviewed(i))
    }

    fn view(&mut self, file: usize) -> &mut View {
        self.views.entry(self.files[file].diff.path.clone()).or_default()
    }

    fn select(&mut self, file: usize, hunk: usize) {
        self.make_current(file, hunk);
        self.view(file).scroll_to_current = true;
    }

    /// Selects the next hunk of the file that isn't reviewed yet. When there is none, selects
    /// the first such hunk of the next file in the tree and returns that file.
    fn next(&mut self, file: usize) -> Option<String> {
        let current = self.view(file).current;
        if let Some(hunk) = self.next_unreviewed(file, current) {
            self.select(file, hunk);
            return None;
        }
        let files = self.tree.files();
        let at = files.iter().position(|path| *path == self.files[file].diff.path).unwrap_or(0);
        let next = (1..files.len()).map(|i| &files[(at + i) % files.len()]).find(|path| {
            let (done, total) = self.progress_of(path);
            done < total
        })?;
        let (next, index) = (next.clone(), self.by_path[next]);
        let first = self.next_unreviewed(index, self.files[index].diff.hunks.len().saturating_sub(1)).unwrap_or(0);
        self.select(index, first);
        Some(next)
    }

    /// Stages the changed lines of a file that are `chosen`, by the index of their hunk and their
    /// index in it, and unstages the others.
    fn stage(&mut self, file: usize, chosen: impl Fn(usize, usize) -> bool) {
        let FileReview { diff, staged, .. } = &mut self.files[file];
        let result = self.repo.stage_lines(diff, &chosen);
        if result.is_ok() {
            for (h, (hunk, staged)) in diff.hunks.iter().zip(staged).enumerate() {
                for (i, (line, staged)) in hunk.lines.iter().zip(staged).enumerate() {
                    *staged = line.kind == LineKind::Context || chosen(h, i);
                }
            }
        }
        self.staging_done(result);
    }

    /// Stages or unstages all of a file without changed lines.
    fn stage_file(&mut self, file: usize, staged: bool) {
        let result = self.repo.stage_file(&self.files[file].diff.path, staged);
        if result.is_ok() {
            self.files[file].file_staged = staged;
        }
        self.staging_done(result);
    }

    fn staging_done(&mut self, result: Result<(), String>) {
        self.error = result.err();
        self.staged_changed = true;
        self.update_progress();
    }

    /// Marks a hunk as reviewed, or not when it is; moving on to the next one when it is
    /// reviewed, which may be in another file that is returned then.
    fn toggle_reviewed(&mut self, file: usize, hunk: usize) -> Option<String> {
        let reviewed = !self.files[file].is_reviewed(hunk);
        let staged = self.files[file].staged.clone();
        self.stage(file, |h, i| if h == hunk { reviewed } else { staged[h][i] });
        self.make_current(file, hunk);
        if reviewed && self.files[file].is_reviewed(hunk) { self.next(file) } else { None }
    }

    /// Marks one changed line of a hunk as reviewed, or not when it is.
    fn toggle_line(&mut self, file: usize, hunk: usize, line: usize) {
        let staged = self.files[file].staged.clone();
        self.stage(file, |h, i| staged[h][i] != ((h, i) == (hunk, line)));
        self.make_current(file, hunk);
    }

    /// Marks all changes of a file, or of the files in a directory, as reviewed or not.
    fn set_reviewed(&mut self, path: &str, reviewed: bool) {
        let inside = |file: &str| file.strip_prefix(path).is_some_and(|rest| rest.is_empty() || rest.starts_with('/'));
        let files: Vec<usize> = (0..self.files.len()).filter(|&index| inside(&self.files[index].diff.path)).collect();
        for file in files {
            (0..self.files[file].diff.hunks.len()).for_each(|hunk| self.see(file, hunk));
            let (done, total) = self.files[file].progress();
            if done == if reviewed { total } else { 0 } {
                continue;
            }
            if self.files[file].diff.hunks.is_empty() {
                self.stage_file(file, reviewed);
            } else {
                self.stage(file, |_, _| reviewed);
            }
        }
    }

    /// Handles the review keys. Returns the file to show next, when moving on to another one.
    fn handle_keys(&mut self, ctx: &egui::Context, file: usize) -> Option<String> {
        if ctx.memory(|m| m.focused().is_some()) {
            return None;
        }
        let pressed = |key| ctx.input(|i| i.modifiers.is_none() && i.key_pressed(key));
        if pressed(Key::N) {
            return self.next(file);
        }
        let count = self.files[file].diff.hunks.len();
        if count == 0 {
            let reviewed = !self.files[file].file_staged;
            if pressed(Key::Space) {
                self.stage_file(file, reviewed);
                return if reviewed { self.next(file) } else { None };
            }
            return None;
        }
        let current = self.view(file).current;
        if pressed(Key::J) && current + 1 < count {
            self.select(file, current + 1);
        }
        if pressed(Key::K) && current > 0 {
            self.select(file, current - 1);
        }
        if pressed(Key::Space) {
            return self.toggle_reviewed(file, current);
        }
        None
    }

    /// Shows the tree of changed files with their review progress. Returns the file that was clicked.
    pub fn tree_ui(&mut self, ui: &mut egui::Ui, collapsed: &mut BTreeSet<String>, selected: Option<&str>) -> Option<String> {
        if self.files.is_empty() {
            ui.weak("No changes");
            return None;
        }
        let (added, deleted) = status_colors(ui.visuals().dark_mode);
        let weak = ui.visuals().weak_text_color();
        let mut mark = None;
        let Self { tree, files, by_path, progress, unseen, .. } = self;
        let progress_of = |path: &str| progress.get(path).copied().unwrap_or_default();
        let mut menu = |ui: &mut egui::Ui, path: &str| {
            let (done, total) = progress_of(path);
            if done < total && ui.button("Mark as reviewed").clicked() {
                mark = Some((path.to_string(), true));
            }
            if done > 0 && ui.button("Mark as not reviewed").clicked() {
                mark = Some((path.to_string(), false));
            }
        };
        let look = |path: &str| {
            let (done, total) = progress_of(path);
            let color = match by_path.get(path).map(|&index| files[index].diff.status) {
                _ if done == total => Some(weak),
                Some(FileStatus::Added) => Some(added),
                Some(FileStatus::Deleted) => Some(deleted),
                _ => None,
            };
            FileLook { color, detail: (total > 0).then(|| format!("{done}/{total}")), dot: unseen.contains(path).then_some(NEW_COLOR) }
        };
        let clicked = tree.ui(ui, collapsed, selected, look, Some(&mut menu));
        if let Some((path, reviewed)) = mark {
            self.set_reviewed(&path, reviewed);
        }
        clicked
    }

    /// Shows the changes of one file, taking the review keys when `active`. Comments and
    /// questions go to one of the `agents`.
    pub fn ui(&mut self, ui: &mut egui::Ui, path: Option<&str>, active: bool, agents: &[Agent]) -> Option<Outcome> {
        let Some(path) = path else {
            ui.centered_and_justified(|ui| ui.weak("Choose a file under Changes to review it"));
            return None;
        };
        let Some(&index) = self.by_path.get(path) else {
            ui.centered_and_justified(|ui| ui.weak(format!("No changes in {path}")));
            return None;
        };
        let mut next = if active { self.handle_keys(ui.ctx(), index) } else { None };
        let mut outcome = None;
        let (done, total) = self.progress_of(path);
        let Self { repo, files, views, texts, error, comments, .. } = self;
        let file = &files[index];
        let text = texts.entry(path.to_string()).or_insert_with(|| FileText::new(&repo.root, &file.diff));
        let view = views.entry(path.to_string()).or_default();
        let gaps = gaps(&file.diff.hunks, text.new.lines.len());
        view.expanded.resize(gaps.len(), false);

        let mut mark_file = None;
        let unsent = comments.iter().filter(|comment| !comment.sent).count();
        ui.horizontal(|ui| {
            ui.label(RichText::new(file.diff.display_name()).monospace().strong());
            ui.weak(format!("{done}/{total} reviewed"));
            if done < total && ui.small_button("Mark file reviewed").clicked() {
                mark_file = Some(true);
            }
            if done > 0 && done == total && ui.small_button("Mark file not reviewed").clicked() {
                mark_file = Some(false);
            }
            let label = if unsent == 1 { "Send 1 comment".to_string() } else { format!("Send {unsent} comments") };
            if unsent > 0 && let Some(terminal) = send_button(ui, &label, true, agents) {
                let text = comments::feedback(comments.iter().filter(|comment| !comment.sent));
                comments.iter_mut().for_each(|comment| comment.sent = true);
                outcome = Some(Outcome::Send { terminal, text });
            }
        });
        if let Some(error) = error {
            ui.colored_label(ui.visuals().error_fg_color, error.as_str());
        }
        ui.add_space(4.0);
        let draft = if view.selection.is_some() { egui::Panel::bottom(("draft", path)).show(ui, |ui| draft_ui(ui, egui::Id::unique(("draft", path)), view, agents)).inner } else { None };
        let file_comments: Vec<(usize, &Comment)> = comments.iter().enumerate().filter(|(_, comment)| comment.path == path).collect();
        let (clicked, lines) = egui::ScrollArea::both()
            .id_salt(path)
            .auto_shrink(false)
            .show(ui, |ui| {
                if file.diff.binary {
                    ui.weak("Binary file");
                }
                rows_ui(ui, file, text, &gaps, view, &file_comments)
            })
            .inner;
        view.scroll_to_current = false;
        match draft {
            Some(Draft::Comment) if !lines.is_empty() => {
                comments.push(Comment::new(path, lines, &file_lines(&text.content, &file.diff), &view.draft));
                view.draft.clear();
                view.selection = None;
            }
            Some(Draft::Ask(terminal)) if !lines.is_empty() => {
                outcome = Some(Outcome::Send { terminal, text: comments::quote(path, &lines, &view.draft) });
                view.draft.clear();
                view.selection = None;
            }
            Some(Draft::Cancel) => view.selection = None,
            _ => {}
        }
        match clicked {
            Some(Clicked::Row(hunk)) => self.make_current(index, hunk),
            Some(Clicked::Hunk(hunk)) => next = self.toggle_reviewed(index, hunk),
            Some(Clicked::Line(hunk, line)) => self.toggle_line(index, hunk, line),
            Some(Clicked::DeleteComment(comment)) => {
                self.comments.remove(comment);
                self.view(index).selection = None;
            }
            None => {}
        }
        if let Some(reviewed) = mark_file {
            self.set_reviewed(path, reviewed);
            if reviewed {
                next = self.next(index);
            }
        }
        next.map(Outcome::Show).or(outcome)
    }
}

/// What is done with the comment written on the selected rows.
enum Draft {
    Comment,
    /// Ask the agent in a terminal to act on it right away.
    Ask(u64),
    Cancel,
}

/// The box for a comment on the selected rows.
fn draft_ui(ui: &mut egui::Ui, id: egui::Id, view: &mut View, agents: &[Agent]) -> Option<Draft> {
    let focused = ui.memory(|m| m.has_focus(id));
    // taken before the text field would make it a line break
    let submit = focused && ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Enter));
    ui.add_space(4.0);
    let edit = egui::TextEdit::multiline(&mut view.draft).id(id).desired_rows(3).desired_width(f32::INFINITY).hint_text("A comment on the selected lines, or what an agent should change there");
    let response = ui.add(edit);
    if std::mem::take(&mut view.focus_draft) {
        response.request_focus();
    }
    let ready = !view.draft.trim().is_empty();
    let mut draft = None;
    ui.horizontal(|ui| {
        if ui.add_enabled(ready, egui::Button::new("Comment")).on_hover_text("Cmd+Enter").clicked() || (ready && submit) {
            draft = Some(Draft::Comment);
        }
        if let Some(terminal) = send_button(ui, "Ask agent", ready, agents) {
            draft = Some(Draft::Ask(terminal));
        }
        if ui.button("Cancel").on_hover_text("Esc").clicked() || ((focused || response.lost_focus()) && ui.input(|i| i.key_pressed(Key::Escape))) {
            draft = Some(Draft::Cancel);
        }
    });
    ui.add_space(4.0);
    draft
}

/// A button that sends something to an agent: to the only one right away, or to the one chosen
/// from its menu when there are several. Returns the terminal of the agent.
fn send_button(ui: &mut egui::Ui, label: &str, enabled: bool, agents: &[Agent]) -> Option<u64> {
    match agents {
        [] => {
            ui.add_enabled(false, egui::Button::new(label)).on_disabled_hover_text("Open a terminal with an agent first");
            None
        }
        [(terminal, title)] => ui.add_enabled(enabled, egui::Button::new(label)).on_hover_text(format!("Send to {title}")).clicked().then_some(*terminal),
        _ => {
            let mut chosen = None;
            ui.add_enabled_ui(enabled, |ui| {
                ui.menu_button(label, |ui| {
                    for (terminal, title) in agents {
                        if ui.button(title).clicked() {
                            chosen = Some(*terminal);
                        }
                    }
                })
            });
            chosen
        }
    }
}

/// The colors of the names of added and deleted files.
fn status_colors(dark_mode: bool) -> (Color32, Color32) {
    if dark_mode { (Color32::from_rgb(110, 190, 120), Color32::from_rgb(220, 110, 110)) } else { (Color32::from_rgb(30, 130, 50), Color32::from_rgb(190, 40, 40)) }
}

/// The background of added or removed rows, and of the parts that changed within them.
fn row_colors(kind: LineKind, dark_mode: bool) -> Option<(Color32, Color32)> {
    let rgb = |[r, g, b]: [u8; 3]| Color32::from_rgb(r, g, b);
    let colors = match (kind, dark_mode) {
        (LineKind::Context, _) => return None,
        (LineKind::Added, true) => ([28, 58, 34], [42, 100, 54]),
        (LineKind::Removed, true) => ([70, 30, 32], [125, 45, 48]),
        (LineKind::Added, false) => ([218, 251, 225], [168, 236, 184]),
        (LineKind::Removed, false) => ([255, 226, 226], [255, 186, 186]),
    };
    Some((rgb(colors.0), rgb(colors.1)))
}

/// The lines of a changed file on disk.
fn read_lines(root: &Path, diff: &FileDiff) -> Vec<String> {
    match diff.status {
        FileStatus::Deleted => Vec::new(),
        _ => file_view::read(&root.join(&diff.path)).map(|text| text.lines().map(str::to_string).collect()).unwrap_or_default(),
    }
}

/// An unchanged line of a file, by its gap and its number in the new file.
fn unchanged_line(content: &[String], gap: &Gap, number: usize) -> DiffLine {
    let text = content.get(number - 1).cloned().unwrap_or_default();
    DiffLine { kind: LineKind::Context, old_no: u32::try_from(number as isize + gap.old_offset).ok(), new_no: Some(number as u32), text }
}

/// All lines of a changed file in the order the review shows them: the unchanged ones from
/// the `content` on disk, the changed ones from the diff.
fn file_lines(content: &[String], diff: &FileDiff) -> Vec<DiffLine> {
    let mut lines = Vec::new();
    for (index, gap) in gaps(&diff.hunks, content.len()).iter().enumerate() {
        lines.extend(gap.lines.clone().map(|number| unchanged_line(content, gap, number)));
        lines.extend(diff.hunks.get(index).into_iter().flat_map(|hunk| hunk.lines.iter().cloned()));
    }
    lines
}

impl FileText {
    /// The text of a changed file as it is now and as it was. Where the diff doesn't hold the
    /// unchanged lines, they are taken from the file on disk.
    fn new(root: &Path, diff: &FileDiff) -> Self {
        let path = root.join(&diff.path);
        let content = read_lines(root, diff);
        let lines = file_lines(&content, diff);
        let side = |has: fn(&DiffLine) -> bool| Side::new(&path, &lines.iter().filter(|line| has(line)).map(|line| line.text.as_str()).collect::<Vec<_>>());
        let (new, old) = (side(|line| line.new_no.is_some()), side(|line| line.old_no.is_some()));
        let changed = diff.hunks.iter().map(|hunk| changed_in_lines(hunk, &new, &old)).collect();
        let longest = new.lines.iter().chain(&old.lines).map(|line| line.chars().count()).max().unwrap_or(0);
        Self { content, new, old, changed, longest }
    }

    /// A row of the diff for a line of the old or the new file.
    fn row<'a>(&'a self, line: &DiffLine, changed: &'a [Range<usize>]) -> Line<'a> {
        let side = if line.kind == LineKind::Removed { &self.old } else { &self.new };
        let (text, spans) = side.line(if line.kind == LineKind::Removed { line.old_no } else { line.new_no });
        Line { kind: line.kind, old_no: line.old_no, new_no: line.new_no, text, spans, changed }
    }
}

impl Side {
    fn new(path: &Path, lines: &[&str]) -> Self {
        let mut lines: Vec<String> = lines.iter().map(|line| expand_tabs(line)).collect();
        // colored before being cut, so that what comes after a cut is colored as it should
        let mut spans = Highlighter::spans_of(path, &lines);
        for (line, spans) in lines.iter_mut().zip(&mut spans) {
            if let Some((cut, _)) = line.char_indices().nth(MAX_LINE_CHARS) {
                line.truncate(cut);
                line.push('…');
                spans.retain(|(range, _)| range.start < cut);
                for (range, _) in spans.iter_mut() {
                    range.end = range.end.min(cut);
                }
            }
        }
        Self { lines, spans }
    }

    /// The text of a line by its number, and the parts of it to color.
    fn line(&self, number: Option<u32>) -> (&str, &[(Range<usize>, &'static str)]) {
        let index = number.and_then(|number| (number as usize).checked_sub(1)).unwrap_or(usize::MAX);
        (self.lines.get(index).map_or("", String::as_str), self.spans.get(index).map_or(&[], Vec::as_slice))
    }
}

/// The parts that changed within each line of a hunk. Removed lines are compared with the
/// added lines that follow them, the first with the first and so on.
fn changed_in_lines(hunk: &Hunk, new: &Side, old: &Side) -> Vec<Ranges> {
    let lines = &hunk.lines;
    let mut changed = vec![Vec::new(); lines.len()];
    let mut at = 0;
    while at < lines.len() {
        let removed = lines[at..].iter().take_while(|line| line.kind == LineKind::Removed).count();
        let added = lines[at + removed..].iter().take_while(|line| line.kind == LineKind::Added).count();
        for i in 0..removed.min(added) {
            let (r, a) = (at + i, at + removed + i);
            if let Some((in_old, in_new)) = word_diff::changed_ranges(old.line(lines[r].old_no).0, new.line(lines[a].new_no).0) {
                (changed[r], changed[a]) = (in_old, in_new);
            }
        }
        at += (removed + added).max(1);
    }
    changed
}

/// The unchanged lines before each hunk and after the last one, in a file of `line_count` lines.
fn gaps(hunks: &[Hunk], line_count: usize) -> Vec<Gap> {
    if hunks.is_empty() {
        return Vec::new();
    }
    let mut gaps = Vec::new();
    let (mut old_end, mut new_end) = (1, 1);
    for hunk in hunks {
        let (old, new) = hunk.ranges();
        let (old, new) = (old.start as usize..old.end as usize, new.start as usize..new.end as usize);
        gaps.push(Gap { lines: new_end..new.start.max(new_end), old_offset: old.start as isize - new.start as isize });
        (old_end, new_end) = (old.end, new.end);
    }
    gaps.push(Gap { lines: new_end..(line_count + 1).max(new_end), old_offset: old_end as isize - new_end as isize });
    gaps
}

/// What a row of a review shows.
#[derive(Debug, PartialEq)]
enum Row {
    /// An unchanged line between the hunks: the index of its gap and its number in the new file.
    Unchanged(usize, usize),
    /// Lines of a gap that are folded away: the index of the gap and how many lines.
    Folded(usize, usize),
    /// A line of a hunk: the index of the hunk and of the line in it.
    Hunk(usize, usize),
    /// A line of the text of a comment: the index of the comment and of the line.
    Comment(usize, usize),
}

/// A comment shown after the last line it is on: the kind and number of that line, the index
/// of the comment and how many rows its text takes.
struct CommentRows {
    kind: LineKind,
    number: u32,
    comment: usize,
    rows: usize,
}

/// The lines of a gap that are folded away when it isn't expanded: all but the ones next to the
/// hunks, unless that would be just one line.
fn folded(gaps: &[Gap], hunks: usize, gap: usize) -> Option<Range<usize>> {
    let start = if gap > 0 { CONTEXT } else { 0 };
    let end = if gap < hunks { CONTEXT } else { 0 };
    let lines = gaps[gap].lines.clone();
    (lines.len() > start + end + 1).then(|| lines.start + start..lines.end - end)
}

/// The rows of a file with its comments, and the first row of each hunk.
fn rows(gaps: &[Gap], hunks: &[Hunk], expanded: &[bool], comments: &[CommentRows]) -> (Vec<Row>, Vec<usize>) {
    let (mut rows, mut starts) = (Vec::new(), Vec::new());
    // a row, followed by the comments that end on one of the lines it shows
    let add = |rows: &mut Vec<Row>, row: Row, kind: LineKind, lines: Range<usize>| {
        rows.push(row);
        for comment in comments.iter().filter(|comment| comment.kind == kind && lines.contains(&(comment.number as usize))) {
            rows.extend((0..comment.rows).map(|line| Row::Comment(comment.comment, line)));
        }
    };
    for (index, gap) in gaps.iter().enumerate() {
        let unchanged = |rows: &mut Vec<Row>, lines: Range<usize>| lines.for_each(|number| add(rows, Row::Unchanged(index, number), LineKind::Context, number..number + 1));
        match folded(gaps, hunks.len(), index).filter(|_| !expanded[index]) {
            Some(folded) => {
                unchanged(&mut rows, gap.lines.start..folded.start);
                add(&mut rows, Row::Folded(index, folded.len()), LineKind::Context, folded.clone());
                unchanged(&mut rows, folded.end..gap.lines.end);
            }
            None => unchanged(&mut rows, gap.lines.clone()),
        }
        if let Some(hunk) = hunks.get(index) {
            starts.push(rows.len());
            for (at, line) in hunk.lines.iter().enumerate() {
                let number = line.number() as usize;
                add(&mut rows, Row::Hunk(index, at), line.kind, number..number + 1);
            }
        }
    }
    (rows, starts)
}

/// A row of a diff.
struct Line<'a> {
    kind: LineKind,
    old_no: Option<u32>,
    new_no: Option<u32>,
    text: &'a str,
    spans: &'a [(Range<usize>, &'static str)],
    /// The parts that changed within the line.
    changed: &'a [Range<usize>],
}

/// The width of the reviewed marks, the comment mark, the line numbers and the sign in front
/// of the code, in digits.
const GUTTER_DIGITS: usize = 21;

/// Where the comment mark is in the gutter, in digits.
const COMMENT_MARK: f32 = 4.4;

/// A mark in the gutter that was clicked.
enum Clicked {
    /// A hunk, which becomes the current one.
    Row(usize),
    /// The reviewed mark of a hunk.
    Hunk(usize),
    /// The reviewed mark of a changed line, by the index of its hunk and its index in it.
    Line(usize, usize),
    /// The mark that deletes a comment.
    DeleteComment(usize),
}

/// Shows the rows of a file with the `comments` on it as one piece of code, drawing only the
/// ones in sight, and lets rows be selected to comment on. Returns the mark that was clicked
/// and the lines selected, with the ones folded away between them.
fn rows_ui(ui: &mut egui::Ui, file: &FileReview, text: &FileText, gaps: &[Gap], view: &mut View, comments: &[(usize, &Comment)]) -> (Option<Clicked>, Vec<DiffLine>) {
    let hunks = &file.diff.hunks;
    let ends: Vec<CommentRows> = comments
        .iter()
        .map(|&(index, comment)| CommentRows { kind: comment.last_line().kind, number: comment.last_line().number(), comment: index, rows: comment.text.lines().count().max(1) })
        .collect();
    let (rows, starts) = rows(gaps, hunks, &view.expanded, &ends);
    let style = CodeStyle::new(ui);
    let (row_height, digit) = (style.row_height, style.digit_width);
    let width = digit * (GUTTER_DIGITS + text.longest + 2) as f32;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width().max(width), row_height * rows.len() as f32), Sense::click_and_drag());
    let row_at = |index: usize| egui::Rect::from_min_size(egui::pos2(rect.left(), rect.top() + row_height * index as f32), Vec2::new(rect.width(), row_height));
    let row_under = |y: f32| (((y - rect.top()) / row_height).max(0.0) as usize).min(rows.len().saturating_sub(1));
    if view.scroll_to_current && let Some(&start) = starts.get(view.current) {
        // with a few lines above the hunk
        ui.scroll_to_rect(row_at(start.saturating_sub(3)), Some(Align::TOP));
    }
    let mut clicked = None;
    // a click on a hunk makes it the current one
    if response.clicked()
        && let Some(pointer) = response.interact_pointer_pos()
        && let Some(Row::Hunk(hunk, _)) = rows.get(row_under(pointer.y))
    {
        clicked = Some(Clicked::Row(*hunk));
    }
    let clip = ui.clip_rect();
    let first = ((clip.top() - rect.top()) / row_height).floor().max(0.0) as usize;
    let last = (((clip.bottom() - rect.top()) / row_height).ceil().max(0.0) as usize).min(rows.len());
    // the gutter stays at the left edge when the code scrolls sideways, and covers it
    let gutter_left = rect.left().max(clip.left());
    let column = |digits: f32, row: egui::Rect| egui::Rect::from_min_size(egui::pos2(gutter_left + digit * digits, row.top()), Vec2::splat(row_height));

    // dragging from the comment mark selects rows
    let is_line = |row: &Row| !matches!(row, Row::Comment(..));
    if response.drag_started()
        && let Some(origin) = ui.input(|i| i.pointer.press_origin())
        && column(COMMENT_MARK, row_at(0)).x_range().contains(origin.x)
        && rows.get(row_under(origin.y)).is_some_and(is_line)
    {
        view.selection = Some((row_under(origin.y), row_under(origin.y)));
        view.selecting = true;
    }
    if view.selecting {
        if let (Some((start, _)), Some(pointer)) = (view.selection, response.interact_pointer_pos()) {
            view.selection = Some((start, row_under(pointer.y)));
        }
        view.selecting = response.dragged();
        view.focus_draft = !view.selecting;
    }
    let selected = view.selection.map(|(start, end)| start.min(end)..=start.max(end));

    let hovered = ui.input(|i| i.pointer.hover_pos()).filter(|pointer| rect.contains(*pointer) && clip.contains(*pointer)).map(|pointer| row_under(pointer.y));
    for (index, row) in rows.iter().enumerate().take(last).skip(first) {
        let row_rect = row_at(index);
        match *row {
            Row::Folded(gap, hidden) => {
                let visible = egui::Rect::from_x_y_ranges(gutter_left..=clip.right(), row_rect.y_range());
                if fold_ui(ui, visible, hidden, response.id.with(("fold", gap))) {
                    view.expanded[gap] = true;
                    view.selection = None;
                }
            }
            Row::Unchanged(gap, number) => {
                let (line, spans) = text.new.line(Some(number as u32));
                let old_no = u32::try_from(number as isize + gaps[gap].old_offset).ok();
                let line = Line { kind: LineKind::Context, old_no, new_no: Some(number as u32), text: line, spans, changed: &[] };
                line_ui(ui, &style, row_rect, gutter_left, &line, false);
            }
            Row::Hunk(hunk, at) => {
                let line = &hunks[hunk].lines[at];
                let staged = file.staged[hunk][at];
                line_ui(ui, &style, row_rect, gutter_left, &text.row(line, &text.changed[hunk][at]), staged);
                if hunk == view.current {
                    let bar = egui::Rect::from_min_size(egui::pos2(gutter_left, row_rect.top()), Vec2::new(3.0, row_height));
                    ui.painter().rect_filled(bar, 0.0, ui.visuals().selection.stroke.color);
                }
                // a new hunk has a bar of its own between the line numbers and the code
                if file.new[hunk] {
                    let bar = egui::Rect::from_min_size(egui::pos2(gutter_left + digit * 19.7, row_rect.top()), Vec2::new(2.0, row_height));
                    ui.painter().rect_filled(bar, 0.0, NEW_COLOR);
                }
                let (done, total) = file.hunk_progress(hunk);
                if at == 0 {
                    let state = if done == total { Some(true) } else if done == 0 { Some(false) } else { None };
                    if reviewed_mark_ui(ui, column(0.5, row_rect), 0.0, state, "Reviewed", response.id.with(("hunk", hunk))) {
                        clicked = Some(Clicked::Hunk(hunk));
                    }
                }
                // the marks of single lines show up under the pointer, or when they differ within the hunk
                let partly = done > 0 && done < total;
                let shown = (hovered == Some(index) || partly) && line.kind != LineKind::Context;
                if shown && reviewed_mark_ui(ui, column(2.7, row_rect), 2.0, Some(staged), "Reviewed line", response.id.with(("line", hunk, at))) {
                    clicked = Some(Clicked::Line(hunk, at));
                }
            }
            Row::Comment(index, line) => {
                let Some((_, comment)) = comments.iter().find(|(at, _)| *at == index) else { continue };
                let visible = egui::Rect::from_x_y_ranges(gutter_left..=clip.right(), row_rect.y_range());
                comment_line_ui(ui, &style, visible, comment, line);
                if line == 0 && delete_mark_ui(ui, column(0.5, row_rect), response.id.with(("delete comment", index))) {
                    clicked = Some(Clicked::DeleteComment(index));
                }
            }
        }
        if selected.as_ref().is_some_and(|selected| selected.contains(&index)) {
            ui.painter().rect_filled(row_rect, 0.0, ui.visuals().selection.bg_fill.gamma_multiply(0.4));
        }
        if hovered == Some(index) && !view.selecting && matches!(row, Row::Unchanged(..) | Row::Hunk(..)) && comment_mark_ui(ui, column(COMMENT_MARK, row_rect), response.id.with(("comment", index))) {
            let shift = ui.input(|i| i.modifiers.shift);
            view.selection = Some(match view.selection {
                Some((start, _)) if shift => (start, index),
                _ => (index, index),
            });
            view.focus_draft = true;
        }
    }
    let selected = view.selection.and_then(|(start, end)| rows.get(start.min(end)..=start.max(end)));
    let lines = selected.unwrap_or_default().iter().flat_map(|row| match *row {
        Row::Unchanged(gap, number) => vec![unchanged_line(&text.content, &gaps[gap], number)],
        Row::Folded(gap, _) => folded(gaps, hunks.len(), gap).unwrap_or_default().map(|number| unchanged_line(&text.content, &gaps[gap], number)).collect(),
        Row::Hunk(hunk, at) => vec![hunks[hunk].lines[at].clone()],
        Row::Comment(..) => Vec::new(),
    });
    (clicked, lines.collect())
}

/// Draws a line of code with its gutter of line numbers at `gutter_left`. The colors of a
/// reviewed line are faded, to keep the focus on what is left.
fn line_ui(ui: &egui::Ui, style: &CodeStyle, row: egui::Rect, gutter_left: f32, line: &Line, reviewed: bool) {
    let painter = ui.painter();
    let dark = ui.visuals().dark_mode;
    let fade = |color: Color32| if reviewed { color.gamma_multiply(0.4) } else { color };
    let colors = row_colors(line.kind, dark).map(|(bg, mark)| (fade(bg), fade(mark)));
    if let Some((bg, _)) = colors {
        painter.rect_filled(row, 0.0, bg);
    }
    let mark = colors.map_or(Color32::TRANSPARENT, |(_, mark)| mark);
    let job = highlight::layout_job(line.text, line.spans, line.changed, mark, &style.font, style.text_color, dark);
    let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
    let digit = style.digit_width;
    painter.galley(egui::pos2(row.left() + digit * GUTTER_DIGITS as f32, row.top() + 1.0), galley, style.text_color);

    let gutter = egui::Rect::from_min_size(egui::pos2(gutter_left, row.top()), Vec2::new(digit * (GUTTER_DIGITS - 1) as f32, row.height()));
    painter.rect_filled(gutter, 0.0, ui.visuals().panel_fill);
    if let Some((bg, _)) = colors {
        painter.rect_filled(gutter, 0.0, bg);
    }
    let y = row.top() + 1.0;
    style.number(painter, gutter_left + digit * 12.0, y, line.old_no);
    style.number(painter, gutter_left + digit * 17.0, y, line.new_no);
    let sign = match line.kind {
        LineKind::Context => " ",
        LineKind::Added => "+",
        LineKind::Removed => "-",
    };
    style.text(painter, gutter_left + digit * 18.5, y, sign, style.weak_color);
}

/// The background of comments, and the bar at their left edge.
fn comment_colors(dark_mode: bool) -> (Color32, Color32) {
    if dark_mode { (Color32::from_rgb(58, 54, 38), Color32::from_rgb(200, 170, 80)) } else { (Color32::from_rgb(255, 247, 214), Color32::from_rgb(214, 170, 40)) }
}

/// Draws a line of the text of a comment across the visible part of its row. A comment sent to
/// an agent already is faded.
fn comment_line_ui(ui: &egui::Ui, style: &CodeStyle, row: egui::Rect, comment: &Comment, line: usize) {
    let painter = ui.painter();
    let (bg, bar) = comment_colors(ui.visuals().dark_mode);
    let fade = |color: Color32| if comment.sent { color.gamma_multiply(0.5) } else { color };
    painter.rect_filled(row, 0.0, fade(bg));
    let left = row.left() + style.digit_width * GUTTER_DIGITS as f32;
    painter.rect_filled(egui::Rect::from_min_size(egui::pos2(left - style.digit_width, row.top()), Vec2::new(3.0, row.height())), 0.0, fade(bar));
    let font = egui::TextStyle::Body.resolve(ui.style());
    let text = comment.text.lines().nth(line).unwrap_or_default();
    let end = painter.text(egui::pos2(left, row.center().y), egui::Align2::LEFT_CENTER, text, font.clone(), fade(style.text_color));
    if line == 0 && comment.sent {
        painter.text(egui::pos2(end.right() + 12.0, row.center().y), egui::Align2::LEFT_CENTER, "sent", font, style.weak_color);
    }
}

/// A mark in the gutter that is clicked to act on a row: it shows its `sign` and is labeled
/// `label`. Returns true when clicked.
fn gutter_mark_ui(ui: &egui::Ui, rect: egui::Rect, sign: &str, label: &str, id: egui::Id) -> bool {
    let response = ui.interact(rect, id, Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), label));
    let visuals = ui.style().interact(&response);
    let mark = rect.shrink(2.0);
    ui.painter().rect(mark, 3.0, visuals.bg_fill, visuals.bg_stroke, egui::StrokeKind::Inside);
    ui.painter().text(mark.center(), egui::Align2::CENTER_CENTER, sign, egui::TextStyle::Body.resolve(ui.style()), visuals.text_color());
    response.on_hover_text(label).clicked()
}

/// The mark that selects a row to comment on; dragged, it selects more rows.
fn comment_mark_ui(ui: &egui::Ui, rect: egui::Rect, id: egui::Id) -> bool {
    gutter_mark_ui(ui, rect, "+", "Comment on this line", id)
}

fn delete_mark_ui(ui: &egui::Ui, rect: egui::Rect, id: egui::Id) -> bool {
    gutter_mark_ui(ui, rect, "×", "Delete comment", id)
}

/// A wavy line in place of unchanged lines that are folded away. Returns true when clicked,
/// to show them.
fn fold_ui(ui: &egui::Ui, rect: egui::Rect, hidden: usize, id: egui::Id) -> bool {
    let label = format!("Show {hidden} hidden lines");
    let response = ui.interact(rect, id, Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), &label));
    let color = if response.hovered() { ui.visuals().text_color() } else { ui.visuals().weak_text_color().gamma_multiply(0.6) };
    let (amplitude, wavelength) = (2.0, 8.0);
    let points = (0..=(rect.width() / 2.0) as usize)
        .map(|step| {
            let x = step as f32 * 2.0;
            egui::pos2(rect.left() + x, rect.center().y + amplitude * (x / wavelength * std::f32::consts::TAU).sin())
        })
        .collect();
    ui.painter().add(egui::Shape::line(points, Stroke::new(1.0, color)));
    response.on_hover_text(label).clicked()
}

/// A reviewed mark: ticked, empty, or with a dash when `reviewed` is `None` as only some of
/// the lines it is for are reviewed. It is made smaller by `shrink`. Returns true when clicked.
fn reviewed_mark_ui(ui: &egui::Ui, rect: egui::Rect, shrink: f32, reviewed: Option<bool>, label: &str, id: egui::Id) -> bool {
    let response = ui.interact(rect, id, Sense::click());
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, ui.is_enabled(), reviewed == Some(true), label));
    let visuals = ui.style().interact(&response);
    let mark = egui::Rect::from_center_size(rect.center(), Vec2::splat(rect.height() - 5.0 - shrink));
    ui.painter().rect(mark, 2.0, visuals.bg_fill, visuals.bg_stroke, egui::StrokeKind::Inside);
    let at = |x: f32, y: f32| egui::pos2(mark.left() + mark.width() * x, mark.top() + mark.height() * y);
    let sign = match reviewed {
        Some(true) => vec![at(0.2, 0.5), at(0.45, 0.75), at(0.8, 0.25)],
        Some(false) => return response.clicked(),
        None => vec![at(0.25, 0.5), at(0.75, 0.5)],
    };
    ui.painter().add(egui::Shape::line(sign, visuals.fg_stroke));
    response.clicked()
}

#[cfg(test)]
mod tests {
    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT as _, Queryable as _};

    use std::process::Command;

    use super::*;
    use crate::git::tests::{commit_all, temp_repo};
    use crate::testing::TempDir;

    const DIFF: &str = "\
diff --git a/a.txt b/a.txt
--- a/a.txt
+++ b/a.txt
@@ -1 +1 @@
-one
+two
@@ -9 +9 @@
-three
+four
diff --git a/b.txt b/b.txt
--- a/b.txt
+++ b/b.txt
@@ -1 +1 @@
-five
+six
";

    /// A review of the files changed from their first version, which is committed, to the second.
    fn review(name: &str, files: &[(&str, &str, &str)]) -> (Review, TempDir) {
        let (repo, dir) = temp_repo(name, &files.iter().map(|(path, old, _)| (*path, *old)).collect::<Vec<_>>());
        commit_all(&repo);
        for (path, _, new) in files {
            dir.write(path, new);
        }
        let mut review = Review::new(repo);
        refresh(&mut review);
        (review, dir)
    }

    /// Takes over the changes in the working tree and the index, as the editor does when they change.
    fn refresh(review: &mut Review) {
        let changes = review.repo.changes().unwrap();
        review.set_changes(&changes, &BTreeSet::new());
    }

    const TEN: &str = "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n";
    /// Two hunks in a.txt, one in b.txt.
    const TWO_FILES: &[(&str, &str, &str)] = &[("a.txt", TEN, "one\n2\n3\n4\n5\n6\n7\n8\nnine\n10\n"), ("b.txt", "x\n", "y\n")];

    #[test]
    fn marking_a_hunk_reviewed_stages_it_and_moves_on_to_the_next_unreviewed_one() {
        let (mut review, dir) = review("moves-on", TWO_FILES);
        assert_eq!(review.progress(), (0, 6));
        assert_eq!(review.toggle_reviewed(0, 0), None);
        assert_eq!((review.progress(), review.view(0).current), ((2, 6), 1));
        // the last hunk of a file moves on to the next file
        assert_eq!(review.toggle_reviewed(0, 1), Some("b.txt".to_string()));
        assert_eq!((review.progress_of("a.txt"), review.view(0).current), ((4, 4), 1));
        assert!(review.take_staged_changed());

        // what is staged is what is reviewed
        refresh(&mut review);
        assert_eq!(review.progress(), (4, 6));
        let in_index = Command::new("git").args(["show", ":a.txt"]).current_dir(dir.path()).output().unwrap().stdout;
        assert_eq!(String::from_utf8(in_index).unwrap(), TWO_FILES[0].2);
        review.toggle_reviewed(0, 1);
        refresh(&mut review);
        assert_eq!(review.progress(), (2, 6));
    }

    #[test]
    fn marks_single_lines_of_a_hunk_reviewed() {
        let (mut review, _dir) = review("lines", TWO_FILES);
        // only the removal of the replaced line
        review.toggle_line(0, 0, 0);
        refresh(&mut review);
        assert_eq!((review.progress_of("a.txt"), review.files[0].hunk_progress(0)), ((1, 4), (1, 2)));
        assert!(!review.files[0].is_reviewed(0));
        // the whole hunk once the rest of it is marked too
        review.toggle_reviewed(0, 0);
        refresh(&mut review);
        assert!(review.files[0].is_reviewed(0));
        review.toggle_line(0, 0, 1);
        refresh(&mut review);
        assert_eq!(review.files[0].staged[0][..2], [true, false]);
    }

    #[test]
    fn each_run_of_changed_lines_is_a_hunk_with_marks_for_single_lines_under_the_pointer() {
        let (mut review, _dir) = review("runs", &[("a.txt", TEN, "1\ntwo\n3\nfour\n5\n6\n7\n8\n9\n10\n")]);
        assert_eq!(review.files[0].diff.hunks.len(), 2);
        review.toggle_line(0, 1, 0);
        let mut harness = Harness::builder().with_size(egui::vec2(800.0, 600.0)).build_ui_state(|ui, review: &mut Review| { review.ui(ui, Some("a.txt"), true, &[]); }, review);
        harness.run();
        // a hunk with only some of its lines reviewed shows the marks of its lines
        assert_eq!(harness.query_all_by_label("Reviewed").count(), 2);
        assert_eq!(harness.query_all_by_label("Reviewed line").count(), 2);
        harness.get_all_by_label("Reviewed").next().unwrap().hover();
        harness.run();
        assert_eq!(harness.query_all_by_label("Reviewed line").count(), 3);
    }

    /// A review of a.txt, in which the line 2 is replaced, shown with an agent to send to. Keeps
    /// what the review asked for last.
    fn comment_harness(name: &str) -> (Harness<'static, (Review, Option<Outcome>)>, TempDir) {
        let (review, dir) = review(name, &[("a.txt", TEN, &TEN.replace("2\n", "two\n"))]);
        let show = |ui: &mut egui::Ui, (review, outcome): &mut (Review, Option<Outcome>)| {
            if let Some(asked) = review.ui(ui, Some("a.txt"), true, &[(1, "agent".to_string())]) {
                *outcome = Some(asked);
            }
        };
        let mut harness = Harness::builder().with_size(egui::vec2(800.0, 600.0)).build_ui_state(show, (review, None));
        harness.run();
        (harness, dir)
    }

    /// The middle of a row of the review, by how far below the first line of the hunk it is.
    fn row_center(harness: &Harness<'_, (Review, Option<Outcome>)>, below_hunk: f32) -> egui::Pos2 {
        let mark = harness.get_by_label("Reviewed").rect();
        mark.center() + egui::vec2(300.0, below_hunk * mark.height())
    }

    fn comment_on_row(harness: &mut Harness<'_, (Review, Option<Outcome>)>, below_hunk: f32, text: &str) {
        harness.event(egui::Event::PointerMoved(row_center(harness, below_hunk)));
        harness.run();
        harness.get_by_label("Comment on this line").click();
        harness.run();
        // the comment box has the keyboard
        harness.event(egui::Event::Text(text.to_string()));
        harness.run();
    }

    #[test]
    fn comments_on_a_line_until_it_changes() {
        let (mut harness, dir) = comment_harness("comment");
        comment_on_row(&mut harness, 1.0, "Use 2");
        harness.get_by_label("Comment").click();
        harness.run();
        let comments = harness.state().0.comments().to_vec();
        assert_eq!(comments.iter().map(|comment| (comment.lines[0].text.as_str(), comment.text.as_str())).collect::<Vec<_>>(), [("two", "Use 2")]);
        harness.get_by_label("Delete comment");

        // other changes leave it, a change of its line takes it away
        dir.write("a.txt", TEN.replace("2\n", "two\n").replace("9\n", "nine\n"));
        refresh(&mut harness.state_mut().0);
        assert_eq!(harness.state().0.comments().len(), 1);
        dir.write("a.txt", TEN.replace("2\n", "TWO\n"));
        refresh(&mut harness.state_mut().0);
        assert!(harness.state().0.comments().is_empty());
    }

    #[test]
    fn asks_an_agent_about_a_line() {
        let (mut harness, _dir) = comment_harness("ask");
        comment_on_row(&mut harness, 0.0, "Why?");
        harness.get_by_label("Ask agent").click();
        harness.run();
        let text = "@a.txt, removed line 2 of the committed version\n```diff\n-2\n```\nWhy?".to_string();
        assert_eq!(harness.state().1, Some(Outcome::Send { terminal: 1, text }));
        assert!(harness.state().0.comments().is_empty());
        assert!(harness.query_by_label("Ask agent").is_none());
    }

    #[test]
    fn selects_rows_by_dragging_and_sends_the_comments_on_them_as_one() {
        let (mut harness, _dir) = comment_harness("drag");
        // from the line above the hunk to the added line
        harness.event(egui::Event::PointerMoved(row_center(&harness, -1.0)));
        harness.run();
        let start = harness.get_by_label("Comment on this line").rect().center();
        let end = egui::pos2(start.x, row_center(&harness, 1.0).y);
        let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        harness.event(button(start, true));
        harness.run();
        harness.event(egui::Event::PointerMoved(end));
        harness.run();
        harness.event(button(end, false));
        harness.run();
        harness.event(egui::Event::Text("Simpler".to_string()));
        harness.key_press_modifiers(egui::Modifiers::COMMAND, Key::Enter);
        harness.run();
        let comment = harness.state().0.comments()[0].clone();
        assert_eq!(comment.lines.iter().map(|line| line.text.as_str()).collect::<Vec<_>>(), ["1", "2", "two"]);

        harness.get_by_label("Send 1 comment").click();
        harness.run();
        let text = format!("Please address these review comments on the uncommitted changes:\n\n{}", comments::quote("a.txt", &comment.lines, "Simpler"));
        assert_eq!(harness.state().1, Some(Outcome::Send { terminal: 1, text }));
        assert!(harness.state().0.comments()[0].sent);
        assert!(harness.query_by_label("Send 1 comment").is_none());
    }

    #[test]
    fn a_change_made_after_the_review_is_not_reviewed() {
        let (mut review, dir) = review("changed-after", TWO_FILES);
        review.set_reviewed("b.txt", true);
        dir.write("b.txt", "z\n");
        refresh(&mut review);
        // the removal is still reviewed, the new line is not
        assert_eq!(review.progress_of("b.txt"), (1, 2));
    }

    #[test]
    fn marks_files_without_lines_reviewed_as_a_whole() {
        let (mut review, _dir) = review("binary", &[("image.bin", "\0a", "\0b")]);
        assert_eq!(review.progress(), (0, 1));
        review.set_reviewed("image.bin", true);
        refresh(&mut review);
        assert_eq!(review.progress(), (1, 1));
    }

    /// Two changed lines in each file, two hunks in src/a.txt.
    const TREE: &[(&str, &str, &str)] = &[("src/a.txt", TEN, "one\n2\n3\n4\n5\n6\n7\n8\nnine\n10\n"), ("src/ui/b.txt", "x\n", "y\n"), ("srcs.txt", "x\n", "y\n")];

    #[test]
    fn marks_files_and_directories_reviewed_and_counts_their_progress() {
        let (mut review, _dir) = review("directories", TREE);
        review.toggle_reviewed(review.by_path["src/a.txt"], 0);
        assert_eq!([review.progress_of("src"), review.progress_of("src/ui"), review.progress_of("")], [(2, 6), (0, 2), (2, 8)]);
        review.set_reviewed("src", true);
        assert_eq!([review.progress_of("src"), review.progress_of("srcs.txt"), review.progress_of("")], [(6, 6), (0, 2), (6, 8)]);
        review.set_reviewed("src/ui/b.txt", false);
        refresh(&mut review);
        assert_eq!([review.progress_of("src"), review.progress_of("src/ui")], [(4, 6), (0, 2)]);
    }

    #[test]
    fn tells_apart_the_changes_made_since_they_were_looked_at() {
        let (mut review, dir) = review("new", TREE);
        // what there is when the editor starts counts as looked at
        assert!(review.unseen.is_empty());
        dir.write("src/a.txt", "one\n2\nthree\n4\n5\n6\n7\n8\nnine\n10\n");
        refresh(&mut review);
        let a = review.by_path["src/a.txt"];
        assert_eq!(review.files[a].new, [false, true, false]);
        assert_eq!(review.unseen, HashSet::from(["src".to_string(), "src/a.txt".to_string()]));
        // moving to it is looking at it
        assert_eq!(review.next(a), None);
        assert_eq!(review.files[a].new, [false; 3]);
        assert!(review.unseen.is_empty());
    }

    #[test]
    fn moves_on_to_the_next_file_to_review_in_the_order_of_the_tree() {
        let (mut review, _dir) = review("next-file", TREE);
        let a = review.by_path["src/a.txt"];
        // the next hunk of the file first
        assert_eq!((review.next(a), review.view(a).current), (None, 1));
        review.set_reviewed("src/a.txt", true);
        assert_eq!(review.next(a), Some("srcs.txt".to_string()));
        // starting over at the top, on the first hunk that isn't reviewed
        review.set_reviewed("srcs.txt", true);
        assert_eq!(review.next(a), Some("src/ui/b.txt".to_string()));
        assert_eq!(review.view(review.by_path["src/ui/b.txt"]).current, 0);
        review.set_reviewed("src/ui", true);
        assert_eq!(review.next(a), None);
    }

    #[test]
    fn marks_a_directory_reviewed_from_its_menu_in_the_tree() {
        let (review, _dir) = review("tree-menu", TREE);
        let mut harness = Harness::new_ui_state(|ui, review: &mut Review| { review.tree_ui(ui, &mut BTreeSet::new(), None); }, review);
        harness.run();
        harness.get_by_label("ui").click_secondary();
        harness.run();
        assert!(harness.query_by_label("Mark as not reviewed").is_none());
        harness.get_by_label("Mark as reviewed").click();
        harness.run();
        assert_eq!(harness.state().progress_of("src"), (2, 6));
    }

    #[test]
    fn colors_both_sides_and_marks_what_changed_within_lines() {
        let dir = TempDir::new("text");
        dir.write("a.rs", "fn main() {\n    let x = 2;\n}\n");
        let diff = "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -2 +2 @@\n-    let y = 1;\n+    let x = 2;\n";
        let text = FileText::new(dir.path(), &git::parse_diff(diff)[0]);
        assert_eq!(text.old.lines, ["fn main() {", "    let y = 1;", "}"]);
        assert_eq!(text.new.lines, ["fn main() {", "    let x = 2;", "}"]);
        assert_eq!(text.changed, [[vec![8..9, 12..13], vec![8..9, 12..13]]]);
        assert!(text.old.spans[1].contains(&(4..7, "keyword")));
        assert!(text.new.spans[0].contains(&(0..2, "keyword")));
    }

    #[test]
    fn cuts_very_long_lines() {
        let dir = TempDir::new("long-lines");
        let long = format!("\t{}", "é".repeat(1000));
        dir.write("a.txt", format!("{long}\n"));
        let diff = format!("diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n@@ -0,0 +1 @@\n+{long}\n");
        let text = FileText::new(dir.path(), &git::parse_diff(&diff)[0]);
        assert_eq!(text.new.lines, [format!("    {}…", "é".repeat(MAX_LINE_CHARS - 4))]);
        assert_eq!(text.longest, MAX_LINE_CHARS + 1);
    }

    #[test]
    fn draws_only_the_hunks_in_sight() {
        let lines: Vec<String> = (1..=1000).map(|n| format!("line {n}")).collect();
        let changed: Vec<String> = (1..=1000).map(|n| if n % 10 == 1 { format!("new {n}") } else { format!("line {n}") }).collect();
        let (review, _dir) = review("many-hunks", &[("a.txt", &(lines.join("\n") + "\n"), &(changed.join("\n") + "\n"))]);
        let mut harness = Harness::builder().with_size(egui::vec2(800.0, 600.0)).build_ui_state(|ui, review: &mut Review| { review.ui(ui, Some("a.txt"), true, &[]); }, review);
        harness.run();
        let drawn = harness.query_all_by_label("Reviewed").count();
        assert!(drawn > 0 && drawn < 20, "{drawn} hunks drawn");
        // moving far down shows the hunks there, of which one is reviewed
        let is_reviewed = |harness: &Harness<'_, Review>| harness.query_all_by_label("Reviewed").any(|node| node.accesskit_node().toggled() == Some(egui::accesskit::Toggled::True));
        harness.state_mut().stage(0, |hunk, _| hunk == 90);
        harness.run();
        assert!(!is_reviewed(&harness));
        harness.state_mut().select(0, 90);
        harness.run();
        harness.run();
        assert!(is_reviewed(&harness));
    }

    #[test]
    fn finds_the_unchanged_lines_around_the_hunks() {
        let hunks = &git::parse_diff(DIFF)[0].hunks;
        assert_eq!(gaps(hunks, 12), [Gap { lines: 1..1, old_offset: 0 }, Gap { lines: 2..9, old_offset: 0 }, Gap { lines: 10..13, old_offset: 0 }]);

        // two lines added at the top move the old line numbers below them
        let added = git::parse_diff("diff --git a/a b/a\n--- a/a\n+++ b/a\n@@ -2,1 +2,3 @@\n x\n+y\n+z\n");
        assert_eq!(gaps(&added[0].hunks, 6), [Gap { lines: 1..2, old_offset: 0 }, Gap { lines: 5..7, old_offset: -2 }]);
    }

    #[test]
    fn folds_away_the_unchanged_lines_away_from_the_hunks_unless_expanded() {
        let hunks = &git::parse_diff(DIFF)[0].hunks;
        let gaps = [Gap { lines: 8..9, old_offset: 0 }, Gap { lines: 2..12, old_offset: 0 }, Gap { lines: 13..30, old_offset: 0 }];
        let unchanged = |gap: usize, lines: Range<usize>| lines.map(move |number| Row::Unchanged(gap, number));
        let expected: Vec<Row> = [
            // just one line before the first hunk is shown as it is
            vec![Row::Unchanged(0, 8), Row::Hunk(0, 0), Row::Hunk(0, 1)],
            unchanged(1, 2..5).collect(),
            vec![Row::Folded(1, 4)],
            unchanged(1, 9..12).collect(),
            vec![Row::Hunk(1, 0), Row::Hunk(1, 1)],
            unchanged(2, 13..16).collect(),
            vec![Row::Folded(2, 14)],
        ]
        .into_iter()
        .flatten()
        .collect();
        assert_eq!(rows(&gaps, hunks, &[false; 3], &[]), (expected, vec![1, 10]));
        assert_eq!(rows(&gaps, hunks, &[false, true, false], &[]).0[3..13], unchanged(1, 2..12).collect::<Vec<_>>());
    }

    #[test]
    fn shows_folded_lines_when_clicked() {
        let lines: Vec<String> = (1..=100).map(|n| format!("line {n}")).collect();
        let text = lines.join("\n") + "\n";
        let (review, _dir) = review("more-lines", &[("a.txt", &text, &text.replace("line 50\n", "new 50\n"))]);
        let mut harness = Harness::builder().with_size(egui::vec2(800.0, 3000.0)).build_ui_state(|ui, review: &mut Review| { review.ui(ui, Some("a.txt"), true, &[]); }, review);
        harness.run();
        // 46 lines above the hunk and 47 below it
        harness.get_by_label("Show 47 hidden lines");
        harness.get_by_label("Show 46 hidden lines").click();
        harness.run();
        assert_eq!(harness.state_mut().view(0).expanded, [true, false]);
        assert!(harness.query_by_label("Show 46 hidden lines").is_none());
    }

    #[test]
    fn a_review_without_a_file_or_changes_says_so() {
        for (path, label) in [(None, "Choose a file under Changes to review it"), (Some("gone.txt"), "No changes in gone.txt")] {
            let (review, _dir) = review("no-file", &[("a.txt", "x\n", "x\n")]);
            let mut harness = Harness::new_ui_state(move |ui, review: &mut Review| { review.ui(ui, path, true, &[]); }, review);
            harness.run();
            harness.get_by_label(label);
        }
    }
}
