use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::ops::Range;
use std::path::{Path, PathBuf};

use eframe::egui::{self, Align, Color32, Key, RichText, Sense, Stroke, Vec2};

use crate::code::{CodeStyle, expand_tabs};
use crate::file_tree::{FileLook, FileTree};
use crate::file_view;
use crate::git::{self, DiffLine, FileDiff, FileStatus, Hunk, LineKind};
use crate::highlight::{self, Highlighter, Spans};
use crate::word_diff::{self, Ranges};

/// Longer lines are cut to this many characters, as drawing very long lines takes long.
const MAX_LINE_CHARS: usize = 500;

/// Fingerprints of reviewed hunks, persisted as one fingerprint per line.
pub struct ReviewState {
    path: PathBuf,
    reviewed: HashSet<String>,
}

impl ReviewState {
    pub fn load(path: PathBuf) -> Self {
        let reviewed = fs::read_to_string(&path)
            .map(|s| s.lines().map(str::to_string).collect())
            .unwrap_or_default();
        Self { path, reviewed }
    }

    pub fn is_reviewed(&self, fingerprint: &str) -> bool {
        self.reviewed.contains(fingerprint)
    }

    pub fn toggle(&mut self, fingerprint: &str) {
        let reviewed = !self.is_reviewed(fingerprint);
        self.set([fingerprint], reviewed);
    }

    pub fn set<'a>(&mut self, fingerprints: impl IntoIterator<Item = &'a str>, reviewed: bool) {
        for fingerprint in fingerprints {
            if reviewed {
                self.reviewed.insert(fingerprint.to_string());
            } else {
                self.reviewed.remove(fingerprint);
            }
        }
        self.save();
    }

    /// Forgets hunks that are no longer part of the diff, so the file doesn't grow forever.
    pub fn retain_only<'a>(&mut self, current: impl IntoIterator<Item = &'a str>) {
        let current: HashSet<&str> = current.into_iter().collect();
        let before = self.reviewed.len();
        self.reviewed.retain(|fp| current.contains(fp.as_str()));
        if self.reviewed.len() != before {
            self.save();
        }
    }

    fn save(&self) {
        let mut lines: Vec<&str> = self.reviewed.iter().map(String::as_str).collect();
        lines.sort();
        if let Err(e) = fs::write(&self.path, lines.join("\n")) {
            eprintln!("failed to save review state to {}: {e}", self.path.display());
        }
    }
}

/// The working-tree changes, reviewed one file at a time.
pub struct Review {
    root: PathBuf,
    state: ReviewState,
    files: Vec<FileReview>,
    by_path: HashMap<String, usize>,
    /// The changed files, in the same form as the project file tree.
    tree: FileTree,
    /// How each file is shown, kept while other files are reviewed.
    views: HashMap<String, View>,
    /// The lines of the files shown, ready to be drawn.
    texts: HashMap<String, FileText>,
    /// The number of reviewed hunks and of all hunks of every file and directory, and of all
    /// files together under "".
    progress: HashMap<String, (usize, usize)>,
}

struct FileReview {
    diff: FileDiff,
    /// The fingerprint of each hunk.
    fingerprints: Vec<String>,
}

#[derive(Default)]
struct View {
    current: usize,
    scroll_to_current: bool,
    /// For each run of unchanged lines (one before each hunk and one after the last): whether
    /// it is shown rather than folded away.
    expanded: Vec<bool>,
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
    pub fn new(state: ReviewState, root: PathBuf) -> Self {
        Self {
            root,
            state,
            files: Vec::new(),
            by_path: HashMap::new(),
            tree: FileTree::open_by_default(),
            views: HashMap::new(),
            texts: HashMap::new(),
            progress: HashMap::new(),
        }
    }

