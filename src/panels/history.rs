use eframe::egui::{self, Align2, RichText, Sense, TextStyle, Vec2};

use crate::git::{Commit, CommitRange, Repo};
use crate::ui::theme;

const ROW_HEIGHT: f32 = 34.0;

#[derive(PartialEq, Debug)]
pub enum Outcome {
    Open {
        range: CommitRange,
        label: String,
        file: Option<String>,
        new_tab: bool,
    },
}

#[derive(Clone, PartialEq, Debug)]
struct Selected {
    range: CommitRange,
    label: String,
}

pub struct History {
    repo: Repo,
    commits: Vec<Commit>,
    file: Option<String>,
    file_commits: Vec<Commit>,
    selected: Option<Selected>,
    error: Option<String>,
}

impl History {
    pub fn new(repo: Repo) -> Self {
        Self {
            repo,
            commits: Vec::new(),
            file: None,
            file_commits: Vec::new(),
            selected: None,
            error: None,
        }
    }

    pub fn set_commits(&mut self, commits: Vec<Commit>) {
        if commits == self.commits {
            return;
        }
        self.commits = commits;
        if self.file.is_some() {
            self.load_file();
        }
    }

    pub fn show_file(&mut self, path: Option<String>) {
        self.file = path;
        self.selected = None;
        self.load_file();
    }

    pub fn show_commit(&mut self, hash: &str) -> Option<Outcome> {
        self.file = None;
        self.file_commits.clear();
        self.selected = None;
        if let Some(index) = self.commits.iter().position(|commit| commit.hash == hash) {
            return self.open(index);
        }
        let range = self.repo.commit_range(hash).ok()?;
        let label = hash[..hash.len().min(7)].to_string();
        self.selected = Some(Selected {
            range: range.clone(),
            label: label.clone(),
        });
        Some(Outcome::Open {
            range,
            label,
            file: None,
            new_tab: false,
        })
    }

    fn load_file(&mut self) {
        let Some(path) = &self.file else {
            self.file_commits.clear();
            self.error = None;
            return;
        };
        match self.repo.file_log(path) {
            Ok(commits) => {
                self.file_commits = commits;
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
    }

    fn list(&self) -> &[Commit] {
        if self.file.is_some() {
            &self.file_commits
        } else {
            &self.commits
        }
    }

    fn open(&mut self, index: usize) -> Option<Outcome> {
        let commit = self.list().get(index)?.clone();
        let range = self.repo.commit_range(&commit.hash).ok()?;
        let label = commit.short.clone();
        self.selected = Some(Selected {
            range: range.clone(),
            label: label.clone(),
        });
        Some(Outcome::Open {
            range,
            label,
            file: self.file.clone(),
            new_tab: false,
        })
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) -> Option<Outcome> {
        let mut outcome = None;
        if let Some(path) = self.file.clone() {
            let mut all = false;
            egui::Sides::new().shrink_left().truncate().show(
                ui,
                |ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    ui.weak("History of");
                    ui.label(RichText::new(path).strong());
                },
                |ui| {
                    all = ui.add(theme::quiet("All commits")).clicked();
                },
            );
            if all {
                self.show_file(None);
            }
            ui.add_space(4.0);
        }
        if let Some(error) = &self.error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        let commits = self.list();
        if commits.is_empty() {
            ui.weak(if self.file.is_some() {
                "No commits changed this file"
            } else {
                "No commits"
            });
            return outcome;
        }
        let font = TextStyle::Body.resolve(ui.style());
        let small = TextStyle::Small.resolve(ui.style());
        ui.spacing_mut().item_spacing.y = 0.0;
        let mut clicked = None;
        let mut area = egui::ScrollArea::vertical()
            .id_salt("history")
            .auto_shrink(false);
        if self.selected.is_some() {
            area = area.max_height(ui.available_height() * 0.55);
        }
        area.show_rows(ui, ROW_HEIGHT, commits.len(), |ui, range| {
            for (index, commit) in commits.iter().enumerate().take(range.end).skip(range.start) {
                let (rect, response) = ui.allocate_exact_size(
                    Vec2::new(ui.available_width(), ROW_HEIGHT),
                    Sense::click(),
                );
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), &commit.subject)
                });
                let chosen = self
                    .selected
                    .as_ref()
                    .is_some_and(|selected| selected.range.head == commit.hash);
                if chosen {
                    ui.painter()
                        .rect_filled(rect, 2.0, ui.visuals().selection.bg_fill);
                } else if response.hovered() {
                    ui.painter()
                        .rect_filled(rect, 2.0, ui.visuals().widgets.hovered.weak_bg_fill);
                }
                let painter = ui.painter().with_clip_rect(rect);
                let left = rect.left() + 4.0;
                painter.text(
                    egui::pos2(left, rect.top() + 5.0),
                    Align2::LEFT_TOP,
                    &commit.subject,
                    font.clone(),
                    ui.visuals().text_color(),
                );
                painter.text(
                    egui::pos2(left, rect.top() + 20.0),
                    Align2::LEFT_TOP,
                    format!("{} · {} · {}", commit.short, commit.author, commit.time),
                    small.clone(),
                    ui.visuals().weak_text_color(),
                );
                if response.clicked() {
                    clicked = Some(index);
                }
            }
        });
        if let Some(index) = clicked {
            outcome = self.open(index);
            if let Some(Outcome::Open { new_tab, .. }) = &mut outcome {
                *new_tab = ui.input(|i| i.modifiers.command);
            }
        }
        outcome
    }

    pub fn selected(&self) -> Option<(&CommitRange, &str)> {
        self.selected
            .as_ref()
            .map(|selected| (&selected.range, selected.label.as_str()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::tests::{commit_all, temp_repo};
    use crate::testing::TempDir;

    fn repo_with_three(name: &str) -> (Repo, TempDir) {
        let (repo, dir) = temp_repo(name, &[("a.txt", "a\n")]);
        commit_all(&repo);
        dir.write("a.txt", "b\n");
        commit_all(&repo);
        dir.write("a.txt", "c\n");
        commit_all(&repo);
        (repo, dir)
    }

    fn history(name: &str) -> (History, Repo, TempDir) {
        let (repo, dir) = repo_with_three(name);
        let mut history = History::new(repo.clone());
        history.set_commits(repo.log().unwrap());
        (history, repo, dir)
    }

    #[test]
    fn opens_a_commit() {
        let (mut history, _repo, _dir) = history("history-open");
        let commits = history.commits.clone();
        let first = history.open(0).unwrap();
        let Outcome::Open { range, label, .. } = first;
        assert_eq!(range.head, commits[0].hash);
        assert_eq!(label, commits[0].short);
        assert_eq!(history.selected().unwrap().0.head, commits[0].hash);

        let other = history.open(2).unwrap();
        let Outcome::Open { range, label, .. } = other;
        assert_eq!(range.head, commits[2].hash);
        assert_eq!(label, commits[2].short);
    }

    #[test]
    fn shows_a_commit_that_is_on_the_branch() {
        let (mut history, repo, _dir) = history("history-show");
        let hash = repo.log().unwrap()[1].hash.clone();
        let Outcome::Open { range, .. } = history.show_commit(&hash).unwrap();
        assert_eq!(range.head, hash);
    }

    #[test]
    fn shows_the_commits_of_one_file() {
        let (mut history, _repo, _dir) = history("history-file");
        history.show_file(Some("a.txt".to_string()));
        assert_eq!(history.list().len(), 3);
        history.show_file(Some("missing.txt".to_string()));
        assert!(history.list().is_empty());
        history.show_file(None);
        assert_eq!(history.list().len(), 3);
    }
}
