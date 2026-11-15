use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write as _;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Hash of the empty tree, used as the diff base in repos without commits.
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

/// A diff in the form the parser expects, whatever the user's git configuration says. Without
/// unchanged lines, so that each run of changed lines is a hunk of its own. Renames are left
/// out, as a rename is staged as a deleted and an added file.
const DIFF: &[&str] = &["diff", "--no-color", "--no-ext-diff", "--no-renames", "--src-prefix=a/", "--dst-prefix=b/", "--unified=0"];

/// What changed in the working tree.
#[derive(Clone, PartialEq, Default, Debug)]
pub struct Changes {
    /// The diff of the working tree against HEAD, with the untracked files.
    pub diff: String,
    /// The diffs of HEAD to the index and of the index to the working tree.
    pub staged: String,
    pub unstaged: String,
    pub untracked: Vec<String>,
}

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

    /// HEAD, or the empty tree in a repository without commits.
    fn base(&self) -> &'static str {
        if git(&self.root, &["rev-parse", "--verify", "--quiet", "HEAD"]).is_ok() { "HEAD" } else { EMPTY_TREE }
    }

    fn untracked(&self, paths: &[&str]) -> Result<Vec<String>, String> {
        let out = git(&self.root, &[&["ls-files", "--others", "--exclude-standard", "-z", "--"], paths].concat())?;
        Ok(out.split('\0').filter(|p| !p.is_empty()).map(str::to_string).collect())
    }

    pub fn changes(&self) -> Result<Changes, String> {
        let base = self.base();
        let untracked = self.untracked(&[])?;
        let mut diff = git(&self.root, &[DIFF, &[base]].concat())?;
        for path in &untracked {
            diff.push_str(&added_file_diff(path, &self.root.join(path)));
        }
        let staged = git(&self.root, &[DIFF, &["--cached", base]].concat())?;
        let unstaged = git(&self.root, DIFF)?;
        Ok(Changes { diff, staged, unstaged, untracked })
    }

    /// The diff of one file of the working tree against HEAD, as it is now.
    fn file_diff(&self, path: &str) -> Result<Option<FileDiff>, String> {
        let mut raw = git(&self.root, &[DIFF, &[self.base(), "--", path]].concat())?;
        if raw.is_empty() && !self.untracked(&[path])?.is_empty() {
            raw = added_file_diff(path, &self.root.join(path));
        }
        Ok(parse_diff(&raw).pop())
    }

    /// Stages the changed lines of a file that are `chosen`, by the index of their hunk and their
    /// index in it, and unstages the others: the index gets the version of HEAD with just those
    /// changes made. Fails when the file changed since `diff` was taken.
    pub fn stage_lines(&self, diff: &FileDiff, chosen: impl Fn(usize, usize) -> bool) -> Result<(), String> {
        let path = diff.path.as_str();
        if self.file_diff(path)?.is_none_or(|now| now.hunks != diff.hunks) {
            return Err(format!("{path} changed in the meantime, try again"));
        }
        let changed = |kind: LineKind| diff.hunks.iter().enumerate().flat_map(move |(h, hunk)| hunk.lines.iter().enumerate().filter(move |(_, line)| line.kind == kind).map(move |(i, _)| (h, i)));
        // a file that ends up as it is in HEAD, but isn't in HEAD, or is gone from the working tree
        let gone = match diff.status {
            FileStatus::Added => !changed(LineKind::Added).any(|(h, i)| chosen(h, i)),
            FileStatus::Deleted => changed(LineKind::Removed).all(|(h, i)| chosen(h, i)),
            _ => false,
        };
        if gone {
            return git(&self.root, &["update-index", "--force-remove", "--", path]).map(drop);
        }
        let head = if diff.status == FileStatus::Added { Vec::new() } else { run(&self.root, &["show", &format!("{}:{path}", self.base())], None)? };
        let content = compose(&head, &self.read_work(path), &diff.hunks, chosen);
        let hash = String::from_utf8_lossy(&run(&self.root, &["hash-object", "-w", "--stdin"], Some(&content))?).trim().to_string();
        git(&self.root, &["update-index", "--add", "--cacheinfo", &format!("{},{hash},{path}", self.mode(path))]).map(drop)
    }

    /// Stages or unstages all of a file, for changes without lines, such as of a binary file.
    pub fn stage_file(&self, path: &str, staged: bool) -> Result<(), String> {
        let args: &[&str] = match (staged, self.base()) {
            (true, _) => &["add", "--all", "--"],
            (false, EMPTY_TREE) => &["rm", "--cached", "--quiet", "--"],
            (false, _) => &["reset", "--quiet", "--"],
        };
        git(&self.root, &[args, &[path]].concat()).map(drop)
    }

    /// The content of a file in the working tree as git sees it: for a link, where it points.
    fn read_work(&self, path: &str) -> Vec<u8> {
        let file = self.root.join(path);
        match fs::read_link(&file) {
            Ok(target) => target.to_string_lossy().into_owned().into_bytes(),
            Err(_) => fs::read(&file).unwrap_or_default(),
        }
    }

    /// The mode git gives the file: as it is in the working tree, or in HEAD when it is gone.
    fn mode(&self, path: &str) -> String {
        match fs::symlink_metadata(self.root.join(path)) {
            Ok(meta) if meta.is_symlink() => "120000".to_string(),
            Ok(meta) if is_executable(&meta) => "100755".to_string(),
            Ok(_) => "100644".to_string(),
            Err(_) => git(&self.root, &["ls-tree", "HEAD", "--", path])
                .ok()
                .and_then(|out| out.split_whitespace().next().map(str::to_string))
                .unwrap_or_else(|| "100644".to_string()),
        }
    }
}