    pub fn set_diff(&mut self, raw: &str, collapsed: &BTreeSet<String>) {
        self.files = git::parse_diff(raw)
            .into_iter()
            .map(|diff| FileReview { fingerprints: diff.hunks.iter().map(|hunk| hunk.fingerprint(&diff.path)).collect(), diff })
            .collect();
        self.by_path = self.files.iter().enumerate().map(|(index, file)| (file.diff.path.clone(), index)).collect();
        self.state.retain_only(self.files.iter().flat_map(|file| file.fingerprints.iter().map(String::as_str)));
        self.tree.set_files(self.files.iter().map(|file| file.diff.path.clone()).collect(), collapsed);
        self.texts.clear();
        self.update_progress();
        let (files, by_path) = (&self.files, &self.by_path);
        self.views.retain(|path, view| {
            let Some(&index) = by_path.get(path) else { return false };
            let hunks = files[index].diff.hunks.len();
            if view.expanded.len() != hunks + 1 {
                view.expanded.clear();
            }
            view.current = view.current.min(hunks.saturating_sub(1));
            true
        });
    }

    fn update_progress(&mut self) {
        self.progress.clear();
        for file in &self.files {
            let (done, total) = file_progress(&self.state, file);
            let path = file.diff.path.as_str();
            let dirs = path.match_indices('/').map(|(end, _)| &path[..end]);
            for key in ["", path].into_iter().chain(dirs) {
                let progress = self.progress.entry(key.to_string()).or_default();
                *progress = (progress.0 + done, progress.1 + total);
            }
        }
    }

    /// The number of reviewed hunks and of all hunks.
    pub fn progress(&self) -> (usize, usize) {
        self.progress_of("")
    }

    /// The number of reviewed hunks and of all hunks of a file, or of the files in a directory.
    fn progress_of(&self, path: &str) -> (usize, usize) {
        self.progress.get(path).copied().unwrap_or_default()
    }

    fn next_unreviewed(&self, file: usize, from: usize) -> Option<usize> {
        let fingerprints = &self.files[file].fingerprints;
        let n = fingerprints.len();
        (1..=n).map(|i| (from + i) % n).find(|&i| !self.state.is_reviewed(&fingerprints[i]))
    }

    fn view(&mut self, file: usize) -> &mut View {
        self.views.entry(self.files[file].diff.path.clone()).or_default()
    }

    fn select(&mut self, file: usize, hunk: usize) {
        let view = self.view(file);
        view.current = hunk;
        view.scroll_to_current = true;
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
        let first = self.next_unreviewed(index, self.files[index].fingerprints.len() - 1).unwrap_or(0);
        self.select(index, first);
        Some(next)
    }

    /// Marks a hunk as reviewed or not; moving on to the next one when it is reviewed, which
    /// may be in another file that is returned then.
    fn toggle_reviewed(&mut self, file: usize, hunk: usize) -> Option<String> {
        let fingerprint = &self.files[file].fingerprints[hunk];
        self.state.toggle(fingerprint);
        let reviewed = self.state.is_reviewed(fingerprint);
        self.update_progress();
        self.view(file).current = hunk;
        if reviewed { self.next(file) } else { None }
    }

