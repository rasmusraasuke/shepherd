mod diff;
mod history;
mod staging;
pub mod watcher;

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use diff::added_file_diff;
pub use diff::{Conflict, DiffLine, FileDiff, FileStatus, Hunk, LineKind, parse_diff};
pub use history::{Commit, CommitRange};
pub use staging::{Staged, version};

const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

const LEFT: &str = "shepherd: changes left on ";

const LEFT_PULL: &str = "shepherd: changes kept over a pull";

const DIFF: &[&str] = &[
    "diff",
    "--no-color",
    "--no-ext-diff",
    "--find-renames",
    "--src-prefix=a/",
    "--dst-prefix=b/",
    "--unified=0",
];

#[derive(Clone, PartialEq, Default, Debug)]
pub struct Changes {
    pub diff: String,
    pub staged: String,
    pub unstaged: String,
    pub untracked: Vec<String>,
    pub conflicts: Vec<String>,
    pub operation: Option<Operation>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Operation {
    Merge,
    Rebase,
    CherryPick,
    Revert,
    Stash,
}

impl Operation {
    pub fn label(self) -> &'static str {
        match self {
            Operation::Merge => "Merging",
            Operation::Rebase => "Rebasing",
            Operation::CherryPick => "Cherry-picking",
            Operation::Revert => "Reverting",
            Operation::Stash => "Applying a stash",
        }
    }

    pub fn can_abort(self) -> bool {
        !matches!(self, Operation::Stash)
    }

    pub fn can_continue(self) -> bool {
        !matches!(self, Operation::Stash)
    }
}

