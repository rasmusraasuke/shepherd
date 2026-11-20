use super::staging::compose;
use super::*;
use crate::testing::TempDir;

pub fn temp_repo(name: &str, files: &[(&str, &str)]) -> (Repo, TempDir) {
    let dir = TempDir::new(name);
    for (path, content) in files {
        dir.write(path, content);
    }
    git(dir.path(), &["init", "--quiet"]).unwrap();
    identify(dir.path());
    (Repo::open(dir.path()).unwrap(), dir)
}

fn identify(dir: &Path) {
    git(dir, &["config", "user.name", "Test"]).unwrap();
    git(dir, &["config", "user.email", "test@example.com"]).unwrap();
    git(dir, &["config", "commit.gpgsign", "false"]).unwrap();
}

pub fn with_remote(repo: &Repo, name: &str) -> TempDir {
    let remote = TempDir::new(name);
    git(remote.path(), &["init", "--quiet", "--bare"]).unwrap();
    let url = remote.path().to_string_lossy().into_owned();
    git(&repo.root, &["remote", "add", "origin", &url]).unwrap();
    remote
}

fn short_status(repo: &Repo) -> String {
    git(&repo.root, &["status", "--short"]).unwrap()
}

#[test]
fn commits_what_is_staged_pushes_it_and_pulls_what_others_pushed() {
    let (repo, dir) = temp_repo("flow", &[("a.txt", "a\n"), ("b.txt", "b\n")]);
    git(&repo.root, &["config", "pull.rebase", "true"]).unwrap();
    repo.stage_file("a.txt", true).unwrap();
    repo.commit("add a").unwrap();
    assert_eq!(short_status(&repo), "?? b.txt\n");
    assert!(!repo.commit("nothing").unwrap_err().is_empty());
    let status = repo.status();
    assert_eq!(
        (status.upstream, status.branches),
        (false, vec![status.branch.clone()])
    );
    assert!(repo.push().unwrap_err().contains("no remote"));

    let remote = with_remote(&repo, "flow-remote");
    repo.push().unwrap();
    let status = repo.status();
    assert_eq!((status.upstream, status.ahead, status.behind), (true, 0, 0));

    let other = TempDir::new("flow-other");
    let url = remote.path().to_string_lossy().into_owned();
    git(other.path(), &["clone", "--quiet", &url, "."]).unwrap();
    identify(other.path());
    other.write("c.txt", "c\n");
    let other = Repo::open(other.path()).unwrap();
    commit_all(&other);
    other.push().unwrap();

    repo.stage_file("b.txt", true).unwrap();
    dir.write("b.txt", "b\nmore\n");
    dir.write("new.txt", "new\n");
    repo.commit("add b").unwrap();
    dir.write("a.txt", "A\n");
    repo.stage_file("a.txt", true).unwrap();
    repo.sync().unwrap();
    let status = repo.status();
    assert_eq!((status.ahead, status.behind), (2, 0));
    assert!(dir.path().join("c.txt").exists());
    assert_eq!(short_status(&repo), "M  a.txt\n M b.txt\n?? new.txt\n");
    repo.push().unwrap();
    assert_eq!(repo.status().ahead, 0);
}

#[test]
fn a_pull_that_runs_into_conflicts_is_taken_back() {
    let (repo, dir) = temp_repo("conflict", &[("a.txt", "a\n")]);
    commit_all(&repo);
    let remote = with_remote(&repo, "conflict-remote");
    repo.push().unwrap();
    let other = TempDir::new("conflict-other");
    let url = remote.path().to_string_lossy().into_owned();
    git(other.path(), &["clone", "--quiet", &url, "."]).unwrap();
    identify(other.path());
    other.write("a.txt", "theirs\n");
    let other = Repo::open(other.path()).unwrap();
    commit_all(&other);
    other.push().unwrap();

    dir.write("a.txt", "ours\n");
    commit_all(&repo);
    dir.write("b.txt", "b\n");
    assert!(repo.sync().is_err());
    let status = repo.status();
    assert_eq!((status.ahead, status.behind), (1, 1));
    assert_eq!(short_status(&repo), "?? b.txt\n");

    dir.write("a.txt", "ours\nand more\n");
    assert!(
        repo.sync()
            .unwrap_err()
            .contains("Commit the changes of a.txt")
    );
    assert_eq!(short_status(&repo), " M a.txt\n?? b.txt\n");
}

