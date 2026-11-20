use eframe::egui::{self, Align2, RichText, Sense, TextStyle, Vec2};

use super::graph::{self, Row};
use crate::git::{Commit, CommitRange, PAGE, Repo};
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
    commit: Commit,
}

#[derive(Default)]
struct Log {
    commits: Vec<Commit>,
    graph: Vec<Row>,
    complete: bool,
}

impl Log {
    fn push(&mut self, page: Vec<Commit>, asked: usize) {
        self.complete = page.len() < asked;
        self.commits.extend(page);
        self.graph = graph::layout(&self.commits);
    }
}

pub struct History {
    repo: Repo,
    first: Vec<Commit>,
    branch: Log,
    file: Option<String>,
    file_log: Log,
    selected: Option<Selected>,
    error: Option<String>,
    page: usize,
}

impl History {
    pub fn new(repo: Repo) -> Self {
        Self {
            repo,
            first: Vec::new(),
            branch: Log::default(),
            file: None,
            file_log: Log::default(),
            selected: None,
            error: None,
            page: PAGE,
        }
    }

    pub fn set_commits(&mut self, commits: Vec<Commit>) {
        if commits == self.first {
            return;
        }
        self.first = commits.clone();
        let loaded = self.branch.commits.len();
        self.branch = Log::default();
        if loaded > self.page {
            match self.repo.log(0, loaded) {
                Ok(commits) => self.branch.push(commits, loaded),
                Err(error) => self.error = Some(error),
            }
        } else {
            self.branch.push(commits, self.page);
        }
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
        self.file_log = Log::default();
        self.selected = None;
        if let Some(index) = self
            .branch
            .commits
            .iter()
            .position(|commit| commit.hash == hash)
        {
            return self.open(index);
        }
        let range = self.repo.commit_range(hash).ok()?;
        let commit = self.repo.commit_info(hash).ok()?;
        self.select(range, commit)
    }

    fn load_file(&mut self) {
        self.file_log = Log::default();
        self.error = None;
        if self.file.is_some() {
            self.load_more();
        }
    }

    fn load_more(&mut self) {
        let skip = self.log().commits.len();
        let page = match &self.file {
            Some(path) => self.repo.file_log(path, skip, self.page),
            None => self.repo.log(skip, self.page),
        };
        match page {
            Ok(page) => {
                let log = if self.file.is_some() {
                    &mut self.file_log
                } else {
                    &mut self.branch
                };
                log.push(page, self.page);
            }
            Err(error) => self.error = Some(error),
        }
    }

    fn log(&self) -> &Log {
        if self.file.is_some() {
            &self.file_log
        } else {
            &self.branch
        }
    }

    fn list(&self) -> &[Commit] {
        &self.log().commits
    }

    fn open(&mut self, index: usize) -> Option<Outcome> {
        let commit = self.list().get(index)?.clone();
        let range = self.repo.commit_range(&commit.hash).ok()?;
        self.select(range, commit)
    }

