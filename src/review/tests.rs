use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};

use std::fs;
use std::process::Command;

use super::rows::{Row, rows};
use super::*;
use crate::git::tests::{commit_all, conflict_repo, temp_repo};
use crate::testing::TempDir;

const DIFF: &str = "\
diff --git a/a.txt b/a.txt
--- a/a.txt
+++ b/a.txt
@@ -1 +1 @@
-one
+two
@@ -9 +9 @@
-three
+four
diff --git a/b.txt b/b.txt
--- a/b.txt
+++ b/b.txt
@@ -1 +1 @@
-five
+six
";

fn review(name: &str, files: &[(&str, &str, &str)]) -> (Review, TempDir) {
    let (repo, dir) = temp_repo(
        name,
        &files
            .iter()
            .map(|(path, old, _)| (*path, *old))
            .collect::<Vec<_>>(),
    );
    commit_all(&repo);
    for (path, _, new) in files {
        dir.write(path, new);
    }
    let mut review = Review::new(repo);
    refresh(&mut review);
    (review, dir)
}

fn refresh(review: &mut Review) {
    review.wait_for_staging();
    let changes = review.repo.changes().unwrap();
    review.set_changes(&changes, &BTreeSet::new());
}

fn set_reviewed(review: &mut Review, path: &str, reviewed: bool) {
    review.set_reviewed(path, reviewed, &egui::Context::default());
    review.wait_for_staging();
}

const TEN: &str = "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n";
const TWO_FILES: &[(&str, &str, &str)] = &[
    ("a.txt", TEN, "one\n2\n3\n4\n5\n6\n7\n8\nnine\n10\n"),
    ("b.txt", "x\n", "y\n"),
];

#[test]
fn marking_a_hunk_reviewed_stages_it_and_moves_on_to_the_next_unreviewed_one() {
    let (mut review, dir) = review("moves-on", TWO_FILES);
    assert_eq!(review.progress(), (0, 6));
    assert_eq!(review.toggle_reviewed(0, 0), None);
    assert_eq!((review.progress(), review.view(0).current), ((2, 6), 1));
    assert_eq!(review.toggle_reviewed(0, 1), Some("b.txt".to_string()));
    assert_eq!(
        (review.progress_of("a.txt"), review.view(0).current),
        ((4, 4), 1)
    );
    assert!(review.take_changed());

    refresh(&mut review);
    assert_eq!(review.progress(), (4, 6));
    let in_index = Command::new("git")
        .args(["show", ":a.txt"])
        .current_dir(dir.path())
        .output()
        .unwrap()
        .stdout;
    assert_eq!(String::from_utf8(in_index).unwrap(), TWO_FILES[0].2);
    review.toggle_reviewed(0, 1);
    refresh(&mut review);
    assert_eq!(review.progress(), (2, 6));
}

#[test]
fn marks_single_lines_of_a_hunk_reviewed() {
    let (mut review, _dir) = review("lines", TWO_FILES);
    review.toggle_line(0, 0, 0);
    refresh(&mut review);
    assert_eq!(
        (
            review.progress_of("a.txt"),
            review.files[0].hunk_progress(0)
        ),
        ((1, 4), (1, 2))
    );
    assert!(!review.files[0].is_reviewed(0));
    review.toggle_reviewed(0, 0);
    refresh(&mut review);
    assert!(review.files[0].is_reviewed(0));
    review.toggle_line(0, 0, 1);
    refresh(&mut review);
    assert_eq!(review.files[0].staged[0][..2], [true, false]);
}

#[test]
fn each_run_of_changed_lines_is_a_hunk_with_marks_for_single_lines_under_the_pointer() {
    let (mut review, _dir) = review(
        "runs",
        &[("a.txt", TEN, "1\ntwo\n3\nfour\n5\n6\n7\n8\n9\n10\n")],
    );
    assert_eq!(review.files[0].diff.hunks.len(), 2);
    review.toggle_line(0, 1, 0);
    let mut harness = show(review, Some("a.txt"), 600.0);
    assert_eq!(harness.query_all_by_label("Reviewed").count(), 2);
    assert_eq!(harness.query_all_by_label("Reviewed line").count(), 2);
    harness.get_all_by_label("Reviewed").next().unwrap().hover();
    harness.run();
    assert_eq!(harness.query_all_by_label("Reviewed line").count(), 3);
}

struct Shown {
    review: Review,
    files: OpenFiles,
    asked: Option<Outcome>,
}

