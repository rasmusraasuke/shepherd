use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::ops::Range;
use std::path::PathBuf;

use eframe::egui::{self, Align, Color32, Key, RichText, Sense, Stroke, Vec2};

use crate::code::{CodeStyle, expand_tabs};
use crate::file_tree::{FileLook, FileTree};
use crate::file_view;
use crate::git::{self, DiffLine, FileDiff, FileStatus, Hunk, LineKind};

/// How many more unchanged lines are shown when the user asks for more of a file.
const MORE_LINES: usize = 20;

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
        if !self.reviewed.remove(fingerprint) {
            self.reviewed.insert(fingerprint.to_string());
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
    /// The lines of the files shown, for the unchanged code around the changes.
    contents: HashMap<String, Vec<String>>,
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
    /// For each run of unchanged lines (one before each hunk and one after the last): how many
    /// of its lines are shown at its start and at its end.
    shown: Vec<(usize, usize)>,
}

/// A run of unchanged lines between the changes, by their numbers in the new file.
#[derive(Debug, PartialEq)]
struct Gap {
    lines: Range<usize>,
    /// What to add to the number of a line for its number in the old file.
    old_offset: isize,
}

impl Review {
    pub fn new(state: ReviewState, root: PathBuf) -> Self {
        Self { root, state, files: Vec::new(), by_path: HashMap::new(), tree: FileTree::open_by_default(), views: HashMap::new(), contents: HashMap::new() }
    }

    pub fn set_diff(&mut self, raw: &str, collapsed: &BTreeSet<String>) {
        self.files = git::parse_diff(raw)
            .into_iter()
            .map(|diff| FileReview { fingerprints: diff.hunks.iter().map(|hunk| hunk.fingerprint(&diff.path)).collect(), diff })
            .collect();
        self.by_path = self.files.iter().enumerate().map(|(index, file)| (file.diff.path.clone(), index)).collect();
        self.state.retain_only(self.files.iter().flat_map(|file| file.fingerprints.iter().map(String::as_str)));
        self.tree.set_files(self.files.iter().map(|file| file.diff.path.clone()).collect(), collapsed);
        self.contents.clear();
        let (files, by_path) = (&self.files, &self.by_path);
        self.views.retain(|path, view| {
            let Some(&index) = by_path.get(path) else { return false };
            let hunks = files[index].diff.hunks.len();
            if view.shown.len() != hunks + 1 {
                view.shown.clear();
            }
            view.current = view.current.min(hunks.saturating_sub(1));
            true
        });
    }

    /// The number of reviewed hunks and of all hunks.
    pub fn progress(&self) -> (usize, usize) {
        self.files.iter().map(|file| self.file_progress(file)).fold((0, 0), |(a, b), (c, d)| (a + c, b + d))
    }