#[test]
fn switches_branches_taking_the_changes_along_or_leaving_them() {
    let (repo, dir) = temp_repo("branches", &[("a.txt", "a\n")]);
    commit_all(&repo);
    let main = repo.branch();
    dir.write("a.txt", "changed\n");
    repo.create_branch("feature").unwrap();
    assert_eq!(repo.branch(), "feature");
    assert_eq!(short_status(&repo), " M a.txt\n");
    commit_all(&repo);

    dir.write("a.txt", "changed again\n");
    repo.stage_file("a.txt", true).unwrap();
    dir.write("new.txt", "new\n");
    repo.switch(&main, true).unwrap();
    assert_eq!(
        (repo.branch(), short_status(&repo)),
        (main.clone(), String::new())
    );
    assert_eq!(repo.status().left, ["feature"]);
    assert_eq!(repo.status().branches, ["feature", main.as_str()]);

    dir.write("b.txt", "b\n");
    repo.switch("feature", true).unwrap();
    assert_eq!(short_status(&repo), "M  a.txt\n?? new.txt\n");
    assert_eq!(repo.status().left, [main.as_str()]);

    repo.switch(&main, false).unwrap_err();
    assert_eq!(
        (repo.branch().as_str(), short_status(&repo).as_str()),
        ("feature", "M  a.txt\n?? new.txt\n")
    );
    repo.stage_file("a.txt", false).unwrap();
    git(&repo.root, &["checkout", "--", "a.txt"]).unwrap();
    repo.switch(&main, false).unwrap();
    assert_eq!(
        (repo.branch(), short_status(&repo)),
        (main, "?? new.txt\n".to_string())
    );
}

pub fn commit_all(repo: &Repo) {
    git(&repo.root, &["add", "--all"]).unwrap();
    git(
        &repo.root,
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "--quiet",
            "-m",
            "test",
        ],
    )
    .unwrap();
}

#[test]
fn lists_project_files_without_ignored_ones() {
    let (repo, _dir) = temp_repo(
        "list-files",
        &[
            ("src/main.rs", ""),
            ("README.md", ""),
            (".gitignore", "/target\n"),
            ("target/debug/editor", ""),
        ],
    );
    assert_eq!(
        repo.list_files().unwrap(),
        [".gitignore", "README.md", "src/main.rs"]
    );
}

#[test]
fn diffs_new_files_as_git_does() {
    let (repo, _dir) = temp_repo(
        "added",
        &[
            ("a.txt", "one\r\ntwo"),
            ("dir/b.bin", "\0\u{1}"),
            ("empty", ""),
        ],
    );
    let git_diff = |path: &str| {
        let output = Command::new("git")
            .args(["diff", "--no-color", "--no-index", "--", "/dev/null", path])
            .current_dir(&repo.root)
            .output()
            .unwrap();
        format!("{:?}", parse_diff(&String::from_utf8_lossy(&output.stdout)))
    };
    for path in ["a.txt", "dir/b.bin", "empty"] {
        assert_eq!(
            format!(
                "{:?}",
                parse_diff(&added_file_diff(path, &repo.root.join(path)))
            ),
            git_diff(path),
            "{path}"
        );
    }
    let files = parse_diff(&repo.changes().unwrap().diff);
    assert_eq!(
        files
            .iter()
            .map(|file| (file.path.as_str(), file.status, file.binary))
            .collect::<Vec<_>>(),
        [
            ("a.txt", FileStatus::Added, false),
            ("dir/b.bin", FileStatus::Added, true),
            ("empty", FileStatus::Added, false)
        ]
    );
}

fn diff_of(repo: &Repo, path: &str) -> FileDiff {
    repo.file_diff(path, None).unwrap().unwrap()
}

fn staged(repo: &Repo, path: &str) -> String {
    git(&repo.root, &["show", &format!(":{path}")]).unwrap()
}

fn staged_lines(repo: &Repo, path: &str) -> Vec<String> {
    let staged = Staged::new(&repo.changes().unwrap());
    let diff = diff_of(repo, path);
    let lines = diff
        .hunks
        .iter()
        .flat_map(|hunk| &hunk.lines)
        .filter(|line| line.kind != LineKind::Context);
    let staged = lines.filter(|line| staged.is_staged(path, line));
    staged
        .map(|line| {
            format!(
                "{}{}",
                if line.kind == LineKind::Added {
                    "+"
                } else {
                    "-"
                },
                line.text
            )
        })
        .collect()
}