fn show(review: Review, path: Option<&'static str>, height: f32) -> Harness<'static, Shown> {
    show_with(
        review,
        path,
        height,
        Agents {
            open: vec![(1, "agent".to_string())],
            new: None,
        },
    )
}

fn show_with(
    review: Review,
    path: Option<&'static str>,
    height: f32,
    agents: Agents,
) -> Harness<'static, Shown> {
    let files = OpenFiles::new(review.repo.root.clone());
    let ui = move |ui: &mut egui::Ui, shown: &mut Shown| {
        if let Some(asked) = shown.review.ui(ui, path, true, &agents, &mut shown.files) {
            shown.asked = Some(asked);
        }
    };
    let mut harness = Harness::builder()
        .with_size(egui::vec2(800.0, height))
        .build_ui_state(
            ui,
            Shown {
                review,
                files,
                asked: None,
            },
        );
    harness.run();
    harness
}

fn comment_harness(name: &str) -> (Harness<'static, Shown>, TempDir) {
    let (review, dir) = review(name, &[("a.txt", TEN, &TEN.replace("2\n", "two\n"))]);
    (show(review, Some("a.txt"), 600.0), dir)
}

fn row_center(harness: &Harness<'_, Shown>, below_hunk: f32) -> egui::Pos2 {
    let mark = harness.get_by_label("Reviewed").rect();
    mark.center() + egui::vec2(300.0, below_hunk * mark.height())
}

fn comment_on_row(harness: &mut Harness<'_, Shown>, below_hunk: f32, text: &str) {
    harness.event(egui::Event::PointerMoved(row_center(harness, below_hunk)));
    harness.run();
    harness.get_by_label("Comment on this line").click();
    harness.run();
    harness.event(egui::Event::Text(text.to_string()));
    harness.run();
}

#[test]
fn a_commit_is_reviewed_read_only_with_its_committed_content() {
    let (repo, dir) = temp_repo("commit-review", &[("a.txt", "one\n")]);
    commit_all(&repo);
    dir.write("a.txt", "two\n");
    dir.write("b.txt", "b\n");
    commit_all(&repo);
    let hash = repo.log().unwrap()[0].hash.clone();
    let range = repo.commit_range(&hash).unwrap();
    let review = Review::for_commits(repo, range, hash[..7].to_string());
    let harness = show(review, Some("a.txt"), 600.0);

    assert_eq!(
        harness.state().review.texts["a.txt"].work_lines,
        ["two"].map(str::to_string)
    );
    assert_eq!(harness.state().review.progress_of("a.txt"), (2, 2));
    assert!(harness.query_by_label("Mark file reviewed").is_none());
    assert!(harness.query_by_label("Mark file not reviewed").is_none());
    assert_eq!(harness.query_all_by_label("Reviewed").count(), 1);
    assert!(harness.state().review.first_file().is_some());
}

#[test]
fn comments_on_a_committed_line_are_sent_about_the_commit() {
    let (repo, dir) = temp_repo("commit-comment", &[("a.txt", TEN)]);
    commit_all(&repo);
    dir.write("a.txt", TEN.replace("2\n", "two\n"));
    commit_all(&repo);
    let hash = repo.log().unwrap()[0].hash.clone();
    let range = repo.commit_range(&hash).unwrap();
    let review = Review::for_commits(repo, range, hash[..7].to_string());
    let mut harness = show(review, Some("a.txt"), 600.0);
    comment_on_row(&mut harness, 1.0, "Use two");
    harness.get_by_label("Comment").click();
    harness.run();
    harness.get_by_label("Send 1 comment").click();
    harness.run();
    assert!(matches!(
        &harness.state().asked,
        Some(Outcome::Send { text, .. })
            if text.starts_with("Please address these review comments on commit ")
    ));
}

#[test]
fn a_hunk_cannot_be_marked_in_a_commit() {
    let (repo, dir) = temp_repo("commit-marks", &[("a.txt", "one\n")]);
    commit_all(&repo);
    dir.write("a.txt", "two\n");
    commit_all(&repo);
    let hash = repo.log().unwrap()[0].hash.clone();
    let range = repo.commit_range(&hash).unwrap();
    let mut review = Review::for_commits(repo, range, hash[..7].to_string());
    assert_eq!(review.toggle_reviewed(0, 0), None);
    review.toggle_line(0, 0, 0);
    assert_eq!(review.progress(), (2, 2));
}

#[test]
fn comments_on_a_line_until_it_changes() {
    let (mut harness, dir) = comment_harness("comment");
    comment_on_row(&mut harness, 1.0, "Use 2");
    harness.get_by_label("Comment").click();
    harness.run();
    let comments = harness.state().review.comments().to_vec();
    assert_eq!(
        comments
            .iter()
            .map(|comment| (comment.lines[0].text.as_str(), comment.text.as_str()))
            .collect::<Vec<_>>(),
        [("two", "Use 2")]
    );
    harness.get_by_label("Delete comment");

    dir.write(
        "a.txt",
        TEN.replace("2\n", "two\n").replace("9\n", "nine\n"),
    );
    refresh(&mut harness.state_mut().review);
    assert_eq!(harness.state().review.comments().len(), 1);
    dir.write("a.txt", TEN.replace("2\n", "TWO\n"));
    refresh(&mut harness.state_mut().review);
    assert!(harness.state().review.comments().is_empty());
}