    fn file_progress(&self, file: &FileReview) -> (usize, usize) {
        file_progress(&self.state, file)
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

    fn toggle_reviewed(&mut self, file: usize, hunk: usize) {
        let fingerprint = &self.files[file].fingerprints[hunk];
        self.state.toggle(fingerprint);
        let reviewed = self.state.is_reviewed(fingerprint);
        self.view(file).current = hunk;
        if reviewed && let Some(next) = self.next_unreviewed(file, hunk) {
            self.select(file, next);
        }
    }

    fn handle_keys(&mut self, ctx: &egui::Context, file: usize) {
        let count = self.files[file].diff.hunks.len();
        if count == 0 || ctx.memory(|m| m.focused().is_some()) {
            return;
        }
        let current = self.view(file).current;
        let pressed = |key| ctx.input(|i| i.modifiers.is_none() && i.key_pressed(key));
        if pressed(Key::J) && current + 1 < count {
            self.select(file, current + 1);
        }
        if pressed(Key::K) && current > 0 {
            self.select(file, current - 1);
        }
        if pressed(Key::N) && let Some(next) = self.next_unreviewed(file, current) {
            self.select(file, next);
        }
        if pressed(Key::Space) {
            self.toggle_reviewed(file, current);
        }
    }

    /// Shows the tree of changed files with their review progress. Returns the file that was clicked.
    pub fn tree_ui(&mut self, ui: &mut egui::Ui, collapsed: &mut BTreeSet<String>, selected: Option<&str>) -> Option<String> {
        if self.files.is_empty() {
            ui.weak("No changes");
            return None;
        }
        let (added, deleted) = status_colors(ui.visuals().dark_mode);
        let weak = ui.visuals().weak_text_color();
        let Self { tree, files, by_path, state, .. } = self;
        tree.ui(ui, collapsed, selected, |path| {
            let Some(file) = by_path.get(path).map(|&index| &files[index]) else { return FileLook::default() };
            let (done, total) = file_progress(state, file);
            let color = match file.diff.status {
                _ if done == total => Some(weak),
                FileStatus::Added => Some(added),
                FileStatus::Deleted => Some(deleted),
                FileStatus::Modified | FileStatus::Renamed => None,
            };
            FileLook { color, detail: (total > 0).then(|| format!("{done}/{total}")) }
        })
    }

    /// Shows the changes of one file, taking the review keys when `active`.
    pub fn ui(&mut self, ui: &mut egui::Ui, path: Option<&str>, active: bool) {
        let Some(path) = path else {
            ui.centered_and_justified(|ui| ui.weak("Choose a file under Changes to review it"));
            return;
        };
        let Some(&index) = self.by_path.get(path) else {
            ui.centered_and_justified(|ui| ui.weak(format!("No changes in {path}")));
            return;
        };
        if active {
            self.handle_keys(ui.ctx(), index);
        }
        let (done, total) = self.file_progress(&self.files[index]);
        let Self { root, state, files, views, contents, .. } = self;
        let file = &files[index];
        let content = contents.entry(path.to_string()).or_insert_with(|| match file.diff.status {
            FileStatus::Deleted => Vec::new(),
            _ => file_view::read(&root.join(path)).map(|text| text.lines().map(str::to_string).collect()).unwrap_or_default(),
        });
        let view = views.entry(path.to_string()).or_default();
        let gaps = gaps(&file.diff.hunks, content.len());
        view.shown.resize(gaps.len(), (0, 0));

        ui.horizontal(|ui| {
            ui.label(RichText::new(file.diff.display_name()).monospace().strong());
            ui.weak(format!("{done}/{total} reviewed"));
        });
        ui.add_space(4.0);
        let mut toggled = None;
        egui::ScrollArea::vertical().id_salt(path).auto_shrink(false).show(ui, |ui| {
            if file.diff.binary {
                ui.weak("Binary file");
            }
            for (gap_index, gap) in gaps.iter().enumerate() {
                let is_last = gap_index + 1 == gaps.len();
                // lined up with the rows inside the frames of the hunks
                let shown = &mut view.shown[gap_index];
                egui::Frame::new().inner_margin(egui::Margin::symmetric(7, 0)).show(ui, |ui| gap_ui(ui, gap, content, shown, gap_index == 0, is_last));
                if is_last {
                    break;
                }
                let hunk = &file.diff.hunks[gap_index];
                let reviewed = state.is_reviewed(&file.fingerprints[gap_index]);
                let is_current = gap_index == view.current;
                let stroke = if is_current { Stroke::new(2.0, ui.visuals().selection.stroke.color) } else { ui.visuals().widgets.noninteractive.bg_stroke };
                let response = egui::Frame::new().stroke(stroke).corner_radius(4.0).inner_margin(6.0).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let mut checked = reviewed;
                        let checkbox = ui.checkbox(&mut checked, "");
                        checkbox.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, ui.is_enabled(), reviewed, "Reviewed"));
                        if checkbox.changed() {
                            toggled = Some(gap_index);
                        }
                        ui.label(RichText::new(&hunk.header).monospace().weak());
                    });
                    // reviewed hunks collapse to their header to keep the focus on what's left
                    if !reviewed {
                        code_rows(ui, hunk.lines.len(), |i| Line::of(&hunk.lines[i]));
                    }
                });
                if is_current && view.scroll_to_current {
                    ui.scroll_to_rect(response.response.rect, Some(Align::TOP));
                }
            }
        });
        view.scroll_to_current = false;
        if let Some(hunk) = toggled {
            self.toggle_reviewed(index, hunk);
        }
    }
}

