use super::{Changes, DiffLine, FileStatus, Hunk, LineKind, parse_diff};
use std::collections::{HashMap, HashSet};

pub fn version(
    status: FileStatus,
    head: &[u8],
    work: &[u8],
    hunks: &[Hunk],
    chosen: impl Fn(usize, usize) -> bool,
) -> Option<Vec<u8>> {
    let changed = |kind: LineKind| {
        hunks.iter().enumerate().flat_map(move |(h, hunk)| {
            hunk.lines
                .iter()
                .enumerate()
                .filter(move |(_, line)| line.kind == kind)
                .map(move |(i, _)| (h, i))
        })
    };
    let gone = match status {
        FileStatus::Added => !changed(LineKind::Added).any(|(h, i)| chosen(h, i)),
        FileStatus::Deleted => changed(LineKind::Removed).all(|(h, i)| chosen(h, i)),
        _ => false,
    };
    (!gone).then(|| compose(head, work, hunks, chosen))
}

pub(super) fn compose(
    head: &[u8],
    work: &[u8],
    hunks: &[Hunk],
    chosen: impl Fn(usize, usize) -> bool,
) -> Vec<u8> {
    fn push(out: &mut Vec<u8>, line: &[u8]) {
        if out.last().is_some_and(|&last| last != b'\n') {
            out.push(b'\n');
        }
        out.extend_from_slice(line);
    }
    fn split(text: &[u8]) -> Vec<&[u8]> {
        text.split_inclusive(|&b| b == b'\n').collect()
    }
    let (head, work) = (split(head), split(work));
    let line = |lines: &[&[u8]], number: Option<u32>| -> Vec<u8> {
        number
            .and_then(|n| lines.get((n as usize).checked_sub(1)?))
            .map_or(Vec::new(), |line| line.to_vec())
    };
    let mut out = Vec::new();
    let mut next = 1;
    for (h, hunk) in hunks.iter().enumerate() {
        let start = hunk.ranges().0.start as usize;
        for unchanged in head.get(next - 1..start - 1).unwrap_or_default() {
            push(&mut out, unchanged);
        }
        next = next.max(start);
        for (i, diff_line) in hunk.lines.iter().enumerate() {
            match diff_line.kind {
                LineKind::Context => push(&mut out, &line(&head, diff_line.old_no)),
                LineKind::Removed if !chosen(h, i) => {
                    push(&mut out, &line(&head, diff_line.old_no))
                }
                LineKind::Added if chosen(h, i) => push(&mut out, &line(&work, diff_line.new_no)),
                _ => {}
            }
            if let Some(old_no) = diff_line.old_no {
                next = old_no as usize + 1;
            }
        }
    }
    for unchanged in head.get(next - 1..).unwrap_or_default() {
        push(&mut out, unchanged);
    }
    out
}

#[derive(Default)]
pub struct Staged {
    removed: HashMap<String, HashSet<u32>>,
    unstaged_added: HashMap<String, HashSet<u32>>,
    staged_files: HashSet<String>,
    unstaged_files: HashSet<String>,
    untracked: HashSet<String>,
}

impl Staged {
    pub fn new(changes: &Changes) -> Self {
        let lines = |raw: &str, kind: LineKind| -> HashMap<String, HashSet<u32>> {
            parse_diff(raw)
                .into_iter()
                .map(|file| {
                    let numbers = file
                        .hunks
                        .iter()
                        .flat_map(|hunk| &hunk.lines)
                        .filter(|line| line.kind == kind);
                    let numbers = numbers
                        .filter_map(|line| {
                            if kind == LineKind::Removed {
                                line.old_no
                            } else {
                                line.new_no
                            }
                        })
                        .collect();
                    (file.path, numbers)
                })
                .collect()
        };
        let removed = lines(&changes.staged, LineKind::Removed);
        let unstaged_added = lines(&changes.unstaged, LineKind::Added);
        Self {
            staged_files: removed.keys().cloned().collect(),
            unstaged_files: unstaged_added.keys().cloned().collect(),
            removed,
            unstaged_added,
            untracked: changes.untracked.iter().cloned().collect(),
        }
    }

    pub fn is_staged(&self, path: &str, line: &DiffLine) -> bool {
        let has = |lines: &HashMap<String, HashSet<u32>>, number: Option<u32>| {
            number.is_some_and(|n| lines.get(path).is_some_and(|lines| lines.contains(&n)))
        };
        match line.kind {
            LineKind::Context => true,
            LineKind::Removed => has(&self.removed, line.old_no),
            LineKind::Added => {
                !self.untracked.contains(path) && !has(&self.unstaged_added, line.new_no)
            }
        }
    }

    pub fn is_file_staged(&self, path: &str) -> bool {
        self.staged_files.contains(path)
            && !self.unstaged_files.contains(path)
            && !self.untracked.contains(path)
    }
}
