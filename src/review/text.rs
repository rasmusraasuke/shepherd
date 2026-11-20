use std::collections::HashSet;
use std::ops::Range;
use std::path::Path;

use eframe::egui::Color32;

use super::word_diff::Ranges;
use crate::editor::file_view;
use crate::editor::highlight::{Highlighter, Spans};
use crate::git::{CommitRange, Conflict, DiffLine, FileDiff, FileStatus, Hunk, LineKind, Repo};

use super::{line_diff, word_diff};

#[derive(Clone, Debug, PartialEq)]
pub(super) struct ConflictBlock {
    pub(super) block: Range<usize>,
    pub(super) ours: Range<usize>,
    pub(super) theirs: Range<usize>,
    pub(super) ours_label: String,
    pub(super) theirs_label: String,
}

fn marker(line: &str, sign: char) -> Option<String> {
    let count = line.chars().take_while(|letter| *letter == sign).count();
    (count == 7).then(|| line[7..].trim().to_string())
}

pub(super) fn conflicts(lines: &[String]) -> Vec<ConflictBlock> {
    let mut blocks = Vec::new();
    let mut at = 0;
    while at < lines.len() {
        let Some(ours_label) = marker(&lines[at], '<') else {
            at += 1;
            continue;
        };
        let (mut base, mut separator, mut end) = (None, None, None);
        let mut scan = at + 1;
        while scan < lines.len() {
            let line = &lines[scan];
            if base.is_none() && separator.is_none() && marker(line, '|').is_some() {
                base = Some(scan);
            } else if separator.is_none() && line.trim_end() == "=======" {
                separator = Some(scan);
            } else if separator.is_some() && marker(line, '>').is_some() {
                end = Some(scan);
                break;
            }
            scan += 1;
        }
        let (Some(separator), Some(end)) = (separator, end) else {
            at += 1;
            continue;
        };
        let theirs_label = marker(&lines[end], '>').unwrap_or_default();
        blocks.push(ConflictBlock {
            block: at..end + 1,
            ours: at + 1..base.unwrap_or(separator),
            theirs: separator + 1..end,
            ours_label,
            theirs_label,
        });
        at = end + 1;
    }
    blocks
}

fn new_start(hunk: &Hunk) -> usize {
    if hunk.new_count == 0 {
        hunk.new_start as usize
    } else {
        hunk.new_start as usize - 1
    }
}

fn overlaps(hunk: &Hunk, block: &ConflictBlock) -> bool {
    let start = new_start(hunk);
    match hunk.new_count {
        0 => (block.block.start..=block.block.end).contains(&start),
        count => start < block.block.end && block.block.start < start + count,
    }
}

pub(super) fn hunks_with_conflicts(
    old: &[String],
    work: &[String],
    blocks: &[ConflictBlock],
) -> Vec<Hunk> {
    if blocks.is_empty() {
        return line_diff::diff(old, work);
    }
    enum Piece {
        Change(Hunk),
        Conflict(usize),
    }
    let mut pieces: Vec<Piece> = line_diff::diff(old, work)
        .into_iter()
        .filter(|hunk| !blocks.iter().any(|block| overlaps(hunk, block)))
        .map(Piece::Change)
        .collect();
    pieces.extend((0..blocks.len()).map(Piece::Conflict));
    pieces.sort_by_key(|piece| match piece {
        Piece::Change(hunk) => new_start(hunk),
        Piece::Conflict(index) => blocks[*index].block.start,
    });
    let (mut old_pos, mut new_pos) = (0usize, 0usize);
    let mut hunks = Vec::new();
    for piece in pieces {
        match piece {
            Piece::Change(hunk) => {
                let start = new_start(&hunk);
                old_pos += start.saturating_sub(new_pos);
                old_pos += hunk.old_count;
                new_pos = start + hunk.new_count;
                hunks.push(hunk);
            }
            Piece::Conflict(index) => {
                let block = &blocks[index];
                old_pos += block.block.start.saturating_sub(new_pos);
                let count = if old.is_empty() { 0 } else { block.ours.len() };
                let lines = block
                    .block
                    .clone()
                    .map(|at| DiffLine {
                        kind: LineKind::Added,
                        old_no: None,
                        new_no: Some(at as u32 + 1),
                        text: work[at].clone(),
                    })
                    .collect();
                hunks.push(Hunk {
                    old_start: (old_pos + usize::from(count > 0)) as u32,
                    new_start: block.block.start as u32 + 1,
                    old_count: count,
                    new_count: block.block.len(),
                    lines,
                    conflict: Some(Conflict {
                        ours: block.ours_label.clone(),
                        theirs: block.theirs_label.clone(),
                    }),
                });
                old_pos += count;
                new_pos = block.block.end;
            }
        }
    }
    hunks
}

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
    pub(super) conflicts: Vec<ConflictBlock>,
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
        let base_path = diff.old_path.as_deref().unwrap_or(&diff.path);
        let head = repo.committed(base_path);
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
                spans: Highlighter::spans_of(&repo.root.join(base_path), &old),
                lines: old,
            },
            index: Vec::new(),
            staged_removals: HashSet::new(),
            missing: !path.exists(),
            work: String::new(),
            work_lines: Vec::new(),
            revision: None,
            changed: Vec::new(),
            conflicts: Vec::new(),
            longest: 0,
        };
        let index = repo
            .staged(&diff.path)
            .or_else(|| repo.staged(base_path))
            .unwrap_or_default();
        text.set_index(&index);
        text
    }

    pub(super) fn load_commits(repo: &Repo, diff: &FileDiff, range: &CommitRange) -> Self {
        let base_path = diff.old_path.as_deref().unwrap_or(&diff.path);
        let head = repo.content_at(&range.base, base_path);
        let old: Vec<String> = head
            .as_deref()
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect();
        let work = repo.content_at(&range.head, &diff.path).unwrap_or_default();
        let work_lines: Vec<String> = work.lines().map(str::to_string).collect();
        let changed = diff
            .hunks
            .iter()
            .map(|hunk| changed_in_lines(hunk, &old, &work_lines))
            .collect();
        let longest = old
            .iter()
            .chain(&work_lines)
            .map(|line| line.chars().count())
            .max()
            .unwrap_or(0);
        let mut text = Self {
            head,
            old: Side {
                spans: Highlighter::spans_of(&repo.root.join(base_path), &old),
                lines: old,
            },
            index: work_lines.clone(),
            staged_removals: HashSet::new(),
            missing: false,
            work,
            work_lines,
            revision: None,
            changed,
            conflicts: Vec::new(),
            longest,
        };
        let index = text.index.join("\n");
        text.set_index(&index);
        text
    }

    pub(super) fn matches_work(&self, path: &Path) -> bool {
        match file_view::read(path) {
            Ok(work) => work == self.work,
            Err(_) => self.missing,
        }
    }

    pub(super) fn version(
        &self,
        hunks: &[Hunk],
        chosen: impl Fn(usize, usize) -> bool,
    ) -> Option<Vec<u8>> {
        let status = if self.head.is_none() {
            FileStatus::Added
        } else if self.missing && self.work.is_empty() {
            FileStatus::Deleted
        } else {
            FileStatus::Modified
        };
        let head = self.head.as_deref().unwrap_or_default().as_bytes();
        crate::git::version(status, head, self.work.as_bytes(), hunks, chosen)
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