    /// Marks all hunks of a file, or of the files in a directory, as reviewed or not.
    fn set_reviewed(&mut self, path: &str, reviewed: bool) {
        let inside = |file: &str| file.strip_prefix(path).is_some_and(|rest| rest.is_empty() || rest.starts_with('/'));
        let fingerprints = self.files.iter().filter(|file| inside(&file.diff.path)).flat_map(|file| file.fingerprints.iter().map(String::as_str));
        self.state.set(fingerprints, reviewed);
        self.update_progress();
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
        let Self { tree, files, by_path, progress, .. } = self;
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
            FileLook { color, detail: (total > 0).then(|| format!("{done}/{total}")) }
        };
        let clicked = tree.ui(ui, collapsed, selected, look, Some(&mut menu));
        if let Some((path, reviewed)) = mark {
            self.set_reviewed(&path, reviewed);
        }
        clicked
    }

    /// Shows the changes of one file, taking the review keys when `active`. Returns the file to
    /// show next, when the review moves on to another one.
    pub fn ui(&mut self, ui: &mut egui::Ui, path: Option<&str>, active: bool) -> Option<String> {
        let Some(path) = path else {
            ui.centered_and_justified(|ui| ui.weak("Choose a file under Changes to review it"));
            return None;
        };
        let Some(&index) = self.by_path.get(path) else {
            ui.centered_and_justified(|ui| ui.weak(format!("No changes in {path}")));
            return None;
        };
        let mut next = if active { self.handle_keys(ui.ctx(), index) } else { None };
        let (done, total) = self.progress_of(path);
        let Self { root, state, files, views, texts, .. } = self;
        let file = &files[index];
        let text = texts.entry(path.to_string()).or_insert_with(|| FileText::new(root, &file.diff));
        let view = views.entry(path.to_string()).or_default();
        let gaps = gaps(&file.diff.hunks, text.new.lines.len());
        view.expanded.resize(gaps.len(), false);

        let mut mark_file = None;
        ui.horizontal(|ui| {
            ui.label(RichText::new(file.diff.display_name()).monospace().strong());
            ui.weak(format!("{done}/{total} reviewed"));
            if done < total && ui.small_button("Mark file reviewed").clicked() {
                mark_file = Some(true);
            }
            if done > 0 && done == total && ui.small_button("Mark file not reviewed").clicked() {
                mark_file = Some(false);
            }
        });
        ui.add_space(4.0);
        let toggled = egui::ScrollArea::both()
            .id_salt(path)
            .auto_shrink(false)
            .show(ui, |ui| {
                if file.diff.binary {
                    ui.weak("Binary file");
                }
                rows_ui(ui, file, text, &gaps, state, view)
            })
            .inner;
        view.scroll_to_current = false;
        if let Some(hunk) = toggled {
            next = self.toggle_reviewed(index, hunk);
        }
        if let Some(reviewed) = mark_file {
            self.set_reviewed(path, reviewed);
            if reviewed {
                next = self.next(index);
            }
        }
        next
    }
}