fn file_progress(state: &ReviewState, file: &FileReview) -> (usize, usize) {
    (file.fingerprints.iter().filter(|fp| state.is_reviewed(fp)).count(), file.fingerprints.len())
}

/// The colors of the names of added and deleted files.
fn status_colors(dark_mode: bool) -> (Color32, Color32) {
    if dark_mode { (Color32::from_rgb(110, 190, 120), Color32::from_rgb(220, 110, 110)) } else { (Color32::from_rgb(30, 130, 50), Color32::from_rgb(190, 40, 40)) }
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

/// Splits the lines of a gap into those shown at its start, the number hidden, and those
/// shown at its end.
fn split(lines: &Range<usize>, (start, end): (usize, usize)) -> (Range<usize>, usize, Range<usize>) {
    if start + end >= lines.len() {
        return (lines.clone(), 0, lines.end..lines.end);
    }
    (lines.start..lines.start + start, lines.len() - start - end, lines.end - end..lines.end)
}

/// Shows the unchanged lines of a gap that were asked for, with buttons for more in between.
/// Lines are added at its start for the hunk above it, at its end for the hunk below it.
fn gap_ui(ui: &mut egui::Ui, gap: &Gap, content: &[String], shown: &mut (usize, usize), is_first: bool, is_last: bool) {
    let line = |number: usize| Line {
        kind: LineKind::Context,
        old_no: u32::try_from(number as isize + gap.old_offset).ok(),
        new_no: Some(number as u32),
        text: content.get(number - 1).map_or("", String::as_str),
    };
    let (start, hidden, end) = split(&gap.lines, *shown);
    code_rows(ui, start.len(), |i| line(start.start + i));
    if hidden > 0 {
        ui.horizontal(|ui| {
            // only the whole gap when there is not much more of it
            let partly = hidden > MORE_LINES;
            let more = format!("Show {MORE_LINES} more lines");
            if partly && !is_last && ui.small_button("Expand up").on_hover_text(&more).clicked() {
                shown.1 += MORE_LINES;
            }
            if partly && !is_first && ui.small_button("Expand down").on_hover_text(&more).clicked() {
                shown.0 += MORE_LINES;
            }
            if ui.small_button(format!("Expand all {hidden} lines")).clicked() {
                shown.0 += hidden;
            }
        });
    }
    code_rows(ui, end.len(), |i| line(end.start + i));
}

/// A row of a diff.
struct Line<'a> {
    kind: LineKind,
    old_no: Option<u32>,
    new_no: Option<u32>,
    text: &'a str,
}

impl<'a> Line<'a> {
    fn of(line: &'a DiffLine) -> Self {
        Self { kind: line.kind, old_no: line.old_no, new_no: line.new_no, text: &line.text }
    }
}