#[test]
fn asks_an_agent_about_a_line() {
    let (mut harness, _dir) = comment_harness("ask");
    comment_on_row(&mut harness, 0.0, "Why?");
    harness.get_by_label("Ask agent").click();
    harness.run();
    let text =
        "@a.txt, removed line 2 of the committed version\n```diff\n-2\n```\nWhy?".to_string();
    assert_eq!(
        harness.state().asked,
        Some(Outcome::Send {
            agent: Some(1),
            text
        })
    );
    assert!(harness.state().review.comments().is_empty());
    assert!(harness.query_by_label("Ask agent").is_none());
}

#[test]
fn starts_a_new_agent_for_comments_when_none_is_open() {
    let (review, _dir) = review("new-agent", &[("a.txt", TEN, &TEN.replace("2\n", "two\n"))]);
    let mut harness = show_with(
        review,
        Some("a.txt"),
        600.0,
        Agents {
            open: Vec::new(),
            new: Some("Claude Code".to_string()),
        },
    );
    comment_on_row(&mut harness, 1.0, "Use 2");
    harness.get_by_label("Comment").click();
    harness.run();
    harness.get_by_label("Send 1 comment").click();
    harness.run();
    assert!(matches!(
        harness.state().asked,
        Some(Outcome::Send { agent: None, .. })
    ));
}

#[test]
fn selects_rows_by_dragging_and_sends_the_comments_on_them_as_one() {
    let (mut harness, _dir) = comment_harness("drag");
    harness.event(egui::Event::PointerMoved(row_center(&harness, -1.0)));
    harness.run();
    let start = harness.get_by_label("Comment on this line").rect().center();
    let end = egui::pos2(start.x, row_center(&harness, 1.0).y);
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    harness.event(button(start, true));
    harness.run();
    harness.event(egui::Event::PointerMoved(end));
    harness.run();
    harness.event(button(end, false));
    harness.run();
    harness.event(egui::Event::Text("Simpler".to_string()));
    harness.key_press_modifiers(egui::Modifiers::COMMAND, Key::Enter);
    harness.run();
    let comment = harness.state().review.comments()[0].clone();
    assert_eq!(
        comment
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        ["1", "2", "two"]
    );

    harness.get_by_label("Send 1 comment").click();
    harness.run();
    let text = format!(
        "Please address these review comments on the uncommitted changes:\n\n{}",
        comments::quote("a.txt", &comment.lines, "Simpler")
    );
    assert_eq!(
        harness.state().asked,
        Some(Outcome::Send {
            agent: Some(1),
            text
        })
    );
    assert!(harness.state().review.comments()[0].sent);
    assert!(harness.query_by_label("Send 1 comment").is_none());
}

fn click_at(harness: &mut Harness<'_, Shown>, pos: egui::Pos2, button: egui::PointerButton) {
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: Modifiers::NONE,
        });
        harness.run();
    }
}

#[test]
fn edits_the_file_anywhere_and_shows_what_changed_as_it_is_typed() {
    let (mut harness, dir) = comment_harness("edit");
    harness.state_mut().review.toggle_reviewed(0, 0);
    let pos = row_center(&harness, 2.0);
    click_at(&mut harness, pos, egui::PointerButton::Primary);
    harness.event(egui::Event::Text("j".to_string()));
    harness.run();
    harness.run();
    let review = &harness.state().review;
    let shown: Vec<(LineKind, &str, bool)> = review.files[0].diff.hunks[0]
        .lines
        .iter()
        .zip(&review.files[0].staged[0])
        .map(|(line, staged)| (line.kind, line.text.as_str(), *staged))
        .collect();
    assert_eq!(
        shown,
        [
            (LineKind::Removed, "2", true),
            (LineKind::Removed, "3", false),
            (LineKind::Added, "two", true),
            (LineKind::Added, "3j", false)
        ]
    );
    harness.state_mut().files.save_all();
    assert_eq!(
        fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        TEN.replace("2\n3\n", "two\n3j\n")
    );

    harness.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    harness.run();
    harness.run();
    assert_eq!(harness.state().review.files[0].diff.hunks[0].lines.len(), 2);
    let (mark, editor) = (
        harness.get_by_label("Reviewed").rect(),
        harness.get_by_label("Editor").rect(),
    );
    assert!(
        editor.top() <= mark.top()
            && editor.bottom() >= mark.bottom()
            && editor.left() > mark.right(),
        "{editor:?} by {mark:?}"
    );
}