#[derive(Clone, PartialEq, Default, Debug)]
pub struct Status {
    pub branch: String,
    pub branches: Vec<String>,
    pub upstream: bool,
    pub ahead: usize,
    pub behind: usize,
    pub left: Vec<String>,
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
        Ok(Self {
            root: PathBuf::from(root.trim()),
            git_dir: PathBuf::from(git_dir.trim()),
        })
    }

    pub fn layout_path(&self) -> PathBuf {
        self.git_dir.join("shepherd-layout.json")
    }

    pub fn list_files(&self) -> Result<Vec<String>, String> {
        let out = git(
            &self.root,
            &[
                "ls-files",
                "--cached",
                "--others",
                "--exclude-standard",
                "-z",
            ],
        )?;
        let mut files: Vec<String> = out
            .split('\0')
            .filter(|p| !p.is_empty())
            .map(str::to_string)
            .collect();
        files.sort();
        files.dedup();
        Ok(files)
    }

    pub fn branch(&self) -> String {
        git(&self.root, &["symbolic-ref", "--short", "-q", "HEAD"]).map_or_else(
            |_| "none (detached HEAD)".to_string(),
            |name| name.trim().to_string(),
        )
    }

    pub fn status(&self) -> Status {
        let lines = |args: &[&str]| -> Vec<String> {
            let out = git(&self.root, args).unwrap_or_default();
            out.lines().map(str::to_string).collect()
        };
        let counts = git(
            &self.root,
            &["rev-list", "--left-right", "--count", "HEAD...@{upstream}"],
        );
        let mut count = counts
            .iter()
            .flat_map(|counts| counts.split_whitespace())
            .map(|count| count.parse().unwrap_or(0));
        Status {
            branch: self.branch(),
            branches: lines(&["for-each-ref", "--format=%(refname:short)", "refs/heads"]),
            upstream: counts.is_ok(),
            ahead: count.next().unwrap_or(0),
            behind: count.next().unwrap_or(0),
            left: lines(&["stash", "list", "--format=%gs"])
                .iter()
                .filter_map(|subject| subject.split_once(LEFT))
                .map(|(_, branch)| branch.to_string())
                .collect(),
        }
    }

    pub fn commit(&self, message: &str) -> Result<(), String> {
        git(&self.root, &["commit", "--quiet", "--message", message]).map(drop)
    }

    pub fn push(&self) -> Result<(), String> {
        if self.status().upstream {
            return git(&self.root, &["push", "--quiet"]).map(drop);
        }
        let remotes = git(&self.root, &["remote"])?;
        let remote = remotes
            .lines()
            .find(|remote| *remote == "origin")
            .or(remotes.lines().next())
            .ok_or("There is no remote to push to")?;
        git(
            &self.root,
            &["push", "--quiet", "--set-upstream", remote, "HEAD"],
        )
        .map(drop)
    }

    pub fn sync(&self) -> Result<(), String> {
        git(&self.root, &["fetch", "--quiet", "--all"])?;
        if self.status().behind == 0 {
            return Ok(());
        }
        let incoming = git(&self.root, &["diff", "--name-only", "HEAD...@{upstream}"])?;
        let changed = git(&self.root, &["diff", "--name-only", "HEAD"])?;
        let untracked = self.untracked(&[])?;
        let in_the_way: Vec<&str> = incoming
            .lines()
            .filter(|path| changed.lines().any(|changed| changed == *path))
            .chain(
                untracked
                    .iter()
                    .map(String::as_str)
                    .filter(|path| incoming.lines().any(|incoming| incoming == *path)),
            )
            .collect();
        if !in_the_way.is_empty() {
            return Err(format!(
                "Commit the changes of {} first: the commits to pull change it too",
                in_the_way.join(", ")
            ));
        }
        self.carrying(|| {
            git(
                &self.root,
                &["pull", "--quiet", "--no-rebase", "--ff", "--no-edit"],
            )
            .map(drop)
        })
    }

    pub fn continue_operation(&self) -> Result<(), String> {
        let operation = self
            .current_operation()
            .ok_or("There is nothing to continue")?;
        match operation {
            Operation::Merge => self.editor_git(&["merge", "--continue"]),
            Operation::Rebase => self.editor_git(&["rebase", "--continue"]),
            Operation::CherryPick => self.editor_git(&["cherry-pick", "--continue"]),
            Operation::Revert => self.editor_git(&["revert", "--continue"]),
            Operation::Stash => self.drop_applied_stash(),
        }?;
        self.pop_kept()
    }

    pub fn abort_operation(&self) -> Result<(), String> {
        let operation = self
            .current_operation()
            .ok_or("There is nothing to abort")?;
        let aborted = match operation {
            Operation::Merge => git(&self.root, &["merge", "--abort"]).map(drop),
            Operation::Rebase => git(&self.root, &["rebase", "--abort"]).map(drop),
            Operation::CherryPick => git(&self.root, &["cherry-pick", "--abort"]).map(drop),
            Operation::Revert => git(&self.root, &["revert", "--abort"]).map(drop),
            Operation::Stash => Err("The changes of a stash can't be taken back".to_string()),
        };
        aborted?;
        self.pop_kept()
    }

    fn editor_git(&self, args: &[&str]) -> Result<(), String> {
        git(&self.root, &[&["-c", "core.editor=true"], args].concat()).map(drop)
    }

    fn drop_applied_stash(&self) -> Result<(), String> {
        let stashes = git(&self.root, &["stash", "list", "--format=%gd"])?;
        match stashes.lines().next() {
            Some(stash) => git(&self.root, &["stash", "drop", "--quiet", stash]).map(drop),
            None => Ok(()),
        }
    }

    fn pop_kept(&self) -> Result<(), String> {
        let stashes = git(&self.root, &["stash", "list", "--format=%gd %gs"])?;
        let kept = stashes
            .lines()
            .filter_map(|line| line.split_once(' '))
            .find(|(_, subject)| subject.contains(LEFT_PULL));
        match kept {
            Some((stash, _)) => self.pop(stash),
            None => Ok(()),
        }
    }

    pub fn take_side(&self, path: &str, ours: bool) -> Result<(), String> {
        let (side, stage) = if ours { ("--ours", 2) } else { ("--theirs", 3) };
        if git(&self.root, &["cat-file", "-e", &format!(":{stage}:{path}")]).is_ok() {
            git(&self.root, &["checkout", side, "--", path])?;
            return git(&self.root, &["add", "--all", "--", path]).map(drop);
        }
        git(&self.root, &["rm", "--force", "--quiet", "--", path]).map(drop)
    }

    pub fn create_branch(&self, name: &str) -> Result<(), String> {
        git(&self.root, &["switch", "--quiet", "--create", name]).map(drop)
    }

    pub fn switch(&self, branch: &str, leave: bool) -> Result<(), String> {
        let switch = || git(&self.root, &["switch", "--quiet", branch]).map(drop);
        let stashed = leave && self.stash(&format!("{LEFT}{}", self.branch()))?;
        if let Err(error) = switch() {
            if stashed {
                self.pop("stash@{0}")?;
            }
            return Err(error);
        }
        if !git(&self.root, &["status", "--porcelain"])?.is_empty() {
            return Ok(());
        }
        let stashes = git(&self.root, &["stash", "list", "--format=%gd %gs"])?;
        let left = format!("{LEFT}{branch}");
        let stash = stashes
            .lines()
            .filter_map(|line| line.split_once(' '))
            .find(|(_, subject)| subject.ends_with(&left));
        match stash {
            Some((stash, _)) => self.pop(stash),
            None => Ok(()),
        }
    }

    fn stash(&self, message: &str) -> Result<bool, String> {
        let top = || {
            git(
                &self.root,
                &["rev-parse", "--verify", "--quiet", "refs/stash"],
            )
            .ok()
        };
        let before = top();
        git(
            &self.root,
            &[
                "stash",
                "push",
                "--quiet",
                "--include-untracked",
                "--message",
                message,
            ],
        )?;
        Ok(top() != before)
    }

    fn pop(&self, stash: &str) -> Result<(), String> {
        git(&self.root, &["stash", "pop", "--quiet", "--index", stash])
            .or_else(|_| git(&self.root, &["stash", "pop", "--quiet", stash]))
            .map(drop)
            .map_err(|e| format!("The changes didn't apply and are kept in git stash:\n{e}"))
    }

    fn carrying(&self, action: impl Fn() -> Result<(), String>) -> Result<(), String> {
        let stashed = self.stash(LEFT_PULL)?;
        let done = action();
        if self.current_operation().is_some() {
            let error = done.err().unwrap_or_default();
            return Err(format!(
                "The pull ran into conflicts. Resolve them under Changes.\n{error}"
            ));
        }
        let popped = if stashed {
            self.pop("stash@{0}")
        } else {
            Ok(())
        };
        done.and(popped)
    }

    pub fn conflicts(&self) -> Result<Vec<String>, String> {
        let out = git(
            &self.root,
            &["diff", "--name-only", "--diff-filter=U", "-z"],
        )?;
        Ok(out
            .split('\0')
            .filter(|path| !path.is_empty())
            .map(str::to_string)
            .collect())
    }

    fn current_operation(&self) -> Option<Operation> {
        self.operation(&self.conflicts().unwrap_or_default())
    }

    fn base(&self) -> &'static str {
        if git(&self.root, &["rev-parse", "--verify", "--quiet", "HEAD"]).is_ok() {
            "HEAD"
        } else {
            EMPTY_TREE
        }
    }

    fn untracked(&self, paths: &[&str]) -> Result<Vec<String>, String> {
        let out = git(
            &self.root,
            &[
                &["ls-files", "--others", "--exclude-standard", "-z", "--"],
                paths,
            ]
            .concat(),
        )?;
        Ok(out
            .split('\0')
            .filter(|p| !p.is_empty())
            .map(str::to_string)
            .collect())
    }

    pub fn changes(&self) -> Result<Changes, String> {
        let base = self.base();
        let untracked = self.untracked(&[])?;
        let conflicts = self.conflicts()?;
        let mut diff = git(&self.root, &[DIFF, &[base]].concat())?;
        for path in &untracked {
            diff.push_str(&added_file_diff(path, &self.root.join(path)));
        }
        for path in &conflicts {
            let header = format!("diff --git a/{path} b/{path}");
            if !diff.lines().any(|line| line == header) {
                diff.push_str(&header);
                diff.push('\n');
                if !self.root.join(path).exists() {
                    diff.push_str(&format!(
                        "deleted file mode 100644\n--- a/{path}\n+++ /dev/null\n"
                    ));
                }
            }
        }
        let staged = git(&self.root, &[DIFF, &["--cached", base]].concat())?;
        let unstaged = git(&self.root, DIFF)?;
        let operation = self.operation(&conflicts);
        Ok(Changes {
            diff,
            staged,
            unstaged,
            untracked,
            conflicts,
            operation,
        })
    }

    fn operation(&self, conflicts: &[String]) -> Option<Operation> {
        let exists = |name: &str| self.git_dir.join(name).exists();
        if exists("rebase-merge") || exists("rebase-apply") {
            return Some(Operation::Rebase);
        }
        if exists("MERGE_HEAD") {
            return Some(Operation::Merge);
        }
        if exists("CHERRY_PICK_HEAD") {
            return Some(Operation::CherryPick);
        }
        if exists("REVERT_HEAD") {
            return Some(Operation::Revert);
        }
        (!conflicts.is_empty()).then_some(Operation::Stash)
    }

    fn file_diff(&self, path: &str, old_path: Option<&str>) -> Result<Option<FileDiff>, String> {
        let mut files = vec![self.base(), "--", path];
        files.extend(old_path);
        let mut raw = git(&self.root, &[DIFF, files.as_slice()].concat())?;
        if raw.is_empty() && !self.untracked(&[path])?.is_empty() {
            raw = added_file_diff(path, &self.root.join(path));
        }
        Ok(parse_diff(&raw).pop())
    }

    fn check_unchanged(&self, diff: &FileDiff) -> Result<(), String> {
        let path = diff.path.as_str();
        if self
            .file_diff(path, diff.old_path.as_deref())?
            .is_none_or(|now| now.hunks != diff.hunks)
        {
            return Err(format!("{path} changed in the meantime, try again"));
        }
        Ok(())
    }

    pub fn stage_lines(
        &self,
        diff: &FileDiff,
        chosen: impl Fn(usize, usize) -> bool,
    ) -> Result<(), String> {
        self.check_unchanged(diff)?;
        let path = diff.path.as_str();
        let base_path = diff.old_path.as_deref().unwrap_or(path);
        let head = if diff.status == FileStatus::Added {
            Vec::new()
        } else {
            run(
                &self.root,
                &["show", &format!("{}:{base_path}", self.base())],
                None,
            )?
        };
        self.stage_version(
            path,
            version(
                diff.status,
                &head,
                &self.read_work(path),
                &diff.hunks,
                chosen,
            )
            .as_deref(),
        )?;
        if let Some(old) = &diff.old_path {
            self.remove_from_index(old)?;
        }
        Ok(())
    }

    pub fn remove_from_index(&self, path: &str) -> Result<(), String> {
        git(
            &self.root,
            &[
                "rm",
                "--cached",
                "--force",
                "--ignore-unmatch",
                "--quiet",
                "--",
                path,
            ],
        )
        .map(drop)
    }

    pub fn stage_version(&self, path: &str, content: Option<&[u8]>) -> Result<(), String> {
        let Some(content) = content else {
            return git(&self.root, &["update-index", "--force-remove", "--", path]).map(drop);
        };
        let hash = String::from_utf8_lossy(&run(
            &self.root,
            &["hash-object", "-w", "--stdin"],
            Some(content),
        )?)
        .trim()
        .to_string();
        git(
            &self.root,
            &[
                "update-index",
                "--add",
                "--cacheinfo",
                &format!("{},{hash},{path}", self.mode(path)),
            ],
        )
        .map(drop)
    }

    pub fn committed(&self, path: &str) -> Option<String> {
        git(&self.root, &["show", &format!("{}:{path}", self.base())]).ok()
    }

    pub fn staged(&self, path: &str) -> Option<String> {
        git(&self.root, &["show", &format!(":{path}")]).ok()
    }

    pub fn roll_back(&self, path: &str) -> Result<Vec<String>, String> {
        let base = self.base();
        let changed = git(
            &self.root,
            &["diff", "--name-only", "--no-renames", base, "--", path],
        )?;
        let mut rolled_back = Vec::new();
        for file in changed.lines().filter(|file| !file.is_empty()) {
            if git(&self.root, &["cat-file", "-e", &format!("{base}:{file}")]).is_ok() {
                git(
                    &self.root,
                    &[
                        "restore",
                        "--quiet",
                        "--source",
                        base,
                        "--staged",
                        "--worktree",
                        "--",
                        file,
                    ],
                )?;
            } else {
                git(
                    &self.root,
                    &["rm", "--cached", "--force", "--quiet", "--", file],
                )?;
                let _ = fs::remove_file(self.root.join(file));
            }
            rolled_back.push(file.to_string());
        }
        for file in self.untracked(&[path])? {
            fs::remove_file(self.root.join(&file))
                .map_err(|e| format!("Cannot remove {file}: {e}"))?;
            rolled_back.push(file);
        }
        Ok(rolled_back)
    }

    pub fn stage_files(&self, paths: &[&str], staged: bool) -> Result<(), String> {
        let args: &[&str] = match (staged, self.base()) {
            (true, _) => &["add", "--all", "--"],
            (false, EMPTY_TREE) => &["rm", "--cached", "--quiet", "--"],
            (false, _) => &["reset", "--quiet", "--"],
        };
        git(&self.root, &[args, paths].concat()).map(drop)
    }

    pub fn stage_file(&self, path: &str, staged: bool) -> Result<(), String> {
        self.stage_files(&[path], staged)
    }

    fn read_work(&self, path: &str) -> Vec<u8> {
        let file = self.root.join(path);
        match fs::read_link(&file) {
            Ok(target) => target.to_string_lossy().into_owned().into_bytes(),
            Err(_) => fs::read(&file).unwrap_or_default(),
        }
    }

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

fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    run(dir, args, None).map(|out| String::from_utf8_lossy(&out).into_owned())
}

fn run(dir: &Path, args: &[&str], input: Option<&[u8]>) -> Result<Vec<u8>, String> {
    let mut child = Command::new("git")
        .args(["-c", "core.quotepath=false"])
        .args(args)
        .current_dir(dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to run git: {e}"))?;
    if let (Some(input), Some(mut stdin)) = (input, child.stdin.take()) {
        stdin
            .write_all(input)
            .map_err(|e| format!("failed to run git: {e}"))?;
    }
    let output = child
        .wait_with_output()
        .map_err(|e| format!("failed to run git: {e}"))?;
    if !output.status.success() {
        let said = |out: &[u8]| String::from_utf8_lossy(out).trim().to_string();
        return Err(Some(said(&output.stderr))
            .filter(|error| !error.is_empty())
            .unwrap_or_else(|| said(&output.stdout)));
    }
    Ok(output.stdout)
}

#[cfg(test)]
pub mod tests;
