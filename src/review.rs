use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

use eframe::egui::{self, Align, Color32, Key, RichText, Sense, Stroke, Vec2};

use crate::code::{CodeStyle, expand_tabs};
use crate::git::{self, FileDiff, Hunk, LineKind};

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

/// All working-tree changes as one stream of hunks to review.
pub struct Review {
    state: ReviewState,
    files: Vec<FileDiff>,
    /// Every hunk in display order.
    hunks: Vec<HunkRef>,
    current: usize,
    scroll_to_current: bool,
}

struct HunkRef {
    file: usize,
    fingerprint: String,
}

impl Review {
    pub fn new(state: ReviewState) -> Self {
        Self { state, files: Vec::new(), hunks: Vec::new(), current: 0, scroll_to_current: false }
    }

    pub fn set_diff(&mut self, raw: &str) {
        let current_fp = self.hunks.get(self.current).map(|h| h.fingerprint.clone());
        self.files = git::parse_diff(raw);
        self.hunks = self
            .files
            .iter()
            .enumerate()
            .flat_map(|(fi, f)| f.hunks.iter().map(move |h| HunkRef { file: fi, fingerprint: h.fingerprint(&f.path) }))
            .collect();
        self.state.retain_only(self.hunks.iter().map(|h| h.fingerprint.as_str()));
        self.current = current_fp
            .and_then(|fp| self.hunks.iter().position(|h| h.fingerprint == fp))
            .unwrap_or(self.current.min(self.hunks.len().saturating_sub(1)));
    }

    /// The number of reviewed hunks and of all hunks.
    pub fn progress(&self) -> (usize, usize) {
        ((0..self.hunks.len()).filter(|&i| self.is_reviewed(i)).count(), self.hunks.len())
    }

    fn is_reviewed(&self, index: usize) -> bool {
        self.state.is_reviewed(&self.hunks[index].fingerprint)
    }

    fn select(&mut self, index: usize) {
        self.current = index;
        self.scroll_to_current = true;
    }

    fn next_unreviewed(&self) -> Option<usize> {
        let n = self.hunks.len();
        (1..=n).map(|i| (self.current + i) % n).find(|&i| !self.is_reviewed(i))
    }

    fn toggle_reviewed(&mut self, index: usize) {
        self.state.toggle(&self.hunks[index].fingerprint);
        self.current = index;
        if self.is_reviewed(index) {
            if let Some(next) = self.next_unreviewed() {
                self.select(next);
            }
        }
    }

    fn handle_keys(&mut self, ctx: &egui::Context) {
        if self.hunks.is_empty() || ctx.memory(|m| m.focused().is_some()) {
            return;
        }
        let pressed = |key| ctx.input(|i| i.modifiers.is_none() && i.key_pressed(key));
        if pressed(Key::J) && self.current + 1 < self.hunks.len() {
            self.select(self.current + 1);
        }
        if pressed(Key::K) && self.current > 0 {
            self.select(self.current - 1);
        }
        if pressed(Key::N) {
            if let Some(next) = self.next_unreviewed() {
                self.select(next);
            }
        }
        if pressed(Key::Space) {
            self.toggle_reviewed(self.current);
        }
    }

