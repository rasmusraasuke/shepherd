use eframe::egui::{self, Align2, Key};

const MAX_RESULTS: usize = 20;

/// The "open file by name" popup.
#[derive(Default)]
pub struct QuickOpen {
    query: String,
    /// The query the results were found for, to search only when it changes.
    searched: Option<String>,
    results: Vec<String>,
    selected: usize,
}

pub enum Outcome {
    Open(String),
    Closed,
}

impl QuickOpen {
    /// Shows the popup. Returns something once the user picked a file or dismissed the popup.
    pub fn ui(&mut self, ctx: &egui::Context, files: &[String]) -> Option<Outcome> {
        if self.searched.as_ref() != Some(&self.query) {
            self.results = find(files, &self.query, MAX_RESULTS).into_iter().map(str::to_string).collect();
            self.searched = Some(self.query.clone());
            self.selected = 0;
        }
        let pressed = |key| ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, key));
        if pressed(Key::ArrowDown) && self.selected + 1 < self.results.len() {
            self.selected += 1;
        }
        if pressed(Key::ArrowUp) {
            self.selected = self.selected.saturating_sub(1);
        }
        let mut picked = pressed(Key::Enter).then_some(self.selected);

        let id = egui::Id::unique("quick_open");
        let area = egui::Modal::default_area(id).anchor(Align2::CENTER_TOP, [0.0, 60.0]);
        let modal = egui::Modal::new(id).area(area).show(ctx, |ui| {
            ui.set_width(520.0_f32.min(ctx.content_rect().width() - 40.0));
            egui::TextEdit::singleline(&mut self.query).hint_text("Open file by name").desired_width(f32::INFINITY).show(ui).response.request_focus();
            for (index, path) in self.results.iter().enumerate() {
                if ui.selectable_label(index == self.selected, path).clicked() {
                    picked = Some(index);
                }
            }
        });
        match picked.and_then(|index| self.results.get(index)) {
            Some(path) => Some(Outcome::Open(path.clone())),
            None if modal.should_close() => Some(Outcome::Closed),
            None => None,
        }
    }
}

/// The files matching the query, best match first.
fn find<'a>(files: &'a [String], query: &str, limit: usize) -> Vec<&'a str> {
    let query: String = query.chars().filter(|c| !c.is_whitespace()).collect();
    if query.is_empty() {
        return Vec::new();
    }
    let mut found: Vec<(i32, &str)> = files.iter().filter_map(|path| Some((path_score(path, &query)?, path.as_str()))).collect();
    found.sort_by(|(score_a, a), (score_b, b)| score_b.cmp(score_a).then(a.len().cmp(&b.len())).then(a.cmp(b)));
    found.into_iter().take(limit).map(|(_, path)| path).collect()
}

/// A match in the file name is worth more than one spread over the directories.
fn path_score(path: &str, query: &str) -> Option<i32> {
    let name = path.rsplit('/').next().unwrap_or(path);
    score(name, query).map(|score| score + 100).or_else(|| score(path, query))
}

/// How well the query matches the text as a case-insensitive subsequence, or `None` if it doesn't.
/// Consecutive characters and characters at the start of a word count for more.
fn score(text: &str, query: &str) -> Option<i32> {
    let mut wanted = query.chars().peekable();
    let mut score = 0;
    let mut previous: Option<char> = None;
    let mut previous_matched = false;
    for c in text.chars() {
        let Some(want) = wanted.peek() else { break };
        let matched = c.to_lowercase().eq(want.to_lowercase());
        if matched {
            let word_start = previous.is_none_or(|p| !p.is_alphanumeric() || (p.is_lowercase() && c.is_uppercase()));
            score += 1 + if previous_matched { 5 } else { 0 } + if word_start { 10 } else { 0 };
            wanted.next();
        }
        previous_matched = matched;
        previous = Some(c);
    }
    wanted.peek().is_none().then_some(score)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files() -> Vec<String> {
        ["src/app.rs", "src/git.rs", "src/review.rs", "docs/app/readme.md", "src/FileTree.kt", "src/filter.kt"].map(str::to_string).to_vec()
    }

    #[test]
    fn empty_query_finds_nothing() {
        assert!(find(&files(), "  ", 10).is_empty());
    }

    #[test]
    fn finds_subsequences_ignoring_case() {
        assert_eq!(find(&files(), "GIT", 10), ["src/git.rs"]);
        assert_eq!(find(&files(), "rvw", 10), ["src/review.rs"]);
        assert!(find(&files(), "xyz", 10).is_empty());
    }

    #[test]
    fn file_name_matches_come_before_directory_matches() {
        assert_eq!(find(&files(), "app", 10), ["src/app.rs", "docs/app/readme.md"]);
    }

    #[test]
    fn word_starts_beat_scattered_characters() {
        assert_eq!(find(&files(), "ft", 10), ["src/FileTree.kt", "src/filter.kt"]);
    }

    #[test]
    fn directories_can_be_part_of_the_query() {
        assert_eq!(find(&files(), "docs/read", 10), ["docs/app/readme.md"]);
    }

    #[test]
    fn respects_the_limit() {
        assert_eq!(find(&files(), "s", 2).len(), 2);
    }
}