#[cfg(unix)]
fn is_executable(meta: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    meta.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_meta: &fs::Metadata) -> bool {
    false
}

/// The version of a file made from HEAD's version with the `chosen` changes of the diff to the
/// working tree's version, which give the added lines. The lines keep their endings.
fn compose(head: &[u8], work: &[u8], hunks: &[Hunk], chosen: impl Fn(usize, usize) -> bool) -> Vec<u8> {
    fn push(out: &mut Vec<u8>, line: &[u8]) {
        // a line that had none at the end of its file gets one when more follows
        if out.last().is_some_and(|&last| last != b'\n') {
            out.push(b'\n');
        }
        out.extend_from_slice(line);
    }
    fn split(text: &[u8]) -> Vec<&[u8]> {
        text.split_inclusive(|&b| b == b'\n').collect()
    }
    let (head, work) = (split(head), split(work));
    let line = |lines: &[&[u8]], number: Option<u32>| -> Vec<u8> { number.and_then(|n| lines.get((n as usize).checked_sub(1)?)).map_or(Vec::new(), |line| line.to_vec()) };
    let mut out = Vec::new();
    // the first line of HEAD not taken yet, numbered from 1
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
                LineKind::Removed if !chosen(h, i) => push(&mut out, &line(&head, diff_line.old_no)),
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

/// Which of the changes in the working tree are staged.
#[derive(Default)]
pub struct Staged {
    /// For each file: the lines of HEAD whose removal is staged, and the lines of the working
    /// tree whose addition isn't.
    removed: HashMap<String, HashSet<u32>>,
    unstaged_added: HashMap<String, HashSet<u32>>,
    /// The files with staged changes, and with unstaged ones.
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
                    let numbers = file.hunks.iter().flat_map(|hunk| &hunk.lines).filter(|line| line.kind == kind);
                    let numbers = numbers.filter_map(|line| if kind == LineKind::Removed { line.old_no } else { line.new_no }).collect();
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

    /// Whether a changed line of a file in the diff of the working tree to HEAD is staged.
    pub fn is_staged(&self, path: &str, line: &DiffLine) -> bool {
        let has = |lines: &HashMap<String, HashSet<u32>>, number: Option<u32>| number.is_some_and(|n| lines.get(path).is_some_and(|lines| lines.contains(&n)));
        match line.kind {
            LineKind::Context => true,
            LineKind::Removed => has(&self.removed, line.old_no),
            LineKind::Added => !self.untracked.contains(path) && !has(&self.unstaged_added, line.new_no),
        }
    }

    /// Whether all changes of a file are staged, for changes without lines.
    pub fn is_file_staged(&self, path: &str) -> bool {
        self.staged_files.contains(path) && !self.unstaged_files.contains(path) && !self.untracked.contains(path)
    }
}

/// The diff of a new file, as git gives it. Made here rather than by git, as running git for
/// each of many new files takes long.
fn added_file_diff(path: &str, file: &Path) -> String {
    // gone already, or not a file
    let Ok(bytes) = fs::read(file) else { return String::new() };
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
    out.push_str(&format!("--- /dev/null\n+++ b/{path}\n@@ -0,0 +1,{} @@\n", lines.len()));
    for line in lines {
        out.push('+');
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    run(dir, args, None).map(|out| String::from_utf8_lossy(&out).into_owned())
}

/// Runs git with the `input` on its standard input, returning what it writes out.
fn run(dir: &Path, args: &[&str], input: Option<&[u8]>) -> Result<Vec<u8>, String> {
    let mut child = Command::new("git")
        .args(["-c", "core.quotepath=false"])
        .args(args)
        .current_dir(dir)
        .stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to run git: {e}"))?;
    if let (Some(input), Some(mut stdin)) = (input, child.stdin.take()) {
        stdin.write_all(input).map_err(|e| format!("failed to run git: {e}"))?;
    }
    let output = child.wait_with_output().map_err(|e| format!("failed to run git: {e}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(output.stdout)
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

#[derive(Debug, PartialEq)]
pub struct DiffLine {
    pub kind: LineKind,
    pub old_no: Option<u32>,
    pub new_no: Option<u32>,
    pub text: String,
}

#[derive(Debug, PartialEq)]
pub struct Hunk {
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
            file.hunks.push(Hunk { old_start: old_no, new_start: new_no, lines: Vec::new() });
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

    #[test]
    fn diffs_new_files_as_git_does() {
        let (repo, _dir) = temp_repo("added", &[("a.txt", "one\r\ntwo"), ("dir/b.bin", "\0\u{1}"), ("empty", "")]);
        let git_diff = |path: &str| {
            // --no-index exits with 1 when files differ
            let output = Command::new("git").args(["diff", "--no-color", "--no-index", "--", "/dev/null", path]).current_dir(&repo.root).output().unwrap();
            format!("{:?}", parse_diff(&String::from_utf8_lossy(&output.stdout)))
        };
        for path in ["a.txt", "dir/b.bin", "empty"] {
            assert_eq!(format!("{:?}", parse_diff(&added_file_diff(path, &repo.root.join(path)))), git_diff(path), "{path}");
        }
        let files = parse_diff(&repo.changes().unwrap().diff);
        assert_eq!(files.iter().map(|file| (file.path.as_str(), file.status, file.binary)).collect::<Vec<_>>(), [
            ("a.txt", FileStatus::Added, false),
            ("dir/b.bin", FileStatus::Added, true),
            ("empty", FileStatus::Added, false)
        ]);
    }

    /// The diff of the working tree of one file against HEAD.
    fn diff_of(repo: &Repo, path: &str) -> FileDiff {
        repo.file_diff(path).unwrap().unwrap()
    }

    /// The version of a file in the index.
    fn staged(repo: &Repo, path: &str) -> String {
        git(&repo.root, &["show", &format!(":{path}")]).unwrap()
    }

    /// Which changed lines of a file are staged, as their texts with the sign of the change.
    fn staged_lines(repo: &Repo, path: &str) -> Vec<String> {
        let staged = Staged::new(&repo.changes().unwrap());
        let diff = diff_of(repo, path);
        let lines = diff.hunks.iter().flat_map(|hunk| &hunk.lines).filter(|line| line.kind != LineKind::Context);
        let staged = lines.filter(|line| staged.is_staged(path, line));
        staged.map(|line| format!("{}{}", if line.kind == LineKind::Added { "+" } else { "-" }, line.text)).collect()
    }

    #[test]
    fn makes_a_version_from_head_with_the_chosen_changes() {
        let raw = "diff --git a/a b/a\n--- a/a\n+++ b/a\n@@ -2 +2 @@\n-b\n+B\n@@ -4,0 +5 @@\n+e\n";
        let hunks = &parse_diff(raw)[0].hunks;
        let (head, work) = (b"a\r\nb\r\nc\r\nd", b"a\r\nB\r\nc\r\nd\r\ne\r\n");
        let compose = |chosen: &[(usize, usize)]| String::from_utf8(compose(head, work, hunks, |h, i| chosen.contains(&(h, i)))).unwrap();
        assert_eq!(compose(&[]), "a\r\nb\r\nc\r\nd");
        assert_eq!(compose(&[(0, 0)]), "a\r\nc\r\nd");
        assert_eq!(compose(&[(0, 1), (1, 0)]), "a\r\nb\r\nB\r\nc\r\nd\ne\r\n");
        assert_eq!(compose(&[(0, 0), (0, 1), (1, 0)]).replace("d\n", "d\r\n"), String::from_utf8(work.to_vec()).unwrap());
    }

    #[test]
    fn stages_the_chosen_lines_of_a_file() {
        let (repo, dir) = temp_repo("stage-lines", &[("a.txt", "a\nb\nc\nd\n")]);
        commit_all(&repo);
        dir.write("a.txt", "a\nB\nc\nd\ne\n");
        let diff = diff_of(&repo, "a.txt");
        assert!(staged_lines(&repo, "a.txt").is_empty());

        // only the removal of a replaced line
        repo.stage_lines(&diff, |h, i| (h, i) == (0, 0)).unwrap();
        assert_eq!(staged(&repo, "a.txt"), "a\nc\nd\n");
        assert_eq!(staged_lines(&repo, "a.txt"), ["-b"]);

        repo.stage_lines(&diff, |h, _| h == 1).unwrap();
        assert_eq!(staged_lines(&repo, "a.txt"), ["+e"]);
        repo.stage_lines(&diff, |_, _| true).unwrap();
        assert_eq!(staged(&repo, "a.txt"), "a\nB\nc\nd\ne\n");
        assert_eq!(staged_lines(&repo, "a.txt"), ["-b", "+B", "+e"]);

        // a change made after staging is not staged
        dir.write("a.txt", "a\nB\nc\nD\ne\n");
        assert_eq!(staged_lines(&repo, "a.txt"), ["-b", "+B", "+e"]);
        assert!(repo.stage_lines(&diff, |_, _| false).unwrap_err().contains("changed in the meantime"));
    }

    #[test]
    fn stages_new_and_deleted_files_and_takes_them_back() {
        let (repo, dir) = temp_repo("stage-files", &[("old.txt", "one\ntwo\n")]);
        commit_all(&repo);
        fs::remove_file(dir.path().join("old.txt")).unwrap();
        dir.write("new.txt", "x\ny\n");
        let tracked = |path: &str| !git(&repo.root, &["ls-files", "--", path]).unwrap().is_empty();

        let new = diff_of(&repo, "new.txt");
        repo.stage_lines(&new, |_, i| i == 1).unwrap();
        assert_eq!((staged(&repo, "new.txt").as_str(), staged_lines(&repo, "new.txt")), ("y\n", vec!["+y".to_string()]));
        repo.stage_lines(&new, |_, _| false).unwrap();
        assert!(!tracked("new.txt"));

        let old = diff_of(&repo, "old.txt");
        repo.stage_lines(&old, |_, i| i == 0).unwrap();
        assert_eq!(staged(&repo, "old.txt"), "two\n");
        repo.stage_lines(&old, |_, _| true).unwrap();
        assert!(!tracked("old.txt"));
        assert_eq!(staged_lines(&repo, "old.txt"), ["-one", "-two"]);

        // files without lines are staged whole
        dir.write("image.bin", "\0");
        let staged_file = |repo: &Repo| Staged::new(&repo.changes().unwrap()).is_file_staged("image.bin");
        repo.stage_file("image.bin", true).unwrap();
        assert!(staged_file(&repo));
        repo.stage_file("image.bin", false).unwrap();
        assert!(!staged_file(&repo));
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
}
