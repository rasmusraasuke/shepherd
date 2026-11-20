pub mod comments;
mod draft;
mod line_diff;
mod rows;
mod text;
mod word_diff;

use std::cell::Cell;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::hash::{DefaultHasher, Hash as _, Hasher as _};
use std::ops::Range;
use std::sync::mpsc::{self, Receiver};
use std::thread;

use eframe::egui::{self, Color32, Key, Modifiers, RichText};

use crate::editor::Editor;
use crate::editor::buffer::Buffer;
use crate::editor::file_view::OpenFiles;
use crate::git::{self, Changes, DiffLine, FileDiff, FileStatus, Hunk, LineKind, Repo, Staged};
use crate::panels::file_tree::{FileLook, FileTree};
use crate::ui::theme;
use comments::Comment;

pub use draft::new_agent_button;
use draft::{Draft, draft_ui, send_button};
use rows::{Clicked, fade, place_in_file, reviewed_mark_ui, rows_ui};
use text::{FileText, Gap, changed_in_lines, file_lines, gaps, read_lines};

pub const NEW_COLOR: Color32 = theme::DARK.accent;

const CONTEXT: usize = 3;

pub struct Review {
    repo: Repo,
    files: Vec<FileReview>,
    by_path: HashMap<String, usize>,
    tree: FileTree,
    views: HashMap<String, View>,
    texts: HashMap<String, FileText>,
    progress: HashMap<String, (usize, usize)>,
    error: Option<String>,
    changed: bool,
    comments: Vec<Comment>,
    seen: Option<HashSet<u64>>,
    unseen: HashSet<String>,
    rolled_back: Vec<String>,
    staging: Option<Receiver<Result<(), String>>>,
    pending: Vec<StageJob>,
}

struct StageJob {
    repo: Repo,
    paths: Vec<String>,
    versions: Vec<(String, Option<Vec<u8>>)>,
    renamed: Vec<String>,
    reviewed: bool,
    ctx: egui::Context,
}

impl StageJob {
    fn start(self) -> Receiver<Result<(), String>> {
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let result = self.run();
            let _ = sender.send(result);
            self.ctx.request_repaint();
        });
        receiver
    }

    fn run(&self) -> Result<(), String> {
        if !self.paths.is_empty() {
            let paths: Vec<&str> = self.paths.iter().map(String::as_str).collect();
            self.repo.stage_files(&paths, self.reviewed)?;
        }
        for (path, content) in &self.versions {
            self.repo.stage_version(path, content.as_deref())?;
        }
        for old in &self.renamed {
            self.repo.remove_from_index(old)?;
        }
        Ok(())
    }
}

#[derive(Default)]
pub struct Agents {
    pub open: Vec<(u64, String)>,
    pub new: Option<String>,
}

#[derive(Debug, PartialEq)]
pub enum Outcome {
    Show(String),
    Send { agent: Option<u64>, text: String },
    Open { path: String, line: usize },
}

struct FileReview {
    diff: FileDiff,
    staged: Vec<Vec<bool>>,
    file_staged: bool,
    fingerprints: Vec<u64>,
    new: Vec<bool>,
}

fn fingerprint(path: &str, hunk: &Hunk) -> u64 {
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    for line in &hunk.lines {
        (line.kind, &line.text).hash(&mut hasher);
    }
    hasher.finish()
}

impl FileReview {
    fn lines_progress(&self, hunks: Range<usize>) -> (usize, usize) {
        let lines = hunks.flat_map(|h| self.diff.hunks[h].lines.iter().zip(&self.staged[h]));
        let changed = lines.filter(|(line, _)| line.kind != LineKind::Context);
        changed.fold((0, 0), |(done, total), (_, staged)| {
            (done + *staged as usize, total + 1)
        })
    }

    fn progress(&self) -> (usize, usize) {
        match self.diff.hunks.len() {
            0 => (self.file_staged as usize, 1),
            hunks => self.lines_progress(0..hunks),
        }
    }

    fn hunk_progress(&self, hunk: usize) -> (usize, usize) {
        self.lines_progress(hunk..hunk + 1)
    }