#[test]
fn opens_the_place_of_a_change_in_the_file() {
    let (mut harness, _dir) = comment_harness("open");
    harness.key_press(Key::O);
    harness.run();
    assert_eq!(
        harness.state().asked,
        Some(Outcome::Open {
            path: "a.txt".to_string(),
            line: 1
        })
    );
}

#[test]
fn reverts_a_hunk_from_its_menu_or_the_selected_lines_and_undoes_it() {
    let (mut harness, dir) = comment_harness("revert");
    let text = |harness: &mut Harness<'_, Shown>| {
        harness
            .state_mut()
            .files
            .text("a.txt", false)
            .unwrap()
            .0
            .text()
    };
    let pos = row_center(&harness, 1.0);
    click_at(&mut harness, pos, egui::PointerButton::Secondary);
    harness.get_by_label("Revert").click();
    harness.run();
    assert_eq!(text(&mut harness), TEN);
    harness.run();
    harness.get_by_label("No changes left");
    harness.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    harness.run();
    assert_eq!(text(&mut harness), TEN.replace("2\n", "two\n"));

    let Shown { review, files, .. } = harness.state_mut();
    review.toggle_reviewed(0, 0);
    review.revert(0, files.text("a.txt", false).unwrap().0, |_, i| i == 1);
    files.save_all();
    assert_eq!(
        fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        TEN.replace("2\n", "")
    );
    refresh(review);
    assert_eq!(review.progress(), (1, 1));
}

#[test]
fn a_deleted_file_shows_its_lines_and_comes_back_when_typed_into() {
    let (mut review, dir) = review("deleted", &[("a.txt", "one\n", "one\n")]);
    fs::remove_file(dir.path().join("a.txt")).unwrap();
    refresh(&mut review);
    let mut harness = show(review, Some("a.txt"), 600.0);
    assert_eq!(harness.state().review.progress(), (0, 1));
    let pos = row_center(&harness, 1.0);
    click_at(&mut harness, pos, egui::PointerButton::Primary);
    harness.event(egui::Event::Text("two".to_string()));
    harness.run();
    harness.state_mut().files.save_all();
    assert_eq!(fs::read_to_string(dir.path().join("a.txt")).unwrap(), "two");
}

#[test]
fn shows_the_hunk_nearest_to_a_line() {
    let (mut review, _dir) = review("show-line", TREE);
    review.show_line("src/a.txt", 6);
    assert_eq!(review.view(review.by_path["src/a.txt"]).current, 1);
    review.show_line("src/a.txt", 3);
    assert_eq!(review.view(review.by_path["src/a.txt"]).current, 0);
}

#[test]
fn a_change_made_after_the_review_is_not_reviewed() {
    let (mut review, dir) = review("changed-after", TWO_FILES);
    set_reviewed(&mut review, "b.txt", true);
    dir.write("b.txt", "z\n");
    refresh(&mut review);
    assert_eq!(review.progress_of("b.txt"), (1, 2));
}

#[test]
fn marks_files_without_lines_reviewed_as_a_whole() {
    let (mut review, _dir) = review("binary", &[("image.bin", "\0a", "\0b")]);
    assert_eq!(review.progress(), (0, 1));
    set_reviewed(&mut review, "image.bin", true);
    refresh(&mut review);
    assert_eq!(review.progress(), (1, 1));
}

#[test]
fn a_renamed_file_is_one_review_and_stages_as_a_rename() {
    let (repo, dir) = temp_repo("rename", &[("old.txt", "one\ntwo\n")]);
    commit_all(&repo);
    Command::new("git")
        .args(["mv", "old.txt", "new.txt"])
        .current_dir(dir.path())
        .status()
        .unwrap();
    dir.write("new.txt", "one\nTWO\n");
    let mut review = Review::new(repo);
    refresh(&mut review);
    assert_eq!(
        (
            review.files[0].diff.display_name(),
            review.files[0].hunk_progress(0)
        ),
        ("old.txt -> new.txt".to_string(), (0, 2))
    );

    let mut harness = show(review, Some("new.txt"), 600.0);
    harness.get_by_label("Reviewed").click();
    harness.run();
    harness.state_mut().review.wait_for_staging();
    refresh(&mut harness.state_mut().review);
    assert_eq!(harness.state().review.progress_of("new.txt"), (2, 2));

    let output = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .current_dir(dir.path())
            .output()
            .unwrap()
    };
    let status = output(&["status", "--short"]);
    assert_eq!(
        String::from_utf8(status.stdout).unwrap(),
        "R  old.txt -> new.txt\n"
    );
    let staged = output(&["show", ":new.txt"]);
    assert_eq!(String::from_utf8(staged.stdout).unwrap(), "one\nTWO\n");
    assert!(!output(&["show", ":old.txt"]).status.success());

    harness.state_mut().review.roll_back("new.txt");
    refresh(&mut harness.state_mut().review);
    assert_eq!(
        (
            dir.path().join("old.txt").exists(),
            dir.path().join("new.txt").exists(),
            harness.state().review.files.len()
        ),
        (true, false, 0)
    );
}