#[test]
fn makes_a_version_from_head_with_the_chosen_changes() {
    let raw = "diff --git a/a b/a\n--- a/a\n+++ b/a\n@@ -2 +2 @@\n-b\n+B\n@@ -4,0 +5 @@\n+e\n";
    let hunks = &parse_diff(raw)[0].hunks;
    let (head, work) = (b"a\r\nb\r\nc\r\nd", b"a\r\nB\r\nc\r\nd\r\ne\r\n");
    let compose = |chosen: &[(usize, usize)]| {
        String::from_utf8(compose(head, work, hunks, |h, i| chosen.contains(&(h, i)))).unwrap()
    };
    assert_eq!(compose(&[]), "a\r\nb\r\nc\r\nd");
    assert_eq!(compose(&[(0, 0)]), "a\r\nc\r\nd");
    assert_eq!(compose(&[(0, 1), (1, 0)]), "a\r\nb\r\nB\r\nc\r\nd\ne\r\n");
    assert_eq!(
        compose(&[(0, 0), (0, 1), (1, 0)]).replace("d\n", "d\r\n"),
        String::from_utf8(work.to_vec()).unwrap()
    );
}

#[test]
fn stages_the_chosen_lines_of_a_file() {
    let (repo, dir) = temp_repo("stage-lines", &[("a.txt", "a\nb\nc\nd\n")]);
    commit_all(&repo);
    dir.write("a.txt", "a\nB\nc\nd\ne\n");
    let diff = diff_of(&repo, "a.txt");
    assert!(staged_lines(&repo, "a.txt").is_empty());

    repo.stage_lines(&diff, |h, i| (h, i) == (0, 0)).unwrap();
    assert_eq!(staged(&repo, "a.txt"), "a\nc\nd\n");
    assert_eq!(staged_lines(&repo, "a.txt"), ["-b"]);

    repo.stage_lines(&diff, |h, _| h == 1).unwrap();
    assert_eq!(staged_lines(&repo, "a.txt"), ["+e"]);
    repo.stage_lines(&diff, |_, _| true).unwrap();
    assert_eq!(staged(&repo, "a.txt"), "a\nB\nc\nd\ne\n");
    assert_eq!(staged_lines(&repo, "a.txt"), ["-b", "+B", "+e"]);

    dir.write("a.txt", "a\nB\nc\nD\ne\n");
    assert_eq!(staged_lines(&repo, "a.txt"), ["-b", "+B", "+e"]);
    assert!(
        repo.stage_lines(&diff, |_, _| false)
            .unwrap_err()
            .contains("changed in the meantime")
    );
}

#[test]
fn keeps_a_rename_as_a_rename_when_its_changes_are_staged() {
    let (repo, dir) = temp_repo("stage-rename", &[("old.txt", "one\ntwo\n")]);
    commit_all(&repo);
    git(&repo.root, &["mv", "old.txt", "new.txt"]).unwrap();
    dir.write("new.txt", "one\nTWO\n");
    let changes = repo.changes().unwrap();
    let diff = &parse_diff(&changes.diff)[0];
    assert_eq!(
        (diff.path.as_str(), diff.old_path.as_deref(), diff.status),
        ("new.txt", Some("old.txt"), FileStatus::Renamed)
    );
    assert_eq!(diff.display_name(), "old.txt -> new.txt");

    repo.stage_lines(diff, |_, _| true).unwrap();
    assert_eq!(staged(&repo, "new.txt"), "one\nTWO\n");
    assert!(repo.staged("old.txt").is_none());
    assert_eq!(short_status(&repo), "R  old.txt -> new.txt\n");

    let changes = repo.changes().unwrap();
    let cached = &parse_diff(&changes.staged)[0];
    assert_eq!(
        (
            cached.path.as_str(),
            cached.old_path.as_deref(),
            cached.status
        ),
        ("new.txt", Some("old.txt"), FileStatus::Renamed)
    );
}