fn file_progress(state: &ReviewState, file: &FileReview) -> (usize, usize) {
    (file.fingerprints.iter().filter(|fp| state.is_reviewed(fp)).count(), file.fingerprints.len())
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

impl FileText {
    /// The text of a changed file as it is now and as it was. Where the diff doesn't hold the
    /// unchanged lines, they are taken from the file on disk.
    fn new(root: &Path, diff: &FileDiff) -> Self {
        let path = root.join(&diff.path);
        let content: Vec<String> = match diff.status {
            FileStatus::Deleted => Vec::new(),
            _ => file_view::read(&path).map(|text| text.lines().map(str::to_string).collect()).unwrap_or_default(),
        };
        let (mut new, mut old) = (Vec::new(), Vec::new());
        for (index, gap) in gaps(&diff.hunks, content.len()).iter().enumerate() {
            for number in gap.lines.clone() {
                let line = content.get(number - 1).map_or("", String::as_str);
                new.push(line);
                old.push(line);
            }
            for line in diff.hunks.get(index).map_or(&[][..], |hunk| &hunk.lines) {
                if line.new_no.is_some() {
                    new.push(&line.text);
                }
                if line.old_no.is_some() {
                    old.push(&line.text);
                }
            }
        }
        let (new, old) = (Side::new(&path, &new), Side::new(&path, &old));
        let changed = diff.hunks.iter().map(|hunk| changed_in_lines(hunk, &new, &old)).collect();
        let longest = new.lines.iter().chain(&old.lines).map(|line| line.chars().count()).max().unwrap_or(0);
        Self { new, old, changed, longest }
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
    /// The lines of a gap that are folded away.
    Folded(usize),
    /// A line of a hunk: the index of the hunk and of the line in it.
    Hunk(usize, usize),
}

/// The rows of a file, and the first row of each hunk. A gap that is not expanded is folded
/// into a row of its own, unless it is just one line.
fn rows(gaps: &[Gap], hunks: &[Hunk], expanded: &[bool]) -> (Vec<Row>, Vec<usize>) {
    let (mut rows, mut starts) = (Vec::new(), Vec::new());
    for (index, gap) in gaps.iter().enumerate() {
        if expanded[index] || gap.lines.len() <= 1 {
            rows.extend(gap.lines.clone().map(|number| Row::Unchanged(index, number)));
        } else {
            rows.push(Row::Folded(index));
        }
        if let Some(hunk) = hunks.get(index) {
            starts.push(rows.len());
            rows.extend((0..hunk.lines.len()).map(|line| Row::Hunk(index, line)));
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

/// The width of the reviewed marks, the line numbers and the sign in front of the code, in digits.
const GUTTER_DIGITS: usize = 17;

/// Shows the rows of a file as one piece of code, drawing only the ones in sight. Returns the
/// hunk whose reviewed mark was clicked.
fn rows_ui(ui: &mut egui::Ui, file: &FileReview, text: &FileText, gaps: &[Gap], state: &ReviewState, view: &mut View) -> Option<usize> {
    let hunks = &file.diff.hunks;
    let (rows, starts) = rows(gaps, hunks, &view.expanded);
    let style = CodeStyle::new(ui);
    let row_height = style.row_height;
    let width = style.digit_width * (GUTTER_DIGITS + text.longest + 2) as f32;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width().max(width), row_height * rows.len() as f32), Sense::click());
    let row_at = |index: usize| egui::Rect::from_min_size(egui::pos2(rect.left(), rect.top() + row_height * index as f32), Vec2::new(rect.width(), row_height));
    if view.scroll_to_current && let Some(&start) = starts.get(view.current) {
        // with a few lines above the hunk
        ui.scroll_to_rect(row_at(start.saturating_sub(3)), Some(Align::TOP));
    }
    // a click on a hunk makes it the current one
    if response.clicked()
        && let Some(pointer) = response.interact_pointer_pos()
        && let Some(Row::Hunk(hunk, _)) = rows.get(((pointer.y - rect.top()) / row_height) as usize)
    {
        view.current = *hunk;
    }
    let clip = ui.clip_rect();
    let first = ((clip.top() - rect.top()) / row_height).floor().max(0.0) as usize;
    let last = (((clip.bottom() - rect.top()) / row_height).ceil().max(0.0) as usize).min(rows.len());
    // the gutter stays at the left edge when the code scrolls sideways, and covers it
    let gutter_left = rect.left().max(clip.left());
    let mut toggled = None;
    for (index, row) in rows.iter().enumerate().take(last).skip(first) {
        let row_rect = row_at(index);
        match *row {
            Row::Folded(gap) => {
                let visible = egui::Rect::from_x_y_ranges(gutter_left..=clip.right(), row_rect.y_range());
                if fold_ui(ui, visible, gaps[gap].lines.len(), response.id.with(("fold", gap))) {
                    view.expanded[gap] = true;
                }
            }
            Row::Unchanged(gap, number) => {
                let (line, spans) = text.new.line(Some(number as u32));
                let old_no = u32::try_from(number as isize + gaps[gap].old_offset).ok();
                let line = Line { kind: LineKind::Context, old_no, new_no: Some(number as u32), text: line, spans, changed: &[] };
                line_ui(ui, &style, row_rect, gutter_left, &line, false);
            }
            Row::Hunk(hunk, at) => {
                let reviewed = state.is_reviewed(&file.fingerprints[hunk]);
                line_ui(ui, &style, row_rect, gutter_left, &text.row(&hunks[hunk].lines[at], &text.changed[hunk][at]), reviewed);
                if hunk == view.current {
                    let bar = egui::Rect::from_min_size(egui::pos2(gutter_left, row_rect.top()), Vec2::new(3.0, row_height));
                    ui.painter().rect_filled(bar, 0.0, ui.visuals().selection.stroke.color);
                }
                let mark = egui::Rect::from_min_size(egui::pos2(gutter_left + style.digit_width * 0.5, row_rect.top()), Vec2::splat(row_height));
                if at == 0 && reviewed_mark_ui(ui, mark, reviewed, response.id.with(("reviewed", hunk))) {
                    toggled = Some(hunk);
                }
            }
        }
    }
    toggled
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
    style.number(painter, gutter_left + digit * 8.0, y, line.old_no);
    style.number(painter, gutter_left + digit * 13.0, y, line.new_no);
    let sign = match line.kind {
        LineKind::Context => " ",
        LineKind::Added => "+",
        LineKind::Removed => "-",
    };
    style.text(painter, gutter_left + digit * 14.5, y, sign, style.weak_color);
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

/// The reviewed mark of a hunk, in the gutter of its first line. Returns true when clicked.
fn reviewed_mark_ui(ui: &egui::Ui, rect: egui::Rect, reviewed: bool, id: egui::Id) -> bool {
    let response = ui.interact(rect, id, Sense::click());
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, ui.is_enabled(), reviewed, "Reviewed"));
    let visuals = ui.style().interact(&response);
    let mark = egui::Rect::from_center_size(rect.center(), Vec2::splat(rect.height() - 5.0));
    ui.painter().rect(mark, 2.0, visuals.bg_fill, visuals.bg_stroke, egui::StrokeKind::Inside);
    if reviewed {
        let (left, right) = (mark.left() + mark.width() * 0.2, mark.right() - mark.width() * 0.2);
        let check = vec![egui::pos2(left, mark.center().y), egui::pos2(mark.center().x - mark.width() * 0.05, mark.bottom() - mark.height() * 0.25), egui::pos2(right, mark.top() + mark.height() * 0.25)];
        ui.painter().add(egui::Shape::line(check, visuals.fg_stroke));
    }
    response.clicked()
}

#[cfg(test)]
mod tests {
    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT as _, Queryable as _};

    use super::*;
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

    fn review(dir: &TempDir) -> Review {
        Review::new(ReviewState::load(dir.path().join("reviewed")), dir.path().to_path_buf())
    }

    #[test]
    fn marking_a_hunk_reviewed_moves_on_to_the_next_unreviewed_one() {
        let mut review = review(&TempDir::new("moves-on"));
        review.set_diff(DIFF, &BTreeSet::new());
        assert_eq!(review.progress(), (0, 3));
        assert_eq!(review.toggle_reviewed(0, 0), None);
        assert_eq!((review.progress(), review.view(0).current), ((1, 3), 1));
        // the last hunk of a file moves on to the next file
        assert_eq!(review.toggle_reviewed(0, 1), Some("b.txt".to_string()));
        assert_eq!((review.progress_of("a.txt"), review.view(0).current), ((2, 2), 1));
    }

    const TREE_DIFF: &str = "\
diff --git a/src/a.txt b/src/a.txt
--- a/src/a.txt
+++ b/src/a.txt
@@ -1 +1 @@
-one
+two
@@ -9 +9 @@
-three
+four
diff --git a/src/ui/b.txt b/src/ui/b.txt
--- a/src/ui/b.txt
+++ b/src/ui/b.txt
@@ -1 +1 @@
-five
+six
diff --git a/srcs.txt b/srcs.txt
--- a/srcs.txt
+++ b/srcs.txt
@@ -1 +1 @@
-seven
+eight
";

    #[test]
    fn marks_files_and_directories_reviewed_and_counts_their_progress() {
        let mut review = review(&TempDir::new("directories"));
        review.set_diff(TREE_DIFF, &BTreeSet::new());
        review.toggle_reviewed(review.by_path["src/a.txt"], 0);
        assert_eq!([review.progress_of("src"), review.progress_of("src/ui"), review.progress_of("")], [(1, 3), (0, 1), (1, 4)]);
        review.set_reviewed("src", true);
        assert_eq!([review.progress_of("src"), review.progress_of("srcs.txt"), review.progress_of("")], [(3, 3), (0, 1), (3, 4)]);
        review.set_reviewed("src/ui/b.txt", false);
        assert_eq!([review.progress_of("src"), review.progress_of("src/ui")], [(2, 3), (0, 1)]);
    }

    #[test]
    fn moves_on_to_the_next_file_to_review_in_the_order_of_the_tree() {
        let mut review = review(&TempDir::new("next-file"));
        review.set_diff(TREE_DIFF, &BTreeSet::new());
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
        let mut review = review(&TempDir::new("tree-menu"));
        review.set_diff(TREE_DIFF, &BTreeSet::new());
        let mut harness = Harness::new_ui_state(|ui, review: &mut Review| { review.tree_ui(ui, &mut BTreeSet::new(), None); }, review);
        harness.run();
        harness.get_by_label("ui").click_secondary();
        harness.run();
        assert!(harness.query_by_label("Mark as not reviewed").is_none());
        harness.get_by_label("Mark as reviewed").click();
        harness.run();
        assert_eq!(harness.state().progress_of("src"), (1, 3));
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
        let dir = TempDir::new("many-hunks");
        let lines: Vec<String> = (1..=1000).map(|n| format!("line {n}")).collect();
        dir.write("a.txt", lines.join("\n") + "\n");
        let hunks: String = (0..100).map(|k| format!("@@ -{0} +{0} @@\n-old\n+line {0}\n", k * 10 + 1)).collect();
        let mut review = review(&dir);
        review.set_diff(&format!("diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n{hunks}"), &BTreeSet::new());
        let mut harness = Harness::builder().with_size(egui::vec2(800.0, 600.0)).build_ui_state(|ui, review: &mut Review| { review.ui(ui, Some("a.txt"), true); }, review);
        harness.run();
        let drawn = harness.query_all_by_label("Reviewed").count();
        assert!(drawn > 0 && drawn < 20, "{drawn} hunks drawn");
        // moving far down shows the hunks there, of which one is reviewed
        let is_reviewed = |harness: &Harness<'_, Review>| harness.query_all_by_label("Reviewed").any(|node| node.accesskit_node().toggled() == Some(egui::accesskit::Toggled::True));
        let fingerprint = harness.state().files[0].fingerprints[90].clone();
        harness.state_mut().state.toggle(&fingerprint);
        harness.run();
        assert!(!is_reviewed(&harness));
        harness.state_mut().select(0, 90);
        harness.run();
        harness.run();
        assert!(is_reviewed(&harness));
    }

    #[test]
    fn keeps_reviewed_marks_until_a_hunk_changes() {
        let dir = TempDir::new("keeps-marks");
        let mut review = review(&dir);
        review.set_diff(DIFF, &BTreeSet::new());
        review.toggle_reviewed(1, 0);

        // the first file is no longer changed, the second one is untouched
        review.set_diff(&DIFF[DIFF.find("diff --git a/b.txt").unwrap()..], &BTreeSet::new());
        assert_eq!(review.progress(), (1, 1));

        // a changed hunk needs to be reviewed again
        review.set_diff(&DIFF.replace("six", "seven"), &BTreeSet::new());
        assert_eq!(review.progress(), (0, 3));
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
    fn folds_away_the_unchanged_lines_between_the_hunks_unless_expanded() {
        let hunks = &git::parse_diff(DIFF)[0].hunks;
        let gaps = gaps(hunks, 10);
        // one line before the second hunk is shown as it is
        let gaps = [Gap { lines: 8..9, old_offset: 0 }, gaps[1].clone(), gaps[2].clone()];
        assert_eq!(rows(&gaps, hunks, &[false; 3]), (vec![Row::Unchanged(0, 8), Row::Hunk(0, 0), Row::Hunk(0, 1), Row::Folded(1), Row::Hunk(1, 0), Row::Hunk(1, 1), Row::Unchanged(2, 10)], vec![1, 4]));
        assert_eq!(rows(&gaps, hunks, &[false, true, false]).0[3..10], [Row::Unchanged(1, 2), Row::Unchanged(1, 3), Row::Unchanged(1, 4), Row::Unchanged(1, 5), Row::Unchanged(1, 6), Row::Unchanged(1, 7), Row::Unchanged(1, 8)]);
    }

    #[test]
    fn shows_folded_lines_when_clicked() {
        let dir = TempDir::new("more-lines");
        let lines: Vec<String> = (1..=100).map(|n| format!("line {n}")).collect();
        dir.write("a.txt", lines.join("\n") + "\n");
        let diff = "diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n@@ -47,7 +47,7 @@\n line 47\n line 48\n line 49\n-old 50\n+line 50\n line 51\n line 52\n line 53\n";
        let mut review = review(&dir);
        review.set_diff(diff, &BTreeSet::new());
        let mut harness = Harness::builder().with_size(egui::vec2(800.0, 3000.0)).build_ui_state(|ui, review: &mut Review| { review.ui(ui, Some("a.txt"), true); }, review);
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
        let dir = TempDir::new("no-file");
        for (path, label) in [(None, "Choose a file under Changes to review it"), (Some("gone.txt"), "No changes in gone.txt")] {
            let mut harness = Harness::new_ui_state(move |ui, review: &mut Review| { review.ui(ui, path, true); }, review(&dir));
            harness.run();
            harness.get_by_label(label);
        }
    }
}