#[test]
fn marking_a_renamed_file_reviewed_from_the_tree_keeps_the_rename() {
    let (repo, dir) = temp_repo("rename-tree", &[("old.txt", "one\ntwo\n")]);
    commit_all(&repo);
    Command::new("git")
        .args(["mv", "old.txt", "new.txt"])
        .current_dir(dir.path())
        .status()
        .unwrap();
    dir.write("new.txt", "one\nTWO\n");
    let mut review = Review::new(repo);
    refresh(&mut review);
    let status = |dir: &TempDir| {
        String::from_utf8(
            Command::new("git")
                .args(["status", "--short"])
                .current_dir(dir.path())
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap()
    };

    set_reviewed(&mut review, "new.txt", true);
    refresh(&mut review);
    assert_eq!(review.progress_of("new.txt"), (2, 2));
    assert_eq!(status(&dir), "R  old.txt -> new.txt\n");

    set_reviewed(&mut review, "new.txt", false);
    refresh(&mut review);
    assert_eq!(status(&dir), " D old.txt\n?? new.txt\n");
    assert_eq!(review.files.len(), 2);
}

#[test]
fn tells_new_changed_and_deleted_files_apart_by_color() {
    let (review, dir) = review(
        "status",
        &[
            ("kept.txt", "a\n", "a\n"),
            ("gone.txt", "a\n", "a\n"),
            ("edited.txt", "a\n", "b\n"),
        ],
    );
    drop(review);
    fs::remove_file(dir.path().join("gone.txt")).unwrap();
    dir.write("new.txt", "a\n");
    let mut review = Review::new(Repo::open(dir.path()).unwrap());
    refresh(&mut review);
    let colors = ["new.txt", "edited.txt", "gone.txt", "kept.txt"]
        .map(|path| review.status(path).map(|status| status_color(status, true)));
    assert_eq!(
        colors,
        [
            Some(theme::DARK.added),
            Some(theme::DARK.modified),
            Some(theme::DARK.removed),
            None
        ]
    );
}

const TREE: &[(&str, &str, &str)] = &[
    ("src/a.txt", TEN, "one\n2\n3\n4\n5\n6\n7\n8\nnine\n10\n"),
    ("src/ui/b.txt", "x\n", "y\n"),
    ("srcs.txt", "x\n", "y\n"),
];

#[test]
fn marks_files_and_directories_reviewed_and_counts_their_progress() {
    let (mut review, _dir) = review("directories", TREE);
    review.toggle_reviewed(review.by_path["src/a.txt"], 0);
    assert_eq!(
        [
            review.progress_of("src"),
            review.progress_of("src/ui"),
            review.progress_of("")
        ],
        [(2, 6), (0, 2), (2, 8)]
    );
    set_reviewed(&mut review, "src", true);
    assert_eq!(
        [
            review.progress_of("src"),
            review.progress_of("srcs.txt"),
            review.progress_of("")
        ],
        [(6, 6), (0, 2), (6, 8)]
    );
    set_reviewed(&mut review, "src/ui/b.txt", false);
    refresh(&mut review);
    assert_eq!(
        [review.progress_of("src"), review.progress_of("src/ui")],
        [(4, 6), (0, 2)]
    );
}

#[test]
fn tells_apart_the_changes_made_since_they_were_looked_at() {
    let (mut review, dir) = review("new", TREE);
    assert!(review.unseen.is_empty());
    dir.write("src/a.txt", "one\n2\nthree\n4\n5\n6\n7\n8\nnine\n10\n");
    refresh(&mut review);
    let a = review.by_path["src/a.txt"];
    assert_eq!(review.files[a].new, [false, true, false]);
    assert_eq!(
        review.unseen,
        HashSet::from(["src".to_string(), "src/a.txt".to_string()])
    );
    assert_eq!(review.next(a), None);
    assert_eq!(review.files[a].new, [false; 3]);
    assert!(review.unseen.is_empty());
}

#[test]
fn moves_on_to_the_next_file_to_review_in_the_order_of_the_tree() {
    let (mut review, _dir) = review("next-file", TREE);
    let a = review.by_path["src/a.txt"];
    assert_eq!((review.next(a), review.view(a).current), (None, 1));
    set_reviewed(&mut review, "src/a.txt", true);
    assert_eq!(review.next(a), Some("srcs.txt".to_string()));
    set_reviewed(&mut review, "srcs.txt", true);
    assert_eq!(review.next(a), Some("src/ui/b.txt".to_string()));
    assert_eq!(review.view(review.by_path["src/ui/b.txt"]).current, 0);
    set_reviewed(&mut review, "src/ui", true);
    assert_eq!(review.next(a), None);
}

#[test]
fn marks_a_directory_reviewed_from_its_menu_in_the_tree() {
    let (review, _dir) = review("tree-menu", TREE);
    let mut harness = Harness::new_ui_state(
        |ui, review: &mut Review| {
            review.tree_ui(ui, &mut BTreeSet::new(), None, true);
        },
        review,
    );
    harness.run();
    harness.get_by_label("ui").click_secondary();
    harness.run();
    assert!(harness.query_by_label("Mark as not reviewed").is_none());
    harness.get_by_label("Mark as reviewed").click();
    harness.run();
    assert_eq!(harness.state().progress_of("src"), (2, 6));
}

#[test]
fn rolls_back_a_file_or_a_directory_from_its_menu_in_the_tree() {
    let (review, dir) = review("tree-roll-back", TREE);
    let mut harness = Harness::new_ui_state(
        |ui, review: &mut Review| {
            review.tree_ui(ui, &mut BTreeSet::new(), None, true);
        },
        review,
    );
    harness.run();
    harness.get_by_label("a.txt").click_secondary();
    harness.run();
    harness.get_by_label("Roll back").click();
    harness.run();
    assert_eq!(
        harness.state_mut().take_rolled_back(),
        ["src/a.txt".to_string()]
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("src/a.txt")).unwrap(),
        TEN
    );

    harness.get_by_label("src").click_secondary();
    harness.run();
    harness.get_by_label("Roll back changes").click();
    harness.run();
    assert_eq!(
        harness.state_mut().take_rolled_back(),
        ["src/ui/b.txt".to_string()]
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("src/ui/b.txt")).unwrap(),
        "x\n"
    );

    refresh(harness.state_mut());
    harness.run();
    assert_eq!(harness.state().progress(), (0, 2));
    assert_eq!(harness.state().files[0].diff.path, "srcs.txt".to_string());

    harness.get_by_label("srcs.txt").click_secondary();
    harness.run();
    harness.get_by_label("Roll back").click();
    harness.run();
    assert_eq!(
        fs::read_to_string(dir.path().join("srcs.txt")).unwrap(),
        "x\n"
    );
    refresh(harness.state_mut());
    assert_eq!(harness.state().progress(), (0, 0));
    assert!(harness.state().files.is_empty());
}