    fn select(&mut self, range: CommitRange, commit: Commit) -> Option<Outcome> {
        let label = commit.short.clone();
        self.selected = Some(Selected {
            range: range.clone(),
            label: label.clone(),
            commit,
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
        let log = self.log();
        let commits = &log.commits;
        let graph: &[Row] = if self.file.is_some() { &[] } else { &log.graph };
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
        let mut more = false;
        let mut area = egui::ScrollArea::vertical()
            .id_salt("history")
            .auto_shrink(false);
        if self.selected.is_some() {
            area = area.max_height(ui.available_height() * 0.55);
        }
        area.show_rows(ui, ROW_HEIGHT, commits.len(), |ui, range| {
            more = !log.complete && range.end + self.page / 3 >= commits.len();
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
                let background = if chosen {
                    ui.visuals().selection.bg_fill
                } else if response.hovered() {
                    ui.visuals().widgets.hovered.weak_bg_fill
                } else {
                    ui.visuals().panel_fill
                };
                if chosen || response.hovered() {
                    ui.painter().rect_filled(rect, 2.0, background);
                }
                let painter = ui.painter().with_clip_rect(rect);
                let mut left = rect.left() + 4.0;
                let mut color = theme::palette(ui.visuals().dark_mode).accent;
                if let Some(row) = graph.get(index) {
                    row.paint(&painter, rect.with_min_x(left), background);
                    left += row.width() + 6.0;
                    color = row.color();
                }
                let middle = rect.top() + 4.0 + ui.fonts_mut(|fonts| fonts.row_height(&font)) / 2.0;
                let subject = graph::paint_refs(ui, &painter, left, middle, color, &commit.refs);
                painter.text(
                    egui::pos2(subject, middle),
                    Align2::LEFT_CENTER,
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
                if response.on_hover_ui(|ui| details(ui, commit)).clicked() {
                    clicked = Some(index);
                }
            }
        });
        if more {
            self.load_more();
            ui.ctx().request_repaint();
        }
        if let Some(index) = clicked {
            outcome = self.open(index);
            if let Some(Outcome::Open { new_tab, .. }) = &mut outcome {
                *new_tab = ui.input(|i| i.modifiers.command);
            }
        }
        outcome
    }

    pub fn details_ui(&self, ui: &mut egui::Ui) {
        let Some(selected) = &self.selected else {
            return;
        };
        egui::ScrollArea::vertical()
            .id_salt("commit message")
            .max_height(ui.available_height() * 0.4)
            .auto_shrink([false, true])
            .show(ui, |ui| details(ui, &selected.commit));
    }

    pub fn selected(&self) -> Option<(&CommitRange, &str)> {
        self.selected
            .as_ref()
            .map(|selected| (&selected.range, selected.label.as_str()))
    }
}

fn details(ui: &mut egui::Ui, commit: &Commit) {
    ui.set_max_width(ui.available_width().min(420.0));
    ui.spacing_mut().item_spacing.y = 4.0;
    ui.label(RichText::new(&commit.subject).strong());
    if !commit.body.is_empty() {
        ui.label(&commit.body);
    }
    if !commit.refs.is_empty() {
        ui.weak(commit.refs.join(", "));
    }
    ui.weak(format!("{} · {}", commit.short, commit.author));
    ui.weak(when(commit));
}

fn when(commit: &Commit) -> String {
    if commit.date.is_empty() {
        commit.time.clone()
    } else {
        format!("{} ({})", commit.date, commit.time)
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
        history.set_commits(repo.log(0, PAGE).unwrap());
        (history, repo, dir)
    }

    #[test]
    fn opens_a_commit() {
        let (mut history, _repo, _dir) = history("history-open");
        let commits = history.branch.commits.clone();
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
        let hash = repo.log(0, PAGE).unwrap()[1].hash.clone();
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

    #[test]
    fn loads_older_commits_page_by_page() {
        let (repo, dir) = repo_with_three("history-more");
        dir.write("a.txt", "d\n");
        commit_all(&repo);
        let mut history = History::new(repo.clone());
        history.page = 2;
        history.set_commits(repo.log(0, 2).unwrap());
        assert_eq!(history.list().len(), 2);
        assert!(!history.branch.complete);
        history.load_more();
        assert_eq!(history.list().len(), 4);
        assert!(!history.branch.complete);
        history.load_more();
        assert_eq!(history.list().len(), 4);
        assert!(history.branch.complete);
        assert_eq!(history.branch.graph.len(), 4);

        history.set_commits(repo.log(0, 2).unwrap());
        assert_eq!(history.list().len(), 4);
        dir.write("a.txt", "e\n");
        commit_all(&repo);
        history.set_commits(repo.log(0, 2).unwrap());
        assert_eq!(history.list().len(), 4);
        assert_eq!(history.list()[0].hash, repo.log(0, 1).unwrap()[0].hash);
        history.load_more();
        assert_eq!(history.list().len(), 5);
    }

    #[test]
    fn keeps_the_whole_commit_that_is_shown() {
        let (mut history, repo, _dir) = history("history-details");
        let hash = repo.log(0, PAGE).unwrap()[2].hash.clone();
        history.branch = Log::default();
        history.show_commit(&hash).unwrap();
        let commit = &history.selected.as_ref().unwrap().commit;
        assert_eq!(commit.hash, hash);
        assert!(when(commit).ends_with(&format!(" ({})", commit.time)));
    }
}