#[test]
fn stages_new_and_deleted_files_and_takes_them_back() {
    let (repo, dir) = temp_repo("stage-files", &[("old.txt", "one\ntwo\n")]);
    commit_all(&repo);
    fs::remove_file(dir.path().join("old.txt")).unwrap();
    dir.write("new.txt", "x\ny\n");
    let tracked = |path: &str| {
        !git(&repo.root, &["ls-files", "--", path])
            .unwrap()
            .is_empty()
    };

    let new = diff_of(&repo, "new.txt");
    repo.stage_lines(&new, |_, i| i == 1).unwrap();
    assert_eq!(
        (
            staged(&repo, "new.txt").as_str(),
            staged_lines(&repo, "new.txt")
        ),
        ("y\n", vec!["+y".to_string()])
    );
    repo.stage_lines(&new, |_, _| false).unwrap();
    assert!(!tracked("new.txt"));

    let old = diff_of(&repo, "old.txt");
    repo.stage_lines(&old, |_, i| i == 0).unwrap();
    assert_eq!(staged(&repo, "old.txt"), "two\n");
    repo.stage_lines(&old, |_, _| true).unwrap();
    assert!(!tracked("old.txt"));
    assert_eq!(staged_lines(&repo, "old.txt"), ["-one", "-two"]);

    dir.write("image.bin", "\0");
    let staged_file =
        |repo: &Repo| Staged::new(&repo.changes().unwrap()).is_file_staged("image.bin");
    repo.stage_file("image.bin", true).unwrap();
    assert!(staged_file(&repo));
    repo.stage_file("image.bin", false).unwrap();
    assert!(!staged_file(&repo));
}

#[test]
fn stages_many_files_at_once_and_takes_them_back() {
    let (repo, dir) = temp_repo("stage-many", &[("a.txt", "a\n"), ("b.txt", "b\n")]);
    commit_all(&repo);
    dir.write("a.txt", "A\n");
    fs::remove_file(dir.path().join("b.txt")).unwrap();
    dir.write("c.txt", "c\n");
    repo.stage_files(&["a.txt", "b.txt", "c.txt"], true)
        .unwrap();
    assert_eq!(short_status(&repo), "M  a.txt\nD  b.txt\nA  c.txt\n");
    repo.stage_files(&["a.txt", "b.txt", "c.txt"], false)
        .unwrap();
    assert_eq!(short_status(&repo), " M a.txt\n D b.txt\n?? c.txt\n");
}

#[test]
fn rolls_back_the_changes_of_a_file_or_a_directory() {
    let (repo, dir) = temp_repo(
        "roll-back",
        &[
            ("a.txt", "a\n"),
            ("src/kept.txt", "k\n"),
            ("src/edited.txt", "e\n"),
        ],
    );
    commit_all(&repo);
    dir.write("a.txt", "changed\n");
    dir.write("src/edited.txt", "edited\n");
    repo.stage_file("src/edited.txt", true).unwrap();
    dir.write("src/new.txt", "new\n");
    dir.write("staged-new.txt", "staged\n");
    repo.stage_file("staged-new.txt", true).unwrap();
    dir.write("loose.txt", "loose\n");
    assert_eq!(
        short_status(&repo),
        " M a.txt\nM  src/edited.txt\nA  staged-new.txt\n?? loose.txt\n?? src/new.txt\n"
    );

    assert_eq!(repo.roll_back("a.txt").unwrap(), ["a.txt"]);
    assert_eq!(fs::read_to_string(dir.path().join("a.txt")).unwrap(), "a\n");

    assert_eq!(
        repo.roll_back("src").unwrap(),
        ["src/edited.txt", "src/new.txt"]
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("src/edited.txt")).unwrap(),
        "e\n"
    );
    assert!(!dir.path().join("src/new.txt").exists());

    repo.roll_back("staged-new.txt").unwrap();
    assert!(!dir.path().join("staged-new.txt").exists());
    repo.roll_back("loose.txt").unwrap();
    assert!(!dir.path().join("loose.txt").exists());
    assert_eq!(short_status(&repo), "");
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
    assert_eq!(
        (lines[1].kind, lines[1].old_no, lines[1].new_no),
        (LineKind::Removed, Some(2), None)
    );
    assert_eq!((lines[3].kind, lines[3].new_no), (LineKind::Added, Some(3)));
    assert_eq!((lines[4].old_no, lines[4].new_no), (Some(3), Some(4)));

    let new = &files[1];
    assert_eq!(
        (new.path.as_str(), new.status),
        ("new.txt", FileStatus::Added)
    );
    assert_eq!(new.hunks[0].lines[0].text, "--- not a header");

    let old = &files[2];
    assert_eq!(
        (old.path.as_str(), old.status),
        ("old.txt", FileStatus::Deleted)
    );
    assert_eq!(old.hunks[0].lines[0].text, "bye");

    assert_eq!(main.hunks[0].ranges(), (1..4, 1..5));
    assert_eq!(new.hunks[0].ranges(), (1..1, 1..2));
    assert_eq!(old.hunks[0].ranges(), (1..2, 1..1));
}