/// Shows rows of a diff, drawing only the ones that can be seen.
fn code_rows<'a>(ui: &mut egui::Ui, count: usize, line: impl Fn(usize) -> Line<'a>) {
    if count == 0 {
        return;
    }
    let style = CodeStyle::new(ui);
    let (added_bg, removed_bg) = if ui.visuals().dark_mode {
        (Color32::from_rgb(28, 58, 34), Color32::from_rgb(70, 30, 32))
    } else {
        (Color32::from_rgb(218, 251, 225), Color32::from_rgb(255, 226, 226))
    };
    let number_width = style.digit_width * 5.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), style.row_height * count as f32), Sense::hover());
    let clip = ui.clip_rect();
    let first = ((clip.top() - rect.top()) / style.row_height).floor().max(0.0) as usize;
    let last = (((clip.bottom() - rect.top()) / style.row_height).ceil().max(0.0) as usize).min(count);
    let painter = ui.painter();
    for index in first..last {
        let line = line(index);
        let top = rect.top() + style.row_height * index as f32;
        let row = egui::Rect::from_min_size(egui::pos2(rect.left(), top), Vec2::new(rect.width(), style.row_height));
        let (bg, sign) = match line.kind {
            LineKind::Context => (None, " "),
            LineKind::Added => (Some(added_bg), "+"),
            LineKind::Removed => (Some(removed_bg), "-"),
        };
        if let Some(bg) = bg {
            painter.rect_filled(row, 0.0, bg);
        }
        let (x, y) = (row.left(), top + 1.0);
        style.number(painter, x + number_width, y, line.old_no);
        style.number(painter, x + number_width * 2.0, y, line.new_no);
        let code_x = x + number_width * 2.0 + style.digit_width * 2.0;
        style.text(painter, code_x, y, sign, style.weak_color);
        style.text(painter, code_x + style.digit_width * 2.0, y, expand_tabs(line.text), style.text_color);
    }
}

#[cfg(test)]
mod tests {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable as _;

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
    fn marking_a_hunk_reviewed_moves_on_to_the_next_unreviewed_one_in_the_file() {
        let mut review = review(&TempDir::new("moves-on"));
        review.set_diff(DIFF, &BTreeSet::new());
        assert_eq!(review.progress(), (0, 3));
        review.toggle_reviewed(0, 0);
        assert_eq!((review.progress(), review.view(0).current), ((1, 3), 1));
        review.toggle_reviewed(0, 1);
        assert_eq!((review.file_progress(&review.files[0]), review.view(0).current), ((2, 2), 1));
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
    fn shows_more_of_a_gap_from_both_ends_until_all_of_it() {
        assert_eq!(split(&(10..50), (0, 0)), (10..10, 40, 50..50));
        assert_eq!(split(&(10..50), (5, 20)), (10..15, 15, 30..50));
        assert_eq!(split(&(10..50), (25, 20)), (10..50, 0, 50..50));
    }

    #[test]
    fn shows_more_unchanged_lines_when_asked() {
        let dir = TempDir::new("more-lines");
        let lines: Vec<String> = (1..=100).map(|n| format!("line {n}")).collect();
        dir.write("a.txt", lines.join("\n") + "\n");
        let diff = "diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n@@ -47,7 +47,7 @@\n line 47\n line 48\n line 49\n-old 50\n+line 50\n line 51\n line 52\n line 53\n";
        let mut review = review(&dir);
        review.set_diff(diff, &BTreeSet::new());
        let mut harness = Harness::builder().with_size(egui::vec2(800.0, 3000.0)).build_ui_state(|ui, review: &mut Review| review.ui(ui, Some("a.txt"), true), review);
        harness.run();

        // 46 lines above the hunk and 47 below it
        let click = |harness: &mut Harness<'_, Review>, label: &str| {
            harness.query_all_by_label(label).next().unwrap().click();
            harness.run();
        };
        click(&mut harness, "Expand up");
        click(&mut harness, "Expand up");
        click(&mut harness, "Expand down");
        assert_eq!(harness.state_mut().view(0).shown, [(0, 40), (20, 0)]);
        click(&mut harness, "Expand all 6 lines");
        click(&mut harness, "Expand all 27 lines");
        assert_eq!(harness.state_mut().view(0).shown, [(6, 40), (47, 0)]);
        assert!(harness.query_by_label_contains("Expand").is_none());
    }

    #[test]
    fn a_review_without_a_file_or_changes_says_so() {
        let dir = TempDir::new("no-file");
        for (path, label) in [(None, "Choose a file under Changes to review it"), (Some("gone.txt"), "No changes in gone.txt")] {
            let mut harness = Harness::new_ui_state(move |ui, review: &mut Review| review.ui(ui, path, true), review(&dir));
            harness.run();
            harness.get_by_label(label);
        }
    }
}
