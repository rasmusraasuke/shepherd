use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Hash of the empty tree, used as the diff base in repos without commits.
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

#[derive(Clone)]
pub struct Repo {
    pub root: PathBuf,
    git_dir: PathBuf,
}

impl Repo {
    pub fn open(path: &Path) -> Result<Self, String> {
        let root = git(path, &["rev-parse", "--show-toplevel"])?;
        let git_dir = git(path, &["rev-parse", "--absolute-git-dir"])?;
        Ok(Self { root: PathBuf::from(root.trim()), git_dir: PathBuf::from(git_dir.trim()) })
    }

    pub fn review_state_path(&self) -> PathBuf {
        self.git_dir.join("editor-review")
    }

    pub fn layout_path(&self) -> PathBuf {
        self.git_dir.join("editor-layout.json")
    }

    /// Paths of all project files relative to the root: tracked ones plus untracked ones that aren't ignored.
    pub fn list_files(&self) -> Result<Vec<String>, String> {
        let out = git(&self.root, &["ls-files", "--cached", "--others", "--exclude-standard", "-z"])?;
        let mut files: Vec<String> = out.split('\0').filter(|p| !p.is_empty()).map(str::to_string).collect();
        files.sort();
        files.dedup();
        Ok(files)
    }

    /// Unified diff of the working tree (staged and unstaged) against HEAD, plus untracked files.
    pub fn raw_diff(&self) -> Result<String, String> {
        let base = if git(&self.root, &["rev-parse", "--verify", "--quiet", "HEAD"]).is_ok() { "HEAD" } else { EMPTY_TREE };
        let mut out = git(&self.root, &["diff", "--no-color", "--no-ext-diff", "-M", base])?;
        let untracked = git(&self.root, &["ls-files", "--others", "--exclude-standard", "-z"])?;
        for path in untracked.split('\0').filter(|p| !p.is_empty()) {
            // --no-index exits with 1 when files differ, which is always the case here
            let output = Command::new("git")
                .args(["-c", "core.quotepath=false", "diff", "--no-color", "--no-ext-diff", "--no-index", "--", "/dev/null", path])
                .current_dir(&self.root)
                .output()
                .map_err(|e| e.to_string())?;
            out.push_str(&String::from_utf8_lossy(&output.stdout));
        }
        Ok(out)
    }
}

fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .args(["-c", "core.quotepath=false"])
        .args(args)
        .current_dir(dir)
        .output()
        .map_err(|e| format!("failed to run git: {e}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FileStatus {
    Modified,
    Added,
    Deleted,
    Renamed,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LineKind {
    Context,
    Added,
    Removed,
}

#[derive(Debug)]
pub struct DiffLine {
    pub kind: LineKind,
    pub old_no: Option<u32>,
    pub new_no: Option<u32>,
    pub text: String,
}

#[derive(Debug)]
pub struct Hunk {
    pub header: String,
    /// The first old and new line numbers, as the header gives them.
    pub old_start: u32,
    pub new_start: u32,
    pub lines: Vec<DiffLine>,
}

impl Hunk {
    /// The old and the new lines the hunk covers, numbered from 1. A hunk without lines on
    /// one side sits right after the line its header gives for that side.
    pub fn ranges(&self) -> (Range<u32>, Range<u32>) {
        let range = |start: u32, count: usize| {
            let count = count as u32;
            if count == 0 { start + 1..start + 1 } else { start..start + count }
        };
        let old = self.lines.iter().filter(|line| line.old_no.is_some()).count();
        let new = self.lines.iter().filter(|line| line.new_no.is_some()).count();
        (range(self.old_start, old), range(self.new_start, new))
    }

    /// Identifies the hunk by its content, so any change to the code makes it unreviewed again.
    /// Line numbers are left out, so edits elsewhere in the file don't reset it.
    pub fn fingerprint(&self, path: &str) -> String {
        let mut hash = Fnv::new();
        hash.write(path.as_bytes());
        for line in &self.lines {
            hash.write(&[line.kind as u8]);
            hash.write(line.text.as_bytes());
            hash.write(b"\n");
        }
        format!("{:016x}", hash.0)
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
            Some(old) => format!("{old} → {}", self.path),
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
                Some(b'\\') => continue, // "\ No newline at end of file"
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
                if o.is_some() { old_no += 1 }
                if n.is_some() { new_no += 1 }
                hunk.lines.push(DiffLine { kind, old_no: o, new_no: n, text: text.to_string() });
                continue;
            }
        }

        if let Some(rest) = line.strip_prefix("diff --git ") {
            let path = rest.split_once(" b/").map(|(_, b)| b).unwrap_or(rest).to_string();
            files.push(FileDiff { path, old_path: None, status: FileStatus::Modified, binary: false, hunks: Vec::new() });
            continue;
        }
        let Some(file) = files.last_mut() else { continue };

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
            // deleted files have "+++ /dev/null", so their path comes from here
            if file.status == FileStatus::Deleted {
                if let Some(path) = from.strip_prefix("a/") {
                    file.path = path.to_string();
                }
            }
        } else if line.starts_with("Binary files ") {
            file.binary = true;
        } else if line.starts_with("@@") {
            (old_no, new_no) = parse_hunk_start(line);
            file.hunks.push(Hunk { header: line.to_string(), old_start: old_no, new_start: new_no, lines: Vec::new() });
            in_hunk = true;
        }
    }
    files
}