    /// Shows the list of changed files. Returns true when a file was clicked, which selects its first hunk.
    pub fn file_list(&mut self, ui: &mut egui::Ui) -> bool {
        let current_file = self.hunks.get(self.current).map(|h| h.file);
        let mut clicked = None;
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            for (fi, file) in self.files.iter().enumerate() {
                let indexes: Vec<usize> = (0..self.hunks.len()).filter(|&i| self.hunks[i].file == fi).collect();
                let done = indexes.iter().filter(|&&i| self.is_reviewed(i)).count();
                let text = format!("{} {} {}/{}", file.status.letter(), file.display_name(), done, indexes.len());
                let mut text = RichText::new(text).monospace();
                if done == indexes.len() {
                    text = text.weak();
                }
                if ui.selectable_label(current_file == Some(fi), text).clicked() {
                    clicked = Some(indexes.first().copied());
                }
            }
        });
        if let Some(Some(index)) = clicked {
            self.select(index);
        }
        clicked.is_some()
    }

    /// Shows the stream of hunks.
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        self.handle_keys(ui.ctx());
        if self.files.is_empty() {
            ui.centered_and_justified(|ui| ui.weak("No changes"));
            return;
        }
        let mut toggled = None;
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            let mut index = 0;
            for file in &self.files {
                ui.add_space(12.0);
                ui.label(RichText::new(file.display_name()).monospace().strong().size(15.0));
                if file.binary {
                    ui.weak("Binary file");
                }
                for hunk in &file.hunks {
                    let reviewed = self.is_reviewed(index);
                    let is_current = index == self.current;
                    let stroke = if is_current {
                        Stroke::new(2.0, ui.visuals().selection.stroke.color)
                    } else {
                        ui.visuals().widgets.noninteractive.bg_stroke
                    };
                    let response = egui::Frame::new()
                        .stroke(stroke)
                        .corner_radius(4.0)
                        .inner_margin(6.0)
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                let mut checked = reviewed;
                                let checkbox = ui.checkbox(&mut checked, "");
                                checkbox.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, ui.is_enabled(), reviewed, "Reviewed"));
                                if checkbox.changed() {
                                    toggled = Some(index);
                                }
                                ui.label(RichText::new(&hunk.header).monospace().weak());
                            });
                            // reviewed hunks collapse to their header to keep the stream focused on what's left
                            if !reviewed {
                                hunk_lines(ui, hunk);
                            }
                        })
                        .response;
                    if is_current && self.scroll_to_current {
                        ui.scroll_to_rect(response.rect, Some(Align::TOP));
                    }
                    ui.add_space(4.0);
                    index += 1;
                }
            }
        });
        self.scroll_to_current = false;
        if let Some(index) = toggled {
            self.toggle_reviewed(index);
        }
    }
}

fn hunk_lines(ui: &mut egui::Ui, hunk: &Hunk) {
    ui.spacing_mut().item_spacing.y = 0.0;
    let style = CodeStyle::new(ui);
    let (added_bg, removed_bg) = if ui.visuals().dark_mode {
        (Color32::from_rgb(28, 58, 34), Color32::from_rgb(70, 30, 32))
    } else {
        (Color32::from_rgb(218, 251, 225), Color32::from_rgb(255, 226, 226))
    };
    let number_width = style.digit_width * 5.0;
    let width = ui.available_width();

    for line in &hunk.lines {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(width, style.row_height), Sense::hover());
        if !ui.is_rect_visible(rect) {
            continue;
        }
        let painter = ui.painter();
        let (bg, sign) = match line.kind {
            LineKind::Context => (None, " "),
            LineKind::Added => (Some(added_bg), "+"),
            LineKind::Removed => (Some(removed_bg), "-"),
        };
        if let Some(bg) = bg {
            painter.rect_filled(rect, 0.0, bg);
        }
        let y = rect.top() + 1.0;
        let x = rect.left();
        style.number(painter, x + number_width, y, line.old_no);
        style.number(painter, x + number_width * 2.0, y, line.new_no);
        let code_x = x + number_width * 2.0 + style.digit_width * 2.0;
        style.text(painter, code_x, y, sign, style.weak_color);
        style.text(painter, code_x + style.digit_width * 2.0, y, expand_tabs(&line.text), style.text_color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    const DIFF: &str = "\
diff --git a/a.txt b/a.txt
--- a/a.txt
+++ b/a.txt
@@ -1 +1 @@
-one
+two
diff --git a/b.txt b/b.txt
--- a/b.txt
+++ b/b.txt
@@ -1 +1 @@
-three
+four
";

    fn review(dir: &TempDir) -> Review {
        Review::new(ReviewState::load(dir.path().join("reviewed")))
    }

    #[test]
    fn marking_a_hunk_reviewed_moves_on_to_the_next_unreviewed_one() {
        let mut review = review(&TempDir::new("moves-on"));
        review.set_diff(DIFF);
        assert_eq!(review.progress(), (0, 2));
        review.toggle_reviewed(0);
        assert_eq!((review.progress(), review.current), ((1, 2), 1));
    }

    #[test]
    fn keeps_the_current_hunk_and_reviewed_marks_when_the_diff_changes_elsewhere() {
        let dir = TempDir::new("keeps-current");
        let mut review = review(&dir);
        review.set_diff(DIFF);
        review.toggle_reviewed(0);
        assert_eq!(review.current, 1);

        // the first file is no longer changed, the second one is untouched
        review.set_diff(&DIFF[DIFF.find("diff --git a/b.txt").unwrap()..]);
        assert_eq!((review.progress(), review.current), ((0, 1), 0));

        // a changed hunk needs to be reviewed again
        review.toggle_reviewed(0);
        review.set_diff(&DIFF.replace("four", "five"));
        assert_eq!(review.progress(), (0, 2));
    }
}
