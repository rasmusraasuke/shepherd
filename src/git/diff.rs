use std::fs;
use std::ops::Range;
use std::path::Path;

pub(super) fn added_file_diff(path: &str, file: &Path) -> String {
    let Ok(bytes) = fs::read(file) else {
        return String::new();
    };
    let mut out = format!("diff --git a/{path} b/{path}\nnew file mode 100644\n");
    if bytes.contains(&0) {
        out.push_str(&format!("Binary files /dev/null and b/{path} differ\n"));
        return out;
    }
    let text = String::from_utf8_lossy(&bytes);
    if text.is_empty() {
        return out;
    }
    let lines: Vec<&str> = text.lines().collect();
    out.push_str(&format!(
        "--- /dev/null\n+++ b/{path}\n@@ -0,0 +1,{} @@\n",
        lines.len()
    ));
    for line in lines {
        out.push('+');
        out.push_str(line);
        out.push('\n');
    }
    out
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FileStatus {
    Modified,
    Added,
    Deleted,
    Renamed,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum LineKind {
    Context,
    Added,
    Removed,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DiffLine {
    pub kind: LineKind,
    pub old_no: Option<u32>,
    pub new_no: Option<u32>,
    pub text: String,
}

impl DiffLine {
    pub fn number(&self) -> u32 {
        self.new_no.or(self.old_no).unwrap_or(0)
    }
}

#[derive(Debug, PartialEq)]
pub struct Hunk {
    pub old_start: u32,
    pub new_start: u32,
    pub lines: Vec<DiffLine>,
}

impl Hunk {
    pub fn ranges(&self) -> (Range<u32>, Range<u32>) {
        let range = |start: u32, count: usize| {
            let count = count as u32;
            if count == 0 {
                start + 1..start + 1
            } else {
                start..start + count
            }
        };
        let old = self
            .lines
            .iter()
            .filter(|line| line.old_no.is_some())
            .count();
        let new = self
            .lines
            .iter()
            .filter(|line| line.new_no.is_some())
            .count();
        (range(self.old_start, old), range(self.new_start, new))
    }
}

#[derive(Debug)]
pub struct FileDiff {
    pub path: String,
    pub old_path: Option<String>,
    pub status: FileStatus,
    pub binary: bool,
    pub hunks: Vec<Hunk>,
}

impl FileDiff {
    pub fn display_name(&self) -> String {
        match &self.old_path {
            Some(old) => format!("{old} -> {}", self.path),
            None => self.path.clone(),
        }
    }
}

pub fn parse_diff(raw: &str) -> Vec<FileDiff> {
    let mut files: Vec<FileDiff> = Vec::new();
    let mut in_hunk = false;
    let (mut old_no, mut new_no) = (0u32, 0u32);

    for line in raw.lines() {
        if in_hunk {
            let file = files.last_mut().unwrap();
            let hunk = file.hunks.last_mut().unwrap();
            let (kind, text) = match line.as_bytes().first() {
                Some(b' ') => (LineKind::Context, &line[1..]),
                Some(b'+') => (LineKind::Added, &line[1..]),
                Some(b'-') => (LineKind::Removed, &line[1..]),
                Some(b'\\') => continue,
                None => (LineKind::Context, ""),
                _ => {
                    in_hunk = false;
                    (LineKind::Context, "")
                }
            };
            if in_hunk {
                let (o, n) = match kind {
                    LineKind::Context => (Some(old_no), Some(new_no)),
                    LineKind::Added => (None, Some(new_no)),
                    LineKind::Removed => (Some(old_no), None),
                };
                if o.is_some() {
                    old_no += 1
                }
                if n.is_some() {
                    new_no += 1
                }
                hunk.lines.push(DiffLine {
                    kind,
                    old_no: o,
                    new_no: n,
                    text: text.to_string(),
                });
                continue;
            }
        }

        if let Some(rest) = line.strip_prefix("diff --git ") {
            let path = rest
                .split_once(" b/")
                .map(|(_, b)| b)
                .unwrap_or(rest)
                .to_string();
            files.push(FileDiff {
                path,
                old_path: None,
                status: FileStatus::Modified,
                binary: false,
                hunks: Vec::new(),
            });
            continue;
        }
        let Some(file) = files.last_mut() else {
            continue;
        };

        if line.starts_with("new file mode") {
            file.status = FileStatus::Added;
        } else if line.starts_with("deleted file mode") {
            file.status = FileStatus::Deleted;
        } else if let Some(from) = line.strip_prefix("rename from ") {
            file.status = FileStatus::Renamed;
            file.old_path = Some(from.to_string());
        } else if let Some(to) = line.strip_prefix("rename to ") {
            file.path = to.to_string();
        } else if let Some(to) = line.strip_prefix("+++ ") {
            if let Some(path) = to.strip_prefix("b/") {
                file.path = path.to_string();
            }
        } else if let Some(from) = line.strip_prefix("--- ") {
            if file.status == FileStatus::Deleted
                && let Some(path) = from.strip_prefix("a/")
            {
                file.path = path.to_string();
            }
        } else if line.starts_with("Binary files ") {
            file.binary = true;
        } else if line.starts_with("@@") {
            (old_no, new_no) = parse_hunk_start(line);
            file.hunks.push(Hunk {
                old_start: old_no,
                new_start: new_no,
                lines: Vec::new(),
            });
            in_hunk = true;
        }
    }
    files
}

fn parse_hunk_start(header: &str) -> (u32, u32) {
    let mut parts = header.split_whitespace().skip(1);
    let mut start = |prefix: char| -> u32 {
        parts
            .next()
            .and_then(|p| p.strip_prefix(prefix))
            .and_then(|p| p.split(',').next())
            .and_then(|n| n.parse().ok())
            .unwrap_or(0)
    };
    let old = start('-');
    let new = start('+');
    (old, new)
}