/// Parses "@@ -12,5 +12,7 @@ fn context" into the first old and new line numbers.
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

/// FNV-1a, used because std's hasher isn't guaranteed stable across Rust releases
/// and fingerprints are persisted to disk.
struct Fnv(u64);

impl Fnv {
    fn new() -> Self {
        Fnv(0xcbf29ce484222325)
    }

    fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.0 ^= *b as u64;
            self.0 = self.0.wrapping_mul(0x100000001b3);
        }
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::testing::TempDir;

    /// Creates a repository without commits that has the given files in its working tree.
    pub fn temp_repo(name: &str, files: &[(&str, &str)]) -> (Repo, TempDir) {
        let dir = TempDir::new(name);
        for (path, content) in files {
            dir.write(path, content);
        }
        git(dir.path(), &["init", "--quiet"]).unwrap();
        (Repo::open(dir.path()).unwrap(), dir)
    }

    /// Commits everything in the working tree.
    pub fn commit_all(repo: &Repo) {
        git(&repo.root, &["add", "--all"]).unwrap();
        git(&repo.root, &["-c", "user.name=Test", "-c", "user.email=test@example.com", "commit", "--quiet", "-m", "test"]).unwrap();
    }

    #[test]
    fn lists_project_files_without_ignored_ones() {
        let (repo, _dir) = temp_repo("list-files", &[("src/main.rs", ""), ("README.md", ""), (".gitignore", "/target\n"), ("target/debug/editor", "")]);
        assert_eq!(repo.list_files().unwrap(), [".gitignore", "README.md", "src/main.rs"]);
    }

    const DIFF: &str = "\
diff --git a/src/main.rs b/src/main.rs
index 1111111..2222222 100644
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,3 +1,4 @@ fn main() {
 fn main() {
-    println!(\"hi\");
+    println!(\"hello\");
+    println!(\"world\");
 }
diff --git a/new.txt b/new.txt
new file mode 100644
index 0000000..3333333
--- /dev/null
+++ b/new.txt
@@ -0,0 +1 @@
+--- not a header
\\ No newline at end of file
diff --git a/old.txt b/old.txt
deleted file mode 100644
--- a/old.txt
+++ /dev/null
@@ -1 +0,0 @@
-bye
";

    #[test]
    fn parses_files_hunks_and_line_numbers() {
        let files = parse_diff(DIFF);
        assert_eq!(files.len(), 3);

        let main = &files[0];
        assert_eq!(main.path, "src/main.rs");
        assert_eq!(main.status, FileStatus::Modified);
        let lines = &main.hunks[0].lines;
        assert_eq!(lines.len(), 5);
        assert_eq!((lines[1].kind, lines[1].old_no, lines[1].new_no), (LineKind::Removed, Some(2), None));
        assert_eq!((lines[3].kind, lines[3].new_no), (LineKind::Added, Some(3)));
        assert_eq!((lines[4].old_no, lines[4].new_no), (Some(3), Some(4)));

        let new = &files[1];
        assert_eq!((new.path.as_str(), new.status), ("new.txt", FileStatus::Added));
        assert_eq!(new.hunks[0].lines[0].text, "--- not a header");

        let old = &files[2];
        assert_eq!((old.path.as_str(), old.status), ("old.txt", FileStatus::Deleted));
        assert_eq!(old.hunks[0].lines[0].text, "bye");

        assert_eq!(main.hunks[0].ranges(), (1..4, 1..5));
        assert_eq!(new.hunks[0].ranges(), (1..1, 1..2));
        assert_eq!(old.hunks[0].ranges(), (1..2, 1..1));
    }

    #[test]
    fn fingerprint_ignores_line_numbers_but_not_content() {
        let a = &parse_diff(DIFF)[0].hunks[0];
        let shifted = parse_diff(&DIFF.replace("@@ -1,3 +1,4 @@", "@@ -10,3 +10,4 @@"));
        let edited = parse_diff(&DIFF.replace("world", "there"));
        assert_eq!(a.fingerprint("x"), shifted[0].hunks[0].fingerprint("x"));
        assert_ne!(a.fingerprint("x"), edited[0].hunks[0].fingerprint("x"));
    }
}