#[test]
fn marks_a_file_reviewed_with_its_checkbox_in_the_tree_and_everything_at_once() {
    let (review, _dir) = review("tree-marks", TREE);
    let mut harness = Harness::new_ui_state(
        |ui, review: &mut Review| {
            review.tree_ui(ui, &mut BTreeSet::new(), None, true);
        },
        review,
    );
    harness.run();
    harness.get_by_label("Reviewed: src/ui").click();
    harness.run();
    assert_eq!(harness.state().progress_of("src"), (2, 6));
    harness.get_by_label("Reviewed: src/ui").click();
    harness.run();
    assert_eq!(harness.state().progress_of("src"), (0, 6));

    set_reviewed(harness.state_mut(), "", true);
    assert_eq!(harness.state().progress(), (8, 8));
}

#[test]
fn colors_both_sides_and_marks_what_changed_within_lines() {
    let new = "fn main() {\n    let x = 2;\n}\n";
    let (mut review, _dir) = review("text", &[("a.rs", "fn main() {\n    let y = 1;\n}\n", new)]);
    let text = FileText::load(&review.repo, &review.files[0].diff);
    review.texts.insert("a.rs".to_string(), text);
    review.follow_edits(0, &Buffer::new(new), false);
    let text = &review.texts["a.rs"];
    assert_eq!(text.old.lines, ["fn main() {", "    let y = 1;", "}"]);
    assert_eq!(text.work_lines, ["fn main() {", "    let x = 2;", "}"]);
    assert_eq!(text.changed, [[vec![8..9, 12..13], vec![8..9, 12..13]]]);
    assert!(text.old.spans[1].contains(&(4..7, "keyword")));
}

#[test]
fn draws_only_the_hunks_in_sight() {
    let lines: Vec<String> = (1..=1000).map(|n| format!("line {n}")).collect();
    let changed: Vec<String> = (1..=1000)
        .map(|n| {
            if n % 10 == 1 {
                format!("new {n}")
            } else {
                format!("line {n}")
            }
        })
        .collect();
    let (review, _dir) = review(
        "many-hunks",
        &[(
            "a.txt",
            &(lines.join("\n") + "\n"),
            &(changed.join("\n") + "\n"),
        )],
    );
    let mut harness = show(review, Some("a.txt"), 600.0);
    let drawn = harness.query_all_by_label("Reviewed").count();
    assert!(drawn > 0 && drawn < 20, "{drawn} hunks drawn");
    let is_reviewed = |harness: &Harness<'_, Shown>| {
        harness
            .query_all_by_label("Reviewed")
            .any(|node| node.accesskit_node().toggled() == Some(egui::accesskit::Toggled::True))
    };
    harness.state_mut().review.stage(0, |hunk, _| hunk == 90);
    harness.run();
    assert!(!is_reviewed(&harness));
    harness.state_mut().review.select(0, 90);
    harness.run();
    harness.run();
    assert!(is_reviewed(&harness));
}

