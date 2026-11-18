use std::collections::HashSet;
use std::ops::Range;
use std::path::Path;

use eframe::egui::Color32;

use super::word_diff::Ranges;
use crate::editor::file_view;
use crate::editor::highlight::{Highlighter, Spans};
use crate::git::{DiffLine, FileDiff, FileStatus, Hunk, LineKind, Repo};

use super::{line_diff, word_diff};

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Gap {
    pub(super) lines: Range<usize>,
    pub(super) old_offset: isize,
}

pub(super) struct FileText {
    pub(super) head: Option<String>,
    pub(super) old: Side,
    pub(super) index: Vec<String>,
    pub(super) staged_removals: HashSet<u32>,
    pub(super) missing: bool,
    pub(super) work: String,
    pub(super) work_lines: Vec<String>,
    pub(super) revision: Option<u64>,
    pub(super) changed: Vec<Vec<Ranges>>,
    pub(super) longest: usize,
}

pub(super) struct Side {
    pub(super) lines: Vec<String>,
    pub(super) spans: Vec<Spans>,
}

pub(super) fn row_colors(kind: LineKind, dark_mode: bool) -> Option<(Color32, Color32)> {
    let rgb = |[r, g, b]: [u8; 3]| Color32::from_rgb(r, g, b);
    let colors = match (kind, dark_mode) {
        (LineKind::Context, _) => return None,
        (LineKind::Added, true) => ([35, 46, 38], [48, 74, 54]),
        (LineKind::Removed, true) => ([52, 35, 37], [84, 48, 50]),
        (LineKind::Added, false) => ([232, 245, 234], [196, 230, 203]),
        (LineKind::Removed, false) => ([252, 235, 234], [244, 200, 197]),
    };
    Some((rgb(colors.0), rgb(colors.1)))
}

pub(super) fn read_lines(root: &Path, diff: &FileDiff) -> Vec<String> {
    match diff.status {
        FileStatus::Deleted => Vec::new(),
        _ => file_view::read(&root.join(&diff.path))
            .map(|text| text.lines().map(str::to_string).collect())
            .unwrap_or_default(),
    }
}

pub(super) fn unchanged_line(content: &[String], gap: &Gap, number: usize) -> DiffLine {
    let text = content.get(number - 1).cloned().unwrap_or_default();
    DiffLine {
        kind: LineKind::Context,
        old_no: u32::try_from(number as isize + gap.old_offset).ok(),
        new_no: Some(number as u32),
        text,
    }
}

pub(super) fn file_lines(content: &[String], diff: &FileDiff) -> Vec<DiffLine> {
    let mut lines = Vec::new();
    for (index, gap) in gaps(&diff.hunks, content.len()).iter().enumerate() {
        lines.extend(
            gap.lines
                .clone()
                .map(|number| unchanged_line(content, gap, number)),
        );
        lines.extend(
            diff.hunks
                .get(index)
                .into_iter()
                .flat_map(|hunk| hunk.lines.iter().cloned()),
        );
    }
    lines
}

impl FileText {
    pub(super) fn load(repo: &Repo, diff: &FileDiff) -> Self {
        let head = repo.committed(&diff.path);
        let old: Vec<String> = head
            .as_deref()
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect();
        let path = repo.root.join(&diff.path);
        let mut text = Self {
            head,
            old: Side {
                spans: Highlighter::spans_of(&path, &old),
                lines: old,
            },
            index: Vec::new(),
            staged_removals: HashSet::new(),
            missing: !path.exists(),
            work: String::new(),
            work_lines: Vec::new(),
            revision: None,
            changed: Vec::new(),
            longest: 0,
        };
        text.set_index(&repo.staged(&diff.path).unwrap_or_default());
        text
    }

    pub(super) fn set_index(&mut self, staged: &str) {
        self.index = staged.lines().map(str::to_string).collect();
        self.staged_removals = line_diff::diff(&self.old.lines, &self.index)
            .iter()
            .flat_map(|hunk| &hunk.lines)
            .filter_map(|line| line.old_no)
            .collect();
        self.revision = None;
    }
}

impl Side {
    pub(super) fn line(&self, number: Option<u32>) -> (&str, &[(Range<usize>, &'static str)]) {
        let index = number
            .and_then(|number| (number as usize).checked_sub(1))
            .unwrap_or(usize::MAX);
        (
            self.lines.get(index).map_or("", String::as_str),
            self.spans.get(index).map_or(&[], Vec::as_slice),
        )
    }
}

pub(super) fn changed_in_lines(hunk: &Hunk, old: &[String], new: &[String]) -> Vec<Ranges> {
    let line = |lines: &[String], number: Option<u32>| {
        number
            .and_then(|number| lines.get(number as usize - 1))
            .map_or(String::new(), String::clone)
    };
    let lines = &hunk.lines;
    let mut changed = vec![Vec::new(); lines.len()];
    let mut at = 0;
    while at < lines.len() {
        let removed = lines[at..]
            .iter()
            .take_while(|line| line.kind == LineKind::Removed)
            .count();
        let added = lines[at + removed..]
            .iter()
            .take_while(|line| line.kind == LineKind::Added)
            .count();
        for i in 0..removed.min(added) {
            let (r, a) = (at + i, at + removed + i);
            if let Some((in_old, in_new)) =
                word_diff::changed_ranges(&line(old, lines[r].old_no), &line(new, lines[a].new_no))
            {
                (changed[r], changed[a]) = (in_old, in_new);
            }
        }
        at += (removed + added).max(1);
    }
    changed
}

pub(super) fn gaps(hunks: &[Hunk], line_count: usize) -> Vec<Gap> {
    if hunks.is_empty() {
        return Vec::new();
    }
    let mut gaps = Vec::new();
    let (mut old_end, mut new_end) = (1, 1);
    for hunk in hunks {
        let (old, new) = hunk.ranges();
        let (old, new) = (
            old.start as usize..old.end as usize,
            new.start as usize..new.end as usize,
        );
        gaps.push(Gap {
            lines: new_end..new.start.max(new_end),
            old_offset: old.start as isize - new.start as isize,
        });
        (old_end, new_end) = (old.end, new.end);
    }
    gaps.push(Gap {
        lines: new_end..(line_count + 1).max(new_end),
        old_offset: old_end as isize - new_end as isize,
    });
    gaps
}