    fn is_reviewed(&self, hunk: usize) -> bool {
        let (done, total) = self.hunk_progress(hunk);
        done == total
    }

    fn mark(&mut self, reviewed: bool) {
        if self.diff.hunks.is_empty() {
            self.file_staged = reviewed;
            return;
        }
        for (hunk, staged) in self.diff.hunks.iter().zip(&mut self.staged) {
            for (line, staged) in hunk.lines.iter().zip(staged) {
                *staged = line.kind == LineKind::Context || reviewed;
            }
        }
    }
}

#[derive(Default)]
struct View {
    current: usize,
    scroll_to_current: bool,
    expanded: Vec<bool>,
    selection: Option<(usize, usize)>,
    selecting: bool,
    draft: String,
    focus_draft: bool,
    menu_row: Option<usize>,
    editor: Editor,
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
            changed: false,
            comments: Vec::new(),
            seen: None,
            unseen: HashSet::new(),
            rolled_back: Vec::new(),
            staging: None,
            pending: Vec::new(),
        }
    }

    pub fn set_changes(&mut self, changes: &Changes, collapsed: &BTreeSet<String>) {
        let staged = Staged::new(changes);
        let before: HashMap<String, FileReview> = self
            .files
            .drain(..)
            .map(|file| (file.diff.path.clone(), file))
            .collect();
        self.files = git::parse_diff(&changes.diff)
            .into_iter()
            .map(|diff| {
                let lines = diff
                    .hunks
                    .iter()
                    .map(|hunk| {
                        hunk.lines
                            .iter()
                            .map(|line| staged.is_staged(&diff.path, line))
                            .collect()
                    })
                    .collect();
                let fingerprints: Vec<u64> = diff
                    .hunks
                    .iter()
                    .map(|hunk| fingerprint(&diff.path, hunk))
                    .collect();
                FileReview {
                    staged: lines,
                    file_staged: staged.is_file_staged(&diff.path),
                    new: vec![false; fingerprints.len()],
                    fingerprints,
                    diff,
                }
            })
            .collect();
        self.by_path = self
            .files
            .iter()
            .enumerate()
            .map(|(index, file)| (file.diff.path.clone(), index))
            .collect();
        self.tree.set_files(
            self.files
                .iter()
                .map(|file| file.diff.path.clone())
                .collect(),
            collapsed,
        );
        let shown: HashSet<String> = self.texts.drain().map(|(path, _)| path).collect();
        self.error = None;
        self.update_progress();
        let fingerprints: HashSet<u64> = self
            .files
            .iter()
            .flat_map(|file| file.fingerprints.iter().copied())
            .collect();
        match &mut self.seen {
            Some(seen) => seen.retain(|fingerprint| {
                fingerprints.contains(fingerprint)
                    || before
                        .values()
                        .any(|file| file.fingerprints.contains(fingerprint))
            }),
            None => self.seen = Some(fingerprints),
        }
        self.update_new();
        let (files, by_path) = (&self.files, &self.by_path);
        self.views.retain(|path, view| {
            let Some(&index) = by_path.get(path) else {
                return false;
            };
            let hunks = &files[index].diff.hunks;
            if shown.contains(path) {
                return true;
            }
            if view.expanded.len() != hunks.len() + 1 {
                view.expanded.clear();
            }
            if before
                .get(path)
                .is_none_or(|file| file.diff.hunks != *hunks)
            {
                view.selection = None;
            }
            view.current = view.current.min(hunks.len().saturating_sub(1));
            true
        });
        let mut lines = HashMap::new();
        self.comments.retain_mut(|comment| {
            let Some(&index) = by_path.get(&comment.path) else {
                return false;
            };
            let diff = &files[index].diff;
            shown.contains(&diff.path)
                || comment.follow(
                    lines
                        .entry(&diff.path)
                        .or_insert_with(|| file_lines(&read_lines(&self.repo.root, diff), diff)),
                )
        });
    }

    pub fn files_progress(&self) -> Vec<(&str, (usize, usize))> {
        self.tree
            .files()
            .iter()
            .map(|path| (path.as_str(), self.progress_of(path)))
            .collect()
    }

    pub fn first_unreviewed(&self) -> Option<String> {
        self.files_progress()
            .into_iter()
            .find(|(_, (done, total))| done < total)
            .map(|(path, _)| path.to_string())
    }

    pub fn comments(&self) -> &[Comment] {
        &self.comments
    }

    pub fn take_changed(&mut self) -> bool {
        self.poll_staging();
        std::mem::take(&mut self.changed)
    }

    fn poll_staging(&mut self) {
        let Some(receiver) = &self.staging else {
            return;
        };
        let Ok(result) = receiver.try_recv() else {
            return;
        };
        self.staging = None;
        self.staging_done(result);
        self.start_pending();
    }

    fn start_pending(&mut self) {
        if self.staging.is_none() && !self.pending.is_empty() {
            self.staging = Some(self.pending.remove(0).start());
        }
    }

    #[cfg(test)]
    fn wait_for_staging(&mut self) {
        loop {
            let Some(receiver) = self.staging.take() else {
                return;
            };
            if let Ok(result) = receiver.recv() {
                self.staging_done(result);
            }
            self.start_pending();
        }
    }

    pub fn take_rolled_back(&mut self) -> Vec<String> {
        std::mem::take(&mut self.rolled_back)
    }

    pub fn roll_back(&mut self, path: &str) {
        let old = self
            .by_path
            .get(path)
            .and_then(|&file| self.files[file].diff.old_path.clone());
        let mut paths = vec![path.to_string()];
        paths.extend(old);
        for path in paths {
            match self.repo.roll_back(&path) {
                Ok(files) => self.rolled_back.extend(files),
                Err(error) => self.error = Some(error),
            }
        }
        self.changed = true;
    }

    pub fn show_line(&mut self, path: &str, line: usize) {
        let Some(&file) = self.by_path.get(path) else {
            return;
        };
        let line = line as u32 + 1;
        let distance = |hunk: &Hunk| {
            let new = hunk.ranges().1;
            if line < new.start {
                new.start - line
            } else {
                (line + 1).saturating_sub(new.end)
            }
        };
        let hunks = &self.files[file].diff.hunks;
        if let Some(hunk) = (0..hunks.len()).min_by_key(|&hunk| distance(&hunks[hunk])) {
            self.select(file, hunk);
        }
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

    fn update_new(&mut self) {
        self.unseen.clear();
        let seen = self.seen.get_or_insert_default();
        for file in &mut self.files {
            file.new = file
                .fingerprints
                .iter()
                .map(|fingerprint| !seen.contains(fingerprint))
                .collect();
            if file.new.contains(&true) {
                let path = file.diff.path.as_str();
                self.unseen.extend(
                    path.match_indices('/')
                        .map(|(end, _)| path[..end].to_string())
                        .chain([path.to_string()]),
                );
            }
        }
    }

    fn see(&mut self, file: usize, hunk: usize) {
        if let Some(&fingerprint) = self.files[file].fingerprints.get(hunk) {
            self.seen.get_or_insert_default().insert(fingerprint);
            self.update_new();
        }
    }

    fn make_current(&mut self, file: usize, hunk: usize) {
        self.view(file).current = hunk;
        self.see(file, hunk);
    }

    pub fn progress(&self) -> (usize, usize) {
        self.progress_of("")
    }

    pub fn status(&self, path: &str) -> Option<FileStatus> {
        self.by_path
            .get(path)
            .map(|&index| self.files[index].diff.status)
    }

    fn progress_of(&self, path: &str) -> (usize, usize) {
        self.progress.get(path).copied().unwrap_or_default()
    }

    fn next_unreviewed(&self, file: usize, from: usize) -> Option<usize> {
        let file = &self.files[file];
        let n = file.diff.hunks.len();
        (1..=n)
            .map(|i| (from + i) % n)
            .find(|&i| !file.is_reviewed(i))
    }

    fn view(&mut self, file: usize) -> &mut View {
        self.views
            .entry(self.files[file].diff.path.clone())
            .or_default()
    }

    fn select(&mut self, file: usize, hunk: usize) {
        self.make_current(file, hunk);
        self.view(file).scroll_to_current = true;
    }

    fn next(&mut self, file: usize) -> Option<String> {
        let current = self.view(file).current;
        if let Some(hunk) = self.next_unreviewed(file, current) {
            self.select(file, hunk);
            return None;
        }
        let files = self.tree.files();
        let at = files
            .iter()
            .position(|path| *path == self.files[file].diff.path)
            .unwrap_or(0);
        let next = (1..files.len())
            .map(|i| &files[(at + i) % files.len()])
            .find(|path| {
                let (done, total) = self.progress_of(path);
                done < total
            })?;
        let (next, index) = (next.clone(), self.by_path[next]);
        let first = self
            .next_unreviewed(index, self.files[index].diff.hunks.len().saturating_sub(1))
            .unwrap_or(0);
        self.select(index, first);
        Some(next)
    }

    fn stage(&mut self, file: usize, chosen: impl Fn(usize, usize) -> bool) {
        let FileReview { diff, staged, .. } = &mut self.files[file];
        let old = diff.old_path.clone();
        let result = match self.texts.get_mut(&diff.path) {
            Some(text) => {
                let version = text.version(&diff.hunks, &chosen);
                let result = self
                    .repo
                    .stage_version(&diff.path, version.as_deref())
                    .and_then(|()| match &old {
                        Some(old) => self.repo.remove_from_index(old),
                        None => Ok(()),
                    });
                if result.is_ok() {
                    text.set_index(&String::from_utf8_lossy(&version.unwrap_or_default()));
                }
                result
            }
            None => self.repo.stage_lines(diff, &chosen),
        };
        if result.is_ok() {
            for (h, (hunk, staged)) in diff.hunks.iter().zip(staged).enumerate() {
                for (i, (line, staged)) in hunk.lines.iter().zip(staged).enumerate() {
                    *staged = line.kind == LineKind::Context || chosen(h, i);
                }
            }
        }
        self.staging_done(result);
    }

    fn stage_file(&mut self, file: usize, staged: bool) {
        let diff = &self.files[file].diff;
        let result = self
            .repo
            .stage_file(&diff.path, staged)
            .and_then(|()| match &diff.old_path {
                Some(old) => self.repo.stage_file(old, staged),
                None => Ok(()),
            });
        if result.is_ok() {
            self.files[file].file_staged = staged;
        }
        self.staging_done(result);
    }

    fn staging_done(&mut self, result: Result<(), String>) {
        self.error = result.err();
        self.changed = true;
        self.update_progress();
    }

    fn revert(
        &mut self,
        file: usize,
        buffer: &mut Buffer,
        reverted: impl Fn(usize, usize) -> bool,
    ) {
        let FileReview { diff, staged, .. } = &self.files[file];
        let is_reverted =
            |h: usize, i: usize| diff.hunks[h].lines[i].kind != LineKind::Context && reverted(h, i);
        let affected: Vec<usize> = (0..diff.hunks.len())
            .filter(|&h| (0..diff.hunks[h].lines.len()).any(|i| is_reverted(h, i)))
            .collect();
        let (Some(&first), Some(&last)) = (affected.first(), affected.last()) else {
            return;
        };
        if affected
            .iter()
            .any(|&h| (0..diff.hunks[h].lines.len()).any(|i| is_reverted(h, i) && staged[h][i]))
        {
            let staged = staged.clone();
            self.stage(file, |h, i| staged[h][i] && !reverted(h, i));
        }
        let (hunks, work) = (
            &self.files[file].diff.hunks,
            &self.texts[&self.files[file].diff.path].work_lines,
        );
        let start = hunks[first].ranges().1.start as usize - 1;
        let (mut at, mut lines) = (start, Vec::new());
        for (h, hunk) in hunks.iter().enumerate().take(last + 1).skip(first) {
            let new = hunk.ranges().1;
            lines.extend_from_slice(&work[at..new.start as usize - 1]);
            for (i, line) in hunk.lines.iter().enumerate() {
                if (line.kind == LineKind::Removed) == reverted(h, i) {
                    lines.push(line.text.clone());
                }
            }
            at = new.end as usize - 1;
        }
        buffer.replace_lines(start..at, &lines);
    }

    fn follow_edits(&mut self, file: usize, buffer: &Buffer, editing: bool) {
        let path = self.files[file].diff.path.clone();
        let text = self.texts.get_mut(&path).expect("the text of a file shown");
        if text.revision == Some(buffer.revision()) {
            return;
        }
        let lines = buffer.lines();
        let work = &lines[..lines.len() - (lines.last().is_some_and(String::is_empty)) as usize];
        let before = gaps(&self.files[file].diff.hunks, text.work_lines.len());
        let hunks = line_diff::diff(&text.old.lines, work);
        let unstaged: HashSet<u32> = line_diff::diff(&text.index, work)
            .iter()
            .flat_map(|hunk| &hunk.lines)
            .filter_map(|line| line.new_no)
            .collect();
        let staged = hunks
            .iter()
            .map(|hunk| {
                let staged = |line: &DiffLine| match line.kind {
                    LineKind::Context => true,
                    LineKind::Removed => line
                        .old_no
                        .is_some_and(|number| text.staged_removals.contains(&number)),
                    LineKind::Added => line
                        .new_no
                        .is_some_and(|number| !unstaged.contains(&number)),
                };
                hunk.lines.iter().map(staged).collect()
            })
            .collect();
        text.changed = hunks
            .iter()
            .map(|hunk| changed_in_lines(hunk, &text.old.lines, work))
            .collect();
        text.longest = text
            .old
            .lines
            .iter()
            .chain(work)
            .map(|line| line.chars().count())
            .max()
            .unwrap_or(0);
        text.work = buffer.text();
        text.work_lines = work.to_vec();
        text.revision = Some(buffer.revision());

        let review = &mut self.files[file];
        review.fingerprints = hunks.iter().map(|hunk| fingerprint(&path, hunk)).collect();
        review.new = vec![false; hunks.len()];
        review.staged = staged;
        let changed = review.diff.hunks != hunks;
        review.diff.hunks = hunks;
        if editing {
            self.seen
                .get_or_insert_default()
                .extend(review.fingerprints.iter().copied());
        }
        let file_lines = file_lines(&text.work_lines, &review.diff);
        let after = gaps(&review.diff.hunks, text.work_lines.len());
        self.comments
            .retain_mut(|comment| comment.path != path || comment.follow(&file_lines));
        self.update_progress();
        self.update_new();
        let view = self.view(file);
        if changed {
            view.selection = None;
        }
        if view.expanded.len() != after.len() {
            let shown = |gap: &Gap| {
                before.iter().zip(&view.expanded).any(|(was, &expanded)| {
                    expanded && was.lines.start < gap.lines.end && gap.lines.start < was.lines.end
                })
            };
            view.expanded = after.iter().map(shown).collect();
        }
        view.current = view.current.min(after.len().saturating_sub(2));
    }

    fn toggle_reviewed(&mut self, file: usize, hunk: usize) -> Option<String> {
        let reviewed = !self.files[file].is_reviewed(hunk);
        let staged = self.files[file].staged.clone();
        self.stage(file, |h, i| if h == hunk { reviewed } else { staged[h][i] });
        self.make_current(file, hunk);
        if reviewed && self.files[file].is_reviewed(hunk) {
            self.next(file)
        } else {
            None
        }
    }

    fn toggle_line(&mut self, file: usize, hunk: usize, line: usize) {
        let staged = self.files[file].staged.clone();
        self.stage(file, |h, i| staged[h][i] != ((h, i) == (hunk, line)));
        self.make_current(file, hunk);
    }

    pub fn set_reviewed(&mut self, path: &str, reviewed: bool, ctx: &egui::Context) {
        let inside = |file: &str| {
            path.is_empty()
                || file
                    .strip_prefix(path)
                    .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
        };
        let files: Vec<usize> = (0..self.files.len())
            .filter(|&index| inside(&self.files[index].diff.path))
            .collect();
        let mut paths = Vec::new();
        let mut versions = Vec::new();
        let mut renamed = Vec::new();
        for file in files {
            (0..self.files[file].diff.hunks.len()).for_each(|hunk| self.see(file, hunk));
            let (done, total) = self.files[file].progress();
            if done == if reviewed { total } else { 0 } {
                continue;
            }
            let diff = &self.files[file].diff;
            let path = diff.path.clone();
            if let Some(old) = &diff.old_path {
                if reviewed {
                    renamed.push(old.clone());
                } else {
                    paths.push(old.clone());
                }
            }
            let from_buffer = reviewed
                && self
                    .texts
                    .get(&path)
                    .is_some_and(|text| !text.matches_work(&self.repo.root.join(&path)));
            if from_buffer {
                versions.push((path, self.all_version(file)));
            } else {
                paths.push(path);
            }
            self.files[file].mark(reviewed);
        }
        if paths.is_empty() && versions.is_empty() && renamed.is_empty() {
            return;
        }
        self.update_progress();
        self.error = None;
        let job = StageJob {
            repo: self.repo.clone(),
            paths,
            versions,
            renamed,
            reviewed,
            ctx: ctx.clone(),
        };
        match &self.staging {
            None => self.staging = Some(job.start()),
            Some(_) => self.pending.push(job),
        }
    }

    fn all_version(&self, file: usize) -> Option<Vec<u8>> {
        let diff = &self.files[file].diff;
        self.texts[&diff.path].version(&diff.hunks, |_, _| true)
    }

    fn handle_keys(
        &mut self,
        ctx: &egui::Context,
        file: usize,
        buffer: Option<&mut Buffer>,
    ) -> Option<Outcome> {
        if ctx.memory(|m| m.focused().is_some()) {
            return None;
        }
        if let Some(buffer) = buffer {
            if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z)) {
                buffer.redo();
            } else if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Z)) {
                buffer.undo();
            }
        }
        let pressed = |key| ctx.input(|i| i.modifiers.is_none() && i.key_pressed(key));
        if pressed(Key::N) {
            return self.next(file).map(Outcome::Show);
        }
        let count = self.files[file].diff.hunks.len();
        if count == 0 {
            let reviewed = !self.files[file].file_staged;
            if pressed(Key::Space) {
                self.stage_file(file, reviewed);
                return if reviewed {
                    self.next(file).map(Outcome::Show)
                } else {
                    None
                };
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
            return self.toggle_reviewed(file, current).map(Outcome::Show);
        }
        if pressed(Key::O) {
            let diff = &self.files[file].diff;
            return Some(Outcome::Open {
                path: diff.path.clone(),
                line: place_in_file(&diff.hunks[current], None),
            });
        }
        None
    }

    pub fn tree_ui(
        &mut self,
        ui: &mut egui::Ui,
        collapsed: &mut BTreeSet<String>,
        selected: Option<&str>,
    ) -> Option<String> {
        if self.files.is_empty() {
            ui.weak("No changes");
            return None;
        }
        let dark_mode = ui.visuals().dark_mode;
        let mark = Cell::new(None);
        let rolled_back = Cell::new(None);
        let Self {
            tree,
            files,
            by_path,
            progress,
            unseen,
            ..
        } = self;
        let progress_of = |path: &str| progress.get(path).copied().unwrap_or_default();
        let mut menu = |ui: &mut egui::Ui, path: &str| {
            let (done, total) = progress_of(path);
            if done < total && ui.button("Mark as reviewed").clicked() {
                mark.set(Some((path.to_string(), true)));
            }
            if done > 0 && ui.button("Mark as not reviewed").clicked() {
                mark.set(Some((path.to_string(), false)));
            }
            ui.separator();
            let label = match by_path.contains_key(path) {
                true => "Roll back",
                false => "Roll back changes",
            };
            if ui.button(label).clicked() {
                rolled_back.set(Some(path.to_string()));
            }
        };
        let mut reviewed_mark = |ui: &egui::Ui, rect: egui::Rect, path: &str| {
            let (done, total) = progress_of(path);
            let state = if done == total {
                Some(true)
            } else if done == 0 {
                Some(false)
            } else {
                None
            };
            let label = format!("Reviewed: {path}");
            if reviewed_mark_ui(
                ui,
                rect,
                2.0,
                state,
                &label,
                egui::Id::unique(("reviewed", path)),
            ) {
                mark.set(Some((path.to_string(), done < total)));
            }
        };
        let look = |path: &str| {
            let (done, total) = progress_of(path);
            let color = by_path
                .get(path)
                .map(|&index| status_color(files[index].diff.status, dark_mode))
                .map(|color| fade(color, done == total));
            FileLook {
                color,
                detail: (total > 0).then(|| format!("{done}/{total}")),
                dot: unseen.contains(path).then_some(NEW_COLOR),
            }
        };
        let clicked = tree.ui(
            ui,
            collapsed,
            selected,
            look,
            Some(&mut menu),
            Some(&mut reviewed_mark),
        );
        if let Some(path) = rolled_back.into_inner() {
            self.roll_back(&path);
        }
        if let Some((path, reviewed)) = mark.into_inner() {
            self.set_reviewed(&path, reviewed, ui.ctx());
        }
        clicked
    }

    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        path: Option<&str>,
        active: bool,
        agents: &Agents,
        open_files: &mut OpenFiles,
    ) -> Option<Outcome> {
        let Some(path) = path else {
            ui.centered_and_justified(|ui| ui.weak("Choose a file under Changes to review it"));
            return None;
        };
        let Some(&index) = self.by_path.get(path) else {
            ui.centered_and_justified(|ui| ui.weak(format!("No changes in {path}")));
            return None;
        };
        let diff = &self.files[index].diff;
        let mut file_text = if diff.binary {
            Err("Binary file")
        } else {
            open_files.text(path, diff.status == FileStatus::Deleted)
        };
        let mut next = None;
        let mut outcome = if active {
            self.handle_keys(
                ui.ctx(),
                index,
                file_text.as_mut().ok().map(|(buffer, _)| &mut **buffer),
            )
        } else {
            None
        };
        if let Ok((buffer, _)) = &file_text {
            if !self.texts.contains_key(path) {
                let text = FileText::load(&self.repo, &self.files[index].diff);
                self.texts.insert(path.to_string(), text);
            }
            let editing = self
                .views
                .get(path)
                .is_some_and(|view| view.editor.is_active());
            self.follow_edits(index, buffer, editing);
        }
        let (done, total) = self.progress_of(path);
        let Self {
            files,
            views,
            texts,
            error,
            comments,
            ..
        } = self;
        let file = &files[index];
        let view = views.entry(path.to_string()).or_default();

        let mut mark_file = None;
        let unsent = comments.iter().filter(|comment| !comment.sent).count();
        ui.horizontal(|ui| {
            let name = file.diff.display_name();
            let (dir, file_name) = name
                .rsplit_once('/')
                .map_or(("", name.as_str()), |(dir, file_name)| (dir, file_name));
            ui.spacing_mut().item_spacing.x = 0.0;
            if !dir.is_empty() {
                ui.weak(format!("{dir}/"));
            }
            ui.label(RichText::new(file_name).strong());
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.add_space(12.0);
            ui.weak(format!("{done}/{total} reviewed"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let label = if unsent == 1 {
                    "Send 1 comment".to_string()
                } else {
                    format!("Send {unsent} comments")
                };
                if unsent > 0
                    && let Some(agent) = send_button(ui, &label, true, true, agents)
                {
                    let text = comments::feedback(comments.iter().filter(|comment| !comment.sent));
                    comments.iter_mut().for_each(|comment| comment.sent = true);
                    outcome = Some(Outcome::Send { agent, text });
                }
                if done < total && ui.add(theme::quiet("Mark file reviewed")).clicked() {
                    mark_file = Some(true);
                }
                if done > 0
                    && done == total
                    && ui.add(theme::quiet("Mark file not reviewed")).clicked()
                {
                    mark_file = Some(false);
                }
            });
        });
        if let Some(error) = error {
            ui.colored_label(ui.visuals().error_fg_color, error.as_str());
        }
        let mut shown = None;
        match &mut file_text {
            Ok(_) if file.diff.hunks.is_empty() => {
                ui.add_space(4.0);
                ui.weak("No changes left");
            }
            Ok((buffer, highlighter)) => {
                let text = &texts[path];
                view.editor.find_ui(ui, buffer);
                ui.add_space(4.0);
                let draft = if view.selection.is_some() {
                    egui::Panel::bottom(("draft", path))
                        .show(ui, |ui| {
                            draft_ui(ui, egui::Id::unique(("draft", path)), view, agents)
                        })
                        .inner
                } else {
                    None
                };
                let file_comments: Vec<(usize, &Comment)> = comments
                    .iter()
                    .enumerate()
                    .filter(|(_, comment)| comment.path == path)
                    .collect();
                let revision = buffer.revision();
                let (clicked, lines) = egui::ScrollArea::both()
                    .id_salt(path)
                    .auto_shrink(false)
                    .show(ui, |ui| {
                        rows_ui(
                            ui,
                            path,
                            file,
                            text,
                            view,
                            &file_comments,
                            buffer,
                            highlighter.as_deref_mut(),
                        )
                    })
                    .inner;
                if buffer.revision() != revision {
                    ui.ctx().request_repaint();
                }
                view.scroll_to_current = false;
                let mut revert = None;
                match draft {
                    Some(Draft::Comment) if !lines.is_empty() => {
                        comments.push(Comment::new(
                            path,
                            lines,
                            &file_lines(&text.work_lines, &file.diff),
                            &view.draft,
                        ));
                        view.draft.clear();
                        view.selection = None;
                    }
                    Some(Draft::Ask(agent)) if !lines.is_empty() => {
                        outcome = Some(Outcome::Send {
                            agent,
                            text: comments::quote(path, &lines, &view.draft),
                        });
                        view.draft.clear();
                        view.selection = None;
                    }
                    Some(Draft::Revert) => {
                        let hunk_lines =
                            file.diff.hunks.iter().enumerate().flat_map(|(h, hunk)| {
                                hunk.lines
                                    .iter()
                                    .enumerate()
                                    .map(move |(i, line)| ((h, i), line))
                            });
                        revert = Some(
                            hunk_lines
                                .filter(|(_, line)| lines.contains(line))
                                .map(|(at, _)| at)
                                .collect::<HashSet<_>>(),
                        );
                        view.selection = None;
                    }
                    Some(Draft::Cancel) => view.selection = None,
                    _ => {}
                }
                shown = Some((clicked, revert, buffer));
            }
            Err(message) => {
                ui.add_space(4.0);
                ui.weak(*message);
            }
        }
        if let Some((clicked, revert, buffer)) = shown {
            if let Some(revert) = revert {
                self.revert(index, buffer, |h, i| revert.contains(&(h, i)));
            }
            match clicked {
                Some(Clicked::Row(hunk)) => self.make_current(index, hunk),
                Some(Clicked::Hunk(hunk)) => next = self.toggle_reviewed(index, hunk),
                Some(Clicked::Line(hunk, line)) => self.toggle_line(index, hunk, line),
                Some(Clicked::DeleteComment(comment)) => {
                    self.comments.remove(comment);
                    self.view(index).selection = None;
                }
                Some(Clicked::Revert(hunk)) => self.revert(index, buffer, |h, _| h == hunk),
                Some(Clicked::Open(line)) => {
                    outcome = Some(Outcome::Open {
                        path: path.to_string(),
                        line,
                    })
                }
                None => {}
            }
        }
        if let Some(reviewed) = mark_file {
            self.set_reviewed(path, reviewed, ui.ctx());
            if reviewed {
                next = self.next(index);
            }
        }
        next.map(Outcome::Show).or(outcome)
    }
}

pub fn status_color(status: FileStatus, dark_mode: bool) -> Color32 {
    let palette = theme::palette(dark_mode);
    match status {
        FileStatus::Added | FileStatus::Renamed => palette.added,
        FileStatus::Deleted => palette.removed,
        FileStatus::Modified => palette.modified,
    }
}

#[cfg(test)]
mod tests;