#[test]
fn finds_the_unchanged_lines_around_the_hunks() {
    let hunks = &git::parse_diff(DIFF)[0].hunks;
    assert_eq!(
        gaps(hunks, 12),
        [
            Gap {
                lines: 1..1,
                old_offset: 0
            },
            Gap {
                lines: 2..9,
                old_offset: 0
            },
            Gap {
                lines: 10..13,
                old_offset: 0
            }
        ]
    );

    let added =
        git::parse_diff("diff --git a/a b/a\n--- a/a\n+++ b/a\n@@ -2,1 +2,3 @@\n x\n+y\n+z\n");
    assert_eq!(
        gaps(&added[0].hunks, 6),
        [
            Gap {
                lines: 1..2,
                old_offset: 0
            },
            Gap {
                lines: 5..7,
                old_offset: -2
            }
        ]
    );
}

#[test]
fn folds_away_the_unchanged_lines_away_from_the_hunks_unless_expanded() {
    let hunks = &git::parse_diff(DIFF)[0].hunks;
    let gaps = [
        Gap {
            lines: 8..9,
            old_offset: 0,
        },
        Gap {
            lines: 2..12,
            old_offset: 0,
        },
        Gap {
            lines: 13..30,
            old_offset: 0,
        },
    ];
    let unchanged =
        |gap: usize, lines: Range<usize>| lines.map(move |number| Row::Unchanged(gap, number));
    let expected: Vec<Row> = [
        vec![Row::Unchanged(0, 8), Row::Hunk(0, 0), Row::Hunk(0, 1)],
        unchanged(1, 2..5).collect(),
        vec![Row::Folded(1, 4)],
        unchanged(1, 9..12).collect(),
        vec![Row::Hunk(1, 0), Row::Hunk(1, 1)],
        unchanged(2, 13..16).collect(),
        vec![Row::Folded(2, 14)],
    ]
    .into_iter()
    .flatten()
    .collect();
    assert_eq!(
        rows(&gaps, hunks, &[false; 3], &[]),
        (expected, vec![1, 10])
    );
    assert_eq!(
        rows(&gaps, hunks, &[false, true, false], &[]).0[3..13],
        unchanged(1, 2..12).collect::<Vec<_>>()
    );
}

#[test]
fn shows_folded_lines_when_clicked() {
    let lines: Vec<String> = (1..=100).map(|n| format!("line {n}")).collect();
    let text = lines.join("\n") + "\n";
    let (review, _dir) = review(
        "more-lines",
        &[("a.txt", &text, &text.replace("line 50\n", "new 50\n"))],
    );
    let mut harness = show(review, Some("a.txt"), 3000.0);
    harness.get_by_label("Show 47 hidden lines");
    harness.get_by_label("Show 46 hidden lines").click();
    harness.run();
    assert_eq!(harness.state_mut().review.view(0).expanded, [true, false]);
    assert!(harness.query_by_label("Show 46 hidden lines").is_none());
}

#[test]
fn a_review_without_a_file_or_changes_says_so() {
    for (path, label) in [
        (None, "Choose a file under Changes to review it"),
        (Some("gone.txt"), "No changes in gone.txt"),
    ] {
        let (review, _dir) = review("no-file", &[("a.txt", "x\n", "x\n")]);
        show(review, path, 600.0).get_by_label(label);
    }
}

#[test]
fn queues_marks_that_arrive_while_git_is_staging() {
    let (mut review, _dir) = review("queued-marks", TWO_FILES);
    review.set_reviewed("", true, &egui::Context::default());
    review.set_reviewed("", false, &egui::Context::default());
    review.wait_for_staging();
    refresh(&mut review);
    assert_eq!(review.progress(), (0, 6));
}

fn conflict_review(name: &str) -> (Review, TempDir) {
    let (repo, dir) = conflict_repo(name);
    let mut review = Review::new(repo);
    refresh(&mut review);
    (review, dir)
}

fn buffer_text(harness: &mut Harness<'_, Shown>) -> String {
    harness
        .state_mut()
        .files
        .text("a.txt", false)
        .unwrap()
        .0
        .text()
}

#[test]
fn shows_a_conflict_as_a_hunk_that_takes_a_side_or_both() {
    let (review, _dir) = conflict_review("conflict-side");
    assert_eq!(review.conflicts, ["a.txt"]);
    assert_eq!(review.operation, Some(git::Operation::Merge));
    assert_eq!(review.progress(), (0, 0));
    let mut harness = show(review, Some("a.txt"), 600.0);
    harness.get_by_label("1 conflict");
    let block = &harness.state().review.files[0].diff.hunks[0];
    assert_eq!(
        block
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>(),
        ["<<<<<<< HEAD", "ours", "=======", "theirs", ">>>>>>> other"]
    );
    assert_eq!(
        block
            .conflict
            .as_ref()
            .map(|c| (c.ours.as_str(), c.theirs.as_str())),
        Some(("HEAD", "other"))
    );
    harness.get_by_label("Ours").click();
    harness.run();
    harness.run();
    assert_eq!(buffer_text(&mut harness), "one\nours\nthree\n");
    assert!(harness.state().review.files[0].diff.hunks.is_empty());
    assert!(
        harness.state().review.files[0]
            .diff
            .hunks
            .iter()
            .all(|h| h.conflict.is_none())
    );

    let mut harness = show(conflict_review("conflict-theirs").0, Some("a.txt"), 600.0);
    harness.get_by_label("Theirs").click();
    harness.run();
    harness.run();
    assert_eq!(buffer_text(&mut harness), "one\ntheirs\nthree\n");

    let mut harness = show(conflict_review("conflict-both").0, Some("a.txt"), 600.0);
    harness.get_by_label("Both").click();
    harness.run();
    harness.run();
    assert_eq!(buffer_text(&mut harness), "one\nours\ntheirs\nthree\n");
}

#[test]
fn stages_a_resolved_conflict_and_does_not_stage_the_markers() {
    let (mut review, _dir) = conflict_review("conflict-stage");
    review.toggle_reviewed(0, 0);
    assert_eq!(review.repo.changes().unwrap().conflicts, ["a.txt"]);
    set_reviewed(&mut review, "a.txt", true);
    assert_eq!(review.repo.changes().unwrap().conflicts, ["a.txt"]);
    assert!(
        review
            .error
            .as_deref()
            .is_some_and(|error| error.contains("Resolve the conflicts"))
    );

    let mut harness = show(review, Some("a.txt"), 600.0);
    harness.get_by_label("Ours").click();
    harness.run();
    harness.run();
    harness.get_by_label("Mark file reviewed").click();
    harness.run();
    harness.state_mut().review.wait_for_staging();
    refresh(&mut harness.state_mut().review);
    assert!(
        harness
            .state()
            .review
            .repo
            .changes()
            .unwrap()
            .conflicts
            .is_empty()
    );
    assert_eq!(
        harness.state().review.operation,
        Some(git::Operation::Merge)
    );
}

#[test]
fn leaves_conflict_like_lines_alone_when_git_has_no_conflict() {
    let text = "one\n<<<<<<< HEAD\ntwo\n=======\nthree\n>>>>>>> other\n";
    let (review, _dir) = review("markers", &[("a.txt", "one\n", text)]);
    let harness = show(review, Some("a.txt"), 600.0);
    assert!(
        harness.state().review.files[0]
            .diff
            .hunks
            .iter()
            .all(|hunk| hunk.conflict.is_none())
    );
    assert!(harness.query_by_label("Ours").is_none());
}

#[test]
fn sends_a_conflict_or_all_of_them_to_an_agent() {
    let (mut review, _dir) = conflict_review("conflict-agent");
    review.operation = Some(git::Operation::Merge);
    let mut harness = show(review, Some("a.txt"), 600.0);
    harness.get_by_label("Ask agent").click();
    harness.run();
    let Some(Outcome::Send { text, .. }) = &harness.state().asked else {
        panic!("nothing was sent");
    };
    assert!(text.contains("@a.txt"));
    assert!(text.contains("<<<<<<< HEAD"));
    assert!(text.contains("theirs"));

    harness.get_by_label("Send all conflicts").click();
    harness.run();
    let Some(Outcome::Send { text, .. }) = &harness.state().asked else {
        panic!("nothing was sent");
    };
    assert!(text.contains("Resolve the merge conflicts"));
    assert!(text.contains("<<<<<<< HEAD"));
    assert!(text.contains("theirs"));
}

#[test]
fn marks_everything_reviewed_from_the_text_being_edited() {
    let (mut review, dir) = review("mark-edited", &[("a.txt", "one\n", "two\n")]);
    let text = FileText::load(&review.repo, &review.files[0].diff);
    review.texts.insert("a.txt".to_string(), text);
    review.follow_edits(0, &Buffer::new("three\n"), false);
    set_reviewed(&mut review, "", true);
    assert_eq!(review.repo.staged("a.txt").unwrap(), "three\n");
    assert_eq!(
        fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "two\n"
    );
}
