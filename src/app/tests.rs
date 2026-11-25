use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};

use std::path::Path;

use super::bars::TITLE_INSET;
use super::ctl::parse_place;
use super::tabs::TabBehavior;
use super::workspace::Sidebar;
use super::*;
use crate::editor::buffer::Pos;
use crate::git::tests::temp_repo;
use crate::testing::TempDir;

fn harness(name: &str) -> (Harness<'static, App>, TempDir) {
    harness_stepping(name, 0.25)
}

fn harness_stepping(name: &str, step_dt: f32) -> (Harness<'static, App>, TempDir) {
    let (repo, dir) = temp_repo(
        name,
        &[
            ("src/main.rs", "fn main() {}\n"),
            ("README.md", "# Readme\n"),
        ],
    );
    let snapshot = Snapshot::take(&repo);
    let mut app = App::with_settings(
        repo,
        Workspace::default(),
        Settings::default(),
        None,
        egui::Context::default(),
    );
    app.content
        .terminals
        .keep_images_in(dir.path().join("images"));
    app.apply(snapshot);
    let mut harness = Harness::builder()
        .with_step_dt(step_dt)
        .with_size(egui::vec2(1000.0, 700.0))
        .with_context_setup(|ctx| theme::apply(ctx, false))
        .build_ui_state(|ui, app: &mut App| app.show(ui), app);
    harness.run();
    (harness, dir)
}

#[test]
fn switches_between_the_bundled_and_the_system_fonts() {
    let (mut harness, _dir) = harness("fonts");
    let first_font = |harness: &Harness<'_, App>| {
        harness
            .ctx
            .fonts(|fonts| fonts.definitions().families[&egui::FontFamily::Proportional][0].clone())
    };
    assert_eq!(first_font(&harness), "bundled-ui");
    assert!(!harness.state().settings.system_fonts);

    harness.get_by_label("Keyboard shortcuts").click();
    harness.run();
    harness.get_by_label("Use the system fonts").click();
    harness.run();
    assert!(harness.state().settings.system_fonts);
    assert_ne!(first_font(&harness), "bundled-ui");

    harness.get_by_label("Use the system fonts").click();
    harness.run();
    assert!(!harness.state().settings.system_fonts);
    assert_eq!(first_font(&harness), "bundled-ui");
}

#[test]
fn windows_are_titled_with_the_name_of_the_editor_and_the_project() {
    let workspace = Workspace::default();
    let builder = viewport(Path::new("/work/shop"), &workspace.windows[0]);
    assert_eq!(builder.title.as_deref(), Some("Shepherd — /work/shop"));
}

#[test]
fn shows_the_branch_in_the_title_bar_on_every_sidebar() {
    let (mut harness, dir) = harness("title-branch");
    crate::git::tests::commit_all(&Repo::open(dir.path()).unwrap());
    harness.state().watcher.refresh_now();
    let branch = Repo::open(dir.path()).unwrap().branch();
    for (sidebar, first) in [(Sidebar::Files, "README.md"), (Sidebar::Changes, "Commit")] {
        harness.state_mut().workspace.sidebar = sidebar;
        harness.run();
        let button = harness.get_by_label(branch.as_str());
        assert!(button.rect().bottom() < harness.get_by_label(first).rect().top());
        assert!(button.rect().left() >= TITLE_INSET as f32);
        for sync in ["↓", "↑"] {
            let sync = harness.get_by_label(sync).rect();
            assert!(sync.left() >= button.rect().right());
            assert!(sync.bottom() < harness.get_by_label(first).rect().top());
        }
    }
    harness.get_by_label(branch.as_str()).click();
    harness.run();
    harness.get_by_label("Create branch");
}

#[test]
fn the_sidebar_keeps_its_width_when_there_are_no_changes() {
    let (mut harness, dir) = harness("sidebar-width");
    crate::git::tests::commit_all(&Repo::open(dir.path()).unwrap());
    harness.state().watcher.refresh_now();
    let width = |harness: &Harness<'static, App>| harness.get_by_label("README.md").rect().width();
    let before = width(&harness);

    harness.get_by_label("Changes").click();
    wait_for(&mut harness, "No changes");
    for _ in 0..10 {
        harness.run();
    }
    harness.get_by_label("Files").click();
    for _ in 0..10 {
        harness.run();
    }
    assert_eq!(width(&harness), before);
}

#[test]
fn opens_the_history_of_the_branch_and_reviews_a_commit() {
    let (mut harness, dir) = harness("history-open");
    let repo = Repo::open(dir.path()).unwrap();
    crate::git::tests::commit_all(&repo);
    dir.write("README.md", "# Changed\n");
    dir.write("src/main.rs", "fn main() { println!(); }\n");
    repo.stage_file("README.md", true).unwrap();
    repo.stage_file("src/main.rs", true).unwrap();
    repo.commit("change readme").unwrap();
    harness.state().watcher.refresh_now();

    harness.get_by_label("History").click();
    harness.run();
    wait_for(&mut harness, "change readme");
    harness.get_by_label("change readme").click();
    harness.run();

    let tabs = open_tabs(&harness);
    let (first, range) = match &tabs[0][0] {
        Tab::CommitReview {
            path: Some(path),
            range,
            ..
        } => (path.clone(), range.clone()),
        _ => panic!("no commit review"),
    };

    harness.key_press(Key::N);
    harness.run();
    match &open_tabs(&harness)[0][0] {
        Tab::CommitReview {
            path: Some(path),
            range: shown,
            ..
        } => {
            assert_eq!(*shown, range);
            assert_ne!(*path, first);
        }
        _ => panic!("the review of the commit went away"),
    }
}

#[test]
fn shows_a_file_history_and_turns_git_blame_on_and_off() {
    let (mut harness, dir) = harness("file-history");
    let repo = Repo::open(dir.path()).unwrap();
    crate::git::tests::commit_all(&repo);
    dir.write("README.md", "# Changed\n");
    repo.stage_file("README.md", true).unwrap();
    repo.commit("change readme").unwrap();
    harness.state().watcher.refresh_now();
    harness.run();
    let ctx = harness.ctx.clone();

    harness
        .state_mut()
        .perform(Action::FileHistory("README.md".to_string()), &ctx);
    harness.run();
    assert_eq!(harness.state().workspace.sidebar, Sidebar::History);
    assert!(harness.state().workspace.sidebar_open);
    wait_for(&mut harness, "change readme");
    let heading = harness.get_by_label("History of").rect();
    let path = harness.get_by_label("README.md").rect();
    let all = harness.get_by_label("All commits").rect();
    assert!(path.top() < heading.bottom() && heading.top() < path.bottom());
    assert!(all.top() < heading.bottom() && heading.top() < all.bottom());
    assert!(all.left() > path.right());

    harness.state_mut().action = Some(Action::Open(Tab::File("README.md".to_string()), 0));
    harness.run();
    for _ in 0..3 {
        harness.run();
    }

    harness.get_by_label("Editor").click_secondary();
    harness.run();
    harness.get_by_label("Annotate with Git Blame").click();
    harness.run();
    assert!(harness.state().content.open_files.blame_on("README.md"));

    let hash = repo.log(0, crate::git::PAGE).unwrap()[0].hash.clone();
    let editor = harness.get_by_label("Editor").rect();
    harness.event(egui::Event::PointerMoved(egui::pos2(
        editor.left() + 20.0,
        editor.top() + 8.0,
    )));
    harness.run_steps(4);
    assert!(harness.query_by_label(&hash).is_some());

    harness.get_by_label("Editor").click();
    harness.run();
    harness.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::B);
    harness.run();
    assert!(!harness.state().content.open_files.blame_on("README.md"));
    harness.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::B);
    harness.run();
    assert!(harness.state().content.open_files.blame_on("README.md"));

    let pos = egui::pos2(editor.left() + 20.0, editor.top() + 8.0);
    harness.event(egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    harness.event(egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    harness.run();
    assert_eq!(harness.state().workspace.sidebar, Sidebar::History);
    assert!(harness.state().workspace.sidebar_open);
    let (range, _) = harness.state().content.history.selected().unwrap();
    assert_eq!(range.head, hash);
}

#[test]
fn shows_changed_and_new_lines_in_a_file_view() {
    let (mut harness, dir) = harness("file-changes");
    let repo = Repo::open(dir.path()).unwrap();
    dir.write("README.md", "one\ntwo\nthree\n");
    crate::git::tests::commit_all(&repo);
    dir.write("README.md", "one\nTWO\nthree\nnew\n");
    harness.state_mut().action = Some(Action::Open(Tab::File("README.md".to_string()), 0));
    harness.run();

    harness.get_by_label("Changed line 2");
    harness.get_by_label("New line 4");
    harness.get_by_label("Changed line 2").hover();
    harness.run_steps(4);
    assert!(harness.query_by_label("Diff of 2 lines").is_some());

    harness.get_by_label("Changed line 2").click();
    harness.run();
    harness.event(egui::Event::PointerMoved(egui::pos2(10.0, 10.0)));
    harness.run_steps(4);
    assert!(harness.query_by_label("Diff of 2 lines").is_some());
    harness.key_press(Key::Escape);
    harness.run();
    assert!(harness.query_by_label("Diff of 2 lines").is_none());

    repo.stage_file("README.md", true).unwrap();
    repo.commit("commit the changes").unwrap();
    harness
        .state_mut()
        .apply(Ok(Snapshot::take(&repo).unwrap()));
    harness.run();
    assert!(harness.query_by_label("Changed line 2").is_none());
    assert!(harness.query_by_label("New line 4").is_none());
}

#[test]
fn opens_and_closes_the_sidebar_from_the_bar_of_icons_and_shows_the_keys() {
    let (mut harness, _dir) = harness("activity-bar");
    harness.get_by_label("README.md");

    harness.get_by_label("Files").click();
    for _ in 0..10 {
        harness.run();
    }
    assert!(!harness.state().workspace.sidebar_open);
    assert!(harness.query_by_label("README.md").is_none());

    harness.get_by_label("Changes").click();
    harness.run();
    assert!(harness.state().workspace.sidebar_open);
    assert_eq!(harness.state().workspace.sidebar, Sidebar::Changes);

    harness.get_by_label("Keyboard shortcuts").click();
    harness.run();
    harness.get_by_label("Next unreviewed hunk");
    harness.key_press(Key::Escape);
    harness.run();
    assert!(harness.query_by_label("Next unreviewed hunk").is_none());

    harness.key_press_modifiers(Modifiers::COMMAND, Key::Slash);
    harness.run();
    harness.get_by_label("Next unreviewed hunk");
}

fn open_tabs(harness: &Harness<'_, App>) -> Vec<Vec<Tab>> {
    harness
        .state()
        .workspace
        .windows
        .iter()
        .map(|window| window.tabs().into_iter().cloned().collect())
        .collect()
}

#[test]
fn resolves_a_conflict_in_the_review_and_continues_the_merge() {
    let (repo, dir) = crate::git::tests::conflict_repo("app-conflict");
    let snapshot = Snapshot::take(&repo);
    let mut app = App::with_settings(
        repo,
        Workspace::default(),
        Settings::default(),
        None,
        egui::Context::default(),
    );
    app.content
        .terminals
        .keep_images_in(dir.path().join("images"));
    app.apply(snapshot);
    let mut harness = Harness::builder()
        .with_step_dt(0.25)
        .with_size(egui::vec2(1000.0, 700.0))
        .with_context_setup(|ctx| theme::apply(ctx, false))
        .build_ui_state(|ui, app: &mut App| app.show(ui), app);
    harness.run();

    harness.get_by_label("Changes").click();
    harness.run();
    harness.get_by_label("a.txt").click();
    harness.run();
    harness.get_by_label("1 conflict");
    harness.get_by_label("Ours").click();
    harness.run();
    harness.run();
    harness.get_by_label("Mark file reviewed").click();
    for _ in 0..400 {
        harness.step();
        if !harness
            .get_by_label("Continue")
            .accesskit_node()
            .is_disabled()
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    harness.get_by_label("Continue").click();
    for _ in 0..400 {
        harness.step();
        if harness.query_by_label("✓ Continued").is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    harness.get_by_label("✓ Continued");
    assert_eq!(Repo::open(dir.path()).unwrap().status().behind, 0);
}

fn wait_for(harness: &mut Harness<'_, App>, label: &str) {
    for _ in 0..200 {
        harness.run();
        if harness.query_by_label(label).is_some() {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("{label:?} didn't show up");
}

fn wait_for_folds(harness: &mut Harness<'_, App>) -> usize {
    let mut folds = 0;
    for _ in 0..200 {
        harness.run();
        folds = harness.query_all_by_label_contains("hidden lines").count();
        if folds >= 2 {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    folds
}

fn file(path: &str) -> Tab {
    Tab::File(path.to_string())
}

#[test]
fn opens_a_file_from_the_tree_as_a_tab_next_to_the_review() {
    let (mut harness, _dir) = harness("open-from-tree");
    harness.get_by_label("Review");
    harness.get_by_label("src").click();
    harness.run();
    harness.get_by_label("main.rs").click();
    harness.run();

    assert_eq!(
        open_tabs(&harness),
        [vec![Tab::Review(None), file("src/main.rs")]]
    );
    assert_eq!(harness.query_all_by_label("main.rs").count(), 2);
}

#[test]
fn opens_a_file_by_name() {
    let (mut harness, _dir) = harness("quick-open");
    harness.key_press_modifiers(Modifiers::COMMAND, Key::P);
    harness.run();
    harness.get_by_role(egui::Role::TextInput).type_text("read");
    harness.run();
    harness.key_press(Key::Enter);
    harness.run();

    assert_eq!(
        open_tabs(&harness),
        [vec![Tab::Review(None), file("README.md")]]
    );
    assert!(harness.state().quick_open.is_none());
}

fn review(path: &str) -> Tab {
    Tab::Review(Some(path.to_string()))
}

#[test]
fn shows_the_changes_of_a_file_chosen_in_the_tree_of_changes_in_the_review_tab() {
    let (mut harness, _dir) = harness("changes");
    harness.get_by_label("Changes").click();
    harness.run();
    harness.get_by_label("main.rs").click();
    harness.run();
    assert_eq!(open_tabs(&harness), [vec![review("src/main.rs")]]);
    harness.get_by_label("Review: main.rs");

    harness.get_by_label("README.md").click();
    harness.run();
    assert_eq!(open_tabs(&harness), [vec![review("README.md")]]);

    harness.state_mut().action = None;
    harness
        .get_by_label("main.rs")
        .click_modifiers(Modifiers::COMMAND);
    harness.run();
    assert_eq!(
        open_tabs(&harness),
        [vec![review("README.md"), review("src/main.rs")]]
    );

    harness.state_mut().action = Some(Action::NewReview(0, None));
    harness.run();
    harness.get_by_label("Choose a file under Changes to review it");
    harness.get_by_label("README.md").click();
    harness.run();
    assert_eq!(
        open_tabs(&harness),
        [vec![
            review("README.md"),
            review("src/main.rs"),
            Tab::Review(None)
        ]]
    );
}

#[test]
fn moves_the_review_on_to_the_next_file_once_a_file_is_reviewed() {
    let (mut harness, _dir) = harness("review-moves-on");
    harness.get_by_label("Changes").click();
    harness.run();
    harness.get_by_label("README.md").click();
    harness.run();
    harness.key_press(Key::Space);
    harness.run();
    assert_eq!(open_tabs(&harness), [vec![review("src/main.rs")]]);
    assert_eq!(harness.state().content.review.progress(), (1, 2));
}

#[test]
fn jumps_from_a_file_to_its_changes_and_back() {
    let (mut harness, _dir) = harness("jump");
    harness.state_mut().action = Some(Action::Open(file("src/main.rs"), 0));
    harness.run();
    harness.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::R);
    harness.run();
    assert_eq!(
        open_tabs(&harness),
        [vec![review("src/main.rs"), file("src/main.rs")]]
    );
    assert!(
        harness.ctx.memory(|m| m.focused().is_none()),
        "the keys are for the review"
    );

    harness.key_press(Key::O);
    harness.run();
    assert!(
        harness.ctx.memory(|m| m.focused().is_some()),
        "the keys are for the file"
    );
    assert_eq!(
        harness
            .state()
            .content
            .open_files
            .selection()
            .map(|selection| (selection.path, selection.start)),
        Some(("src/main.rs".to_string(), Pos::new(0, 0)))
    );
}

#[test]
fn shows_more_of_a_changed_file_than_its_changes_when_asked() {
    let (mut harness, dir) = harness("review-context");
    let lines: Vec<String> = (1..=60).map(|n| format!("line {n}")).collect();
    fs::write(dir.path().join("README.md"), lines.join("\n") + "\n").unwrap();
    crate::git::tests::commit_all(&Repo::open(dir.path()).unwrap());
    fs::write(
        dir.path().join("README.md"),
        lines.join("\n").replace("line 30\n", "changed\n") + "\n",
    )
    .unwrap();
    harness.state_mut().action = Some(Action::Review {
        path: "README.md".to_string(),
        new_tab: false,
    });
    harness.state().watcher.refresh_now();
    let folds = wait_for_folds(&mut harness);
    assert!(folds >= 2);
    harness.get_by_label("Review: README.md");
    harness
        .query_all_by_label_contains("hidden lines")
        .next()
        .unwrap()
        .click();
    harness.run();
    assert!(harness.query_all_by_label_contains("hidden lines").count() < folds);
}

#[test]
fn pops_a_tab_out_into_its_own_window_and_brings_it_back_when_the_window_is_emptied() {
    let (mut harness, _dir) = harness("pop-out");
    harness.state_mut().action = Some(Action::Open(file("README.md"), 0));
    harness.run();
    let tile = harness.state().workspace.windows[0]
        .tree
        .tiles
        .find_pane(&file("README.md"))
        .unwrap();
    harness.state_mut().action = Some(Action::Move {
        from: 0,
        tile,
        to: None,
    });
    harness.run();

    assert_eq!(
        open_tabs(&harness),
        [vec![Tab::Review(None)], vec![file("README.md")]]
    );
    harness.get_by_label("Review");
    assert_eq!(
        harness
            .query_all_by_role_and_label(egui::Role::Button, "README.md")
            .count(),
        2
    );

    let tile = harness.state().workspace.windows[1]
        .tree
        .tiles
        .find_pane(&file("README.md"))
        .unwrap();
    harness.state_mut().action = Some(Action::Move {
        from: 1,
        tile,
        to: Some(0),
    });
    harness.run();
    assert_eq!(open_tabs(&harness).len(), 1);
}

#[test]
fn drops_a_dragged_tab_into_the_tabs_under_the_pointer_or_into_a_new_window() {
    let (mut harness, _dir) = harness("drop");
    harness.state_mut().action = Some(Action::Open(file("README.md"), 0));
    harness.run();
    let window = &mut harness.state_mut().workspace.windows[0];
    let on_screen = egui::vec2(100.0, 50.0);
    window.screen_rect = Some(egui::Rect::from_min_size(
        on_screen.to_pos2(),
        egui::vec2(1000.0, 700.0),
    ));
    let tile = window.tree.tiles.find_pane(&file("README.md")).unwrap();

    let point = egui::pos2(1500.0, 300.0);
    harness.state_mut().action = Some(Action::Drop(Drag {
        from: 0,
        tile,
        point,
    }));
    harness.run();
    assert_eq!(
        open_tabs(&harness),
        [vec![Tab::Review(None)], vec![file("README.md")]]
    );

    let windows = &harness.state().workspace.windows;
    let review = windows[0].tree.tiles.find_pane(&Tab::Review(None)).unwrap();
    let point = windows[0].tree.tiles.rect(review).unwrap().center() + on_screen;
    let tile = windows[1].tree.tiles.find_pane(&file("README.md")).unwrap();
    harness.state_mut().action = Some(Action::Drop(Drag {
        from: 1,
        tile,
        point,
    }));
    harness.run();
    assert_eq!(
        open_tabs(&harness),
        [vec![Tab::Review(None), file("README.md")]]
    );
    let tiles = &harness.state().workspace.windows[0].tree.tiles;
    assert_eq!(
        tiles.parent_of(tiles.find_pane(&file("README.md")).unwrap()),
        tiles.parent_of(review)
    );
}

#[test]
fn drops_a_dragged_tab_on_the_side_of_the_tabs_under_the_pointer_as_a_split() {
    let (mut harness, _dir) = harness("drop-split");
    let window = &mut harness.state_mut().workspace.windows[0];
    window.screen_rect = Some(egui::Rect::from_min_size(
        egui::Pos2::ZERO,
        egui::vec2(1000.0, 700.0),
    ));
    for (name, towards) in [
        ("README.md", egui::vec2(0.4, 0.0)),
        ("src/main.rs", egui::vec2(0.0, 0.4)),
    ] {
        harness.state_mut().action = Some(Action::Open(file(name), 0));
        harness.run();
        let tile = harness.state().workspace.windows[0]
            .tree
            .tiles
            .find_pane(&file(name))
            .unwrap();
        harness.state_mut().action = Some(Action::Move {
            from: 0,
            tile,
            to: None,
        });
        harness.run();

        let windows = &harness.state().workspace.windows;
        let review = windows[0].tree.tiles.find_pane(&Tab::Review(None)).unwrap();
        let rect = windows[0].tree.tiles.rect(review).unwrap();
        let point = rect.center() + towards * rect.size();
        let tile = windows[1].tree.tiles.find_pane(&file(name)).unwrap();
        harness.state_mut().action = Some(Action::Drop(Drag {
            from: 1,
            tile,
            point,
        }));
        harness.run();
    }

    assert_eq!(
        open_tabs(&harness),
        [vec![
            Tab::Review(None),
            file("README.md"),
            file("src/main.rs")
        ]]
    );
    let tiles = &harness.state().workspace.windows[0].tree.tiles;
    let rect = |tab: &Tab| tiles.rect(tiles.find_pane(tab).unwrap()).unwrap();
    let (review, readme, main) = (
        rect(&Tab::Review(None)),
        rect(&file("README.md")),
        rect(&file("src/main.rs")),
    );
    assert!(review.right() <= readme.left() && review.top() == readme.top());
    assert!(
        review.bottom() <= main.top()
            && review.left() == main.left()
            && main.right() <= readme.left()
    );
}

#[test]
fn shows_the_title_of_a_dragged_tab_at_the_pointer() {
    let (mut harness, _dir) = harness("drag-title");
    harness.state_mut().action = Some(Action::Open(file("README.md"), 0));
    harness.run();
    let titles = |harness: &Harness<'_, App>| harness.query_all_by_label("README.md").count();
    let before = titles(&harness);

    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    let tab = harness
        .query_all_by_label("README.md")
        .last()
        .unwrap()
        .rect()
        .center();
    let pointer = tab + egui::vec2(100.0, 200.0);
    harness.event(egui::Event::PointerMoved(tab));
    harness.event(button(tab, true));
    harness.run();
    harness.event(egui::Event::PointerMoved(pointer));
    harness.run();
    assert_eq!(titles(&harness), before + 1);

    harness.key_press(Key::Escape);
    harness.run();
    assert_eq!(titles(&harness), before);
    harness.event(button(pointer, false));
    harness.run();
    assert_eq!(
        open_tabs(&harness),
        [vec![Tab::Review(None), file("README.md")]]
    );
}

#[test]
fn points_at_a_tab_and_grabs_it_only_while_it_is_held() {
    let (mut harness, _dir) = harness("tab-cursor");
    harness.state_mut().action = Some(Action::Open(file("README.md"), 0));
    harness.run();
    let cursor = |harness: &Harness<'_, App>| harness.output().platform_output.cursor_icon;
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    let tab = harness
        .query_all_by_label("README.md")
        .last()
        .unwrap()
        .rect()
        .center();
    harness.event(egui::Event::PointerMoved(tab));
    harness.run();
    assert_eq!(cursor(&harness), egui::CursorIcon::PointingHand);
    harness.event(button(tab, true));
    harness.run();
    assert_eq!(cursor(&harness), egui::CursorIcon::Grabbing);
    harness.event(button(tab, false));
    harness.run();
    assert_eq!(cursor(&harness), egui::CursorIcon::PointingHand);
}

#[test]
fn edits_an_opened_file_with_the_keyboard_and_saves_it() {
    let (mut harness, dir) = harness("edit");
    let on_disk = || fs::read_to_string(dir.path().join("README.md")).unwrap();
    harness.state_mut().action = Some(Action::Open(file("README.md"), 0));
    harness.run();
    harness.event(egui::Event::Text("Hello".to_string()));
    harness.key_press(Key::Enter);
    harness.run();
    harness.key_press_modifiers(Modifiers::COMMAND, Key::S);
    harness.run();
    assert_eq!(on_disk(), "Hello\n# Readme\n");

    harness.event(egui::Event::Text("j".to_string()));
    harness.key_press(Key::J);
    harness.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    harness.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    harness.run();
    harness.state_mut().content.open_files.save_all();
    assert_eq!(on_disk(), "Hello# Readme\n");
}

#[test]
fn places_the_cursor_with_the_mouse() {
    let (mut harness, dir) = harness("edit-mouse");
    harness.state_mut().action = Some(Action::Open(file("README.md"), 0));
    harness.run();
    let editor = harness.get_by_role(egui::Role::MultilineTextInput).rect();
    let end_of_line = egui::pos2(editor.right() - 20.0, editor.top() + 5.0);
    harness.event(egui::Event::PointerMoved(end_of_line));
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton {
            pos: end_of_line,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
    }
    harness.run();
    harness.event(egui::Event::Text("!".to_string()));
    harness.run();
    harness.state_mut().content.open_files.save_all();
    assert_eq!(
        fs::read_to_string(dir.path().join("README.md")).unwrap(),
        "# Readme!\n"
    );
}

#[test]
fn edits_a_highlighted_file() {
    let (mut harness, dir) = harness("edit-highlighted");
    harness.state_mut().action = Some(Action::Open(file("src/main.rs"), 0));
    harness.run();
    for text in ["// one", "\n", "let s = \"two", "\n\n", "\";"] {
        harness.event(egui::Event::Text(text.to_string()));
        harness.run();
    }
    harness.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    harness.run();
    harness.state_mut().content.open_files.save_all();
    assert_eq!(
        fs::read_to_string(dir.path().join("src/main.rs")).unwrap(),
        "// one\nlet s = \"twofn main() {}\n"
    );
}

#[test]
fn keeps_the_line_numbers_over_text_scrolled_sideways() {
    let (mut harness, dir) = harness("gutter");
    let long_line = "x".repeat(300);
    fs::write(dir.path().join("README.md"), &long_line).unwrap();
    harness.state_mut().action = Some(Action::Open(file("README.md"), 0));
    harness.run();
    harness.key_press(Key::End);
    harness.run();

    let pane_left = harness
        .get_by_role_and_label(egui::Role::Button, "Review")
        .rect()
        .left();
    let editor = harness.get_by_role(egui::Role::MultilineTextInput).rect();
    let gutter = egui::pos2(pane_left + 10.0, editor.top() + 5.0);
    harness.event(egui::Event::PointerMoved(gutter));
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton {
            pos: gutter,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
    }
    harness.run();
    harness.event(egui::Event::Text("!".to_string()));
    harness.run();
    harness.state_mut().content.open_files.save_all();
    let saved = fs::read_to_string(dir.path().join("README.md")).unwrap();
    let at = saved.find('!').unwrap();
    assert!(at > 100 && at < 300, "typed at {at}");
}

#[test]
fn finds_and_replaces_text_in_a_file() {
    let (mut harness, dir) = harness("find");
    fs::write(dir.path().join("README.md"), "one two\nTwo one\n").unwrap();
    harness.state_mut().action = Some(Action::Open(file("README.md"), 0));
    harness.run();
    harness.key_press_modifiers(Modifiers::COMMAND, Key::F);
    harness.run();
    harness.event(egui::Event::Text("two".to_string()));
    harness.run();
    harness.get_by_label("1/2");
    harness.key_press(Key::Enter);
    harness.run();
    harness.get_by_label("2/2");

    harness
        .query_all_by_role(egui::Role::TextInput)
        .last()
        .unwrap()
        .click();
    harness.run();
    harness.event(egui::Event::Text("2".to_string()));
    harness.run();
    harness.get_by_label("Replace").click();
    harness.run();
    harness.get_by_label("1/1");

    harness.key_press(Key::Escape);
    harness.run();
    harness.event(egui::Event::Text("3".to_string()));
    harness.run();
    harness.state_mut().content.open_files.save_all();
    assert_eq!(
        fs::read_to_string(dir.path().join("README.md")).unwrap(),
        "one 3\n2 one\n"
    );
}

#[test]
fn searches_the_project_and_opens_a_place_found() {
    let (mut harness, dir) = harness("project-search");
    fs::write(
        dir.path().join("src/main.rs"),
        "fn main() {\n    println!(\"hello\");\n}\n",
    )
    .unwrap();
    let readme = Reveal {
        path: "README.md".to_string(),
        start: Pos::new(0, 2),
        end: Pos::new(0, 8),
        focus: true,
    };
    harness.state_mut().action = Some(Action::Reveal(readme));
    harness.run();
    harness.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::F);
    harness.run();
    assert_eq!(
        harness
            .get_by_role(egui::Role::TextInput)
            .value()
            .as_deref(),
        Some("Readme")
    );
    wait_for(&mut harness, "1 found in 1 files");

    let search_for = |harness: &mut Harness<'_, App>, query: &str, expected: &str| {
        harness.key_press_modifiers(Modifiers::COMMAND, Key::A);
        harness.event(egui::Event::Text(query.to_string()));
        wait_for(harness, expected);
    };
    search_for(&mut harness, "HELLO", "0 found in 0 files");
    search_for(&mut harness, "hello", "1 found in 1 files");
    harness.key_press(Key::ArrowDown);
    harness.run();
    assert_eq!(
        open_tabs(&harness),
        [vec![
            Tab::Review(None),
            file("README.md"),
            file("src/main.rs")
        ]]
    );
    assert!(harness.get_by_role(egui::Role::TextInput).is_focused());
    harness.key_press(Key::Enter);
    harness.run();
    assert!(harness.state().search_in.is_none());

    harness.event(egui::Event::Text("bye".to_string()));
    harness.run();
    harness.state_mut().content.open_files.save_all();
    assert_eq!(
        fs::read_to_string(dir.path().join("src/main.rs")).unwrap(),
        "fn main() {\n    println!(\"bye\");\n}\n"
    );
}

#[test]
fn opens_the_search_in_files_with_shift_pressed_twice() {
    let (mut harness, _dir) = harness_stepping("double-shift", 0.05);
    let shift = |harness: &mut Harness<'_, App>, modifiers: Modifiers| {
        harness.event(egui::Event::Key {
            key: Key::ShiftLeft,
            physical_key: Some(Key::ShiftLeft),
            pressed: modifiers.shift,
            repeat: false,
            modifiers,
        });
        harness.event(egui::Event::ModifiersChanged(modifiers));
        harness.step();
    };
    for _ in 0..2 {
        shift(&mut harness, Modifiers::SHIFT);
        shift(&mut harness, Modifiers::NONE);
    }
    harness.run();
    assert!(harness.state().search_in.is_some());
    assert!(harness.get_by_role(egui::Role::TextInput).is_focused());
    harness.key_press(Key::Escape);
    harness.run();
    assert!(harness.state().search_in.is_none());
}

#[test]
fn opens_the_quick_chat_with_control_pressed_twice_and_starts_an_agent_tab() {
    use std::os::unix::fs::PermissionsExt as _;

    let (mut harness, dir) = harness_stepping("double-control", 0.05);
    let program = dir.write("agent", "#!/bin/sh\necho \"args: $@\"\nsleep 5\n");
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    harness
        .state_mut()
        .content
        .terminals
        .run_agents_with(&program);
    harness.state_mut().agents = vec![Agent::Claude];
    harness.state_mut().models =
        HashMap::from([(Agent::Claude, crate::agents::tests::claude_sample())]);
    harness.run();

    let ctrl = |harness: &mut Harness<'_, App>, modifiers: Modifiers| {
        harness.event(egui::Event::Key {
            key: Key::ControlLeft,
            physical_key: Some(Key::ControlLeft),
            pressed: modifiers.ctrl,
            repeat: false,
            modifiers,
        });
        harness.event(egui::Event::ModifiersChanged(modifiers));
        harness.step();
    };
    for _ in 0..2 {
        ctrl(&mut harness, Modifiers::CTRL);
        ctrl(&mut harness, Modifiers::NONE);
    }
    harness.run();
    assert!(harness.state().quick_chat.is_some());
    harness.get_by_label("Default model").click();
    harness.run();
    harness.get_by_label("Haiku").click();
    harness.run();
    harness.get_by_label("Default effort").click();
    harness.run();
    harness.get_by_label("high").click();
    harness.run();
    harness.event(egui::Event::Text("hello agent ".to_string()));
    harness.run();
    harness.event(egui::Event::PasteImage(std::sync::Arc::new(
        egui::ColorImage::new([1, 1], vec![egui::Color32::WHITE]),
    )));
    harness.run();
    harness.key_press(Key::Enter);
    harness.run();
    assert!(harness.state().quick_chat.is_none());

    let images: Vec<PathBuf> = fs::read_dir(dir.path().join("images"))
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .collect();
    assert_eq!(images.len(), 1);

    let tabs = open_tabs(&harness);
    assert!(tabs[0].iter().any(|tab| matches!(
        tab,
        Tab::Agent {
            agent: Agent::Claude,
            ..
        }
    )));
    let id = tabs[0]
        .iter()
        .find_map(|tab| match tab {
            Tab::Agent { id, tuning, .. } => {
                assert_eq!(tuning.model(), Some("haiku"));
                assert_eq!(tuning.effort(), Some("high"));
                Some(*id)
            }
            _ => None,
        })
        .unwrap();
    let start = Instant::now();
    let path = images[0].display().to_string();
    let flat_path = path.split_whitespace().collect::<String>();
    while !harness
        .state()
        .content
        .terminals
        .screen(id)
        .is_some_and(|screen| {
            let flat = screen.split_whitespace().collect::<String>();
            flat.contains("helloagent") && flat.contains(&flat_path) && flat.contains("haiku")
        })
    {
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "the agent shows:\n{}",
            harness
                .state()
                .content
                .terminals
                .screen(id)
                .unwrap_or_default()
        );
        harness.run();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(images[0].exists());
}

#[test]
fn drafts_a_commit_message_in_the_background_with_its_own_model() {
    use std::os::unix::fs::PermissionsExt as _;

    let (mut harness, dir) = harness("draft-background");
    let arguments = dir.path().join("draft-args");
    let program = dir.write(
        "agent",
        format!(
            "#!/bin/sh\necho \"$@\" > '{}'\nsleep 1\n",
            arguments.display()
        ),
    );
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    harness
        .state_mut()
        .content
        .terminals
        .run_agents_with(&program);
    harness.state_mut().agents = vec![Agent::Claude];
    harness.state_mut().workspace.draft_agent = crate::agents::Choice {
        agent: Some(Agent::Claude),
        tunings: HashMap::from([(
            Agent::Claude,
            Tuning {
                model: Some("haiku".to_string()),
                effort: Some("max".to_string()),
            },
        )]),
    };
    harness.state_mut().action = Some(Action::Draft {
        text: "write it".to_string(),
    });
    harness.run();
    assert_eq!(open_tabs(&harness), [vec![Tab::Review(None)]]);

    let start = Instant::now();
    loop {
        if let Ok(args) = fs::read_to_string(&arguments) {
            assert!(args.contains("-p"), "{args}");
            assert!(args.contains("--model haiku --effort max"), "{args}");
            assert!(args.contains("write it"), "{args}");
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn opens_an_agent_from_the_plus_of_a_tab_bar_and_lists_it_apart_from_terminals() {
    use std::os::unix::fs::PermissionsExt as _;

    let (mut harness, dir) = harness("agent-tab");
    let program = dir.write("agent", "#!/bin/sh\necho \"args: $@\"\nsleep 5\n");
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    harness
        .state_mut()
        .content
        .terminals
        .run_agents_with(&program);
    harness.state_mut().agents = vec![Agent::Claude];
    harness.run();

    harness.get_by_label("+").click();
    harness.run();
    harness.get_by_label("New Claude Code agent").click();
    harness.run();
    let tabs = open_tabs(&harness);
    assert_eq!(tabs.len(), 1);
    assert_eq!(tabs[0].len(), 2);
    assert!(matches!(
        tabs[0][1],
        Tab::Agent {
            id: 1,
            agent: Agent::Claude,
            ..
        }
    ));

    let context = ask(&mut harness, &["context"]).unwrap();
    assert!(context.contains("Terminals: none"), "{context}");
    assert!(context.contains("- agent 1: Claude Code"), "{context}");
}

#[test]
fn runs_a_program_in_a_new_terminal_that_takes_the_keyboard() {
    let (mut harness, _dir) = harness("terminal-tab");
    harness.state_mut().content.terminals.run("/bin/cat", &[]);
    harness.key_press_modifiers(Modifiers::COMMAND, Key::T);
    harness.run();
    assert_eq!(
        open_tabs(&harness),
        [vec![Tab::Review(None), Tab::Terminal(1)]]
    );
    assert!(harness.get_by_role(egui::Role::Terminal).is_focused());

    harness.event(egui::Event::Text("j hello".to_string()));
    harness.key_press(Key::Enter);
    let start = std::time::Instant::now();
    while harness.state().content.terminals.screen(1).unwrap() != "j hello\nj hello" {
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "{:?}",
            harness.state().content.terminals.screen(1).unwrap()
        );
        harness.run();
        std::thread::sleep(Duration::from_millis(10));
    }

    harness.get_by_label("+").click();
    harness.run();
    harness.get_by_label("New terminal (Cmd+T)").click();
    harness.run();
    assert_eq!(
        open_tabs(&harness),
        [vec![Tab::Review(None), Tab::Terminal(1), Tab::Terminal(2)]]
    );
    let tile = harness.state().workspace.windows[0]
        .tree
        .tiles
        .find_pane(&Tab::Terminal(1))
        .unwrap();
    let state = harness.state_mut();
    let agents = review::Agents::default();
    let mut behavior = TabBehavior {
        content: &mut state.content,
        agents: &agents,
        found: &[],
        font: egui::FontId::default(),
        window: 0,
        window_count: 1,
        dragged_out: false,
        focused: &mut None,
        last_review: &mut None,
        action: &mut None,
    };
    egui_tiles::Behavior::on_tab_close(
        &mut behavior,
        &mut state.workspace.windows[0].tree.tiles,
        tile,
    );
    assert!(!harness.state().content.terminals.is_open(1));
    assert!(harness.state().content.terminals.is_open(2));
}

fn ask(harness: &mut Harness<'_, App>, args: &[&str]) -> Result<String, String> {
    let socket = harness.state().control.as_ref().unwrap().socket();
    let args: Vec<String> = args.iter().map(|arg| arg.to_string()).collect();
    let asking = std::thread::spawn(move || control::ask(&socket, &args));
    while !asking.is_finished() {
        harness.run();
        std::thread::sleep(Duration::from_millis(5));
    }
    asking.join().unwrap()
}

#[test]
fn agents_tell_the_editor_which_session_they_continue_in() {
    let (mut harness, _dir) = harness("agent-session");
    let id = harness.state_mut().workspace.open_agent(
        0,
        None,
        Agent::Claude,
        "s1".to_string(),
        Tuning::default(),
    );
    let hook = r#"{"session_id":"s2","source":"clear"}"#;
    assert_eq!(
        ask(&mut harness, &["agent-session", &id.to_string(), hook]),
        Ok(format!("Agent {id} continues in session s2"))
    );
    assert!(open_tabs(&harness)[0].contains(&Tab::Agent {
        id,
        agent: Agent::Claude,
        session: "s2".to_string(),
        tuning: Tuning::default(),
    }));
    assert!(ask(&mut harness, &["agent-session", "99", hook]).is_err());
    assert!(ask(&mut harness, &["agent-session", &id.to_string(), "{}"]).is_err());
}

#[test]
fn agents_ask_the_editor_what_it_shows_and_have_it_show_files() {
    let (mut harness, _dir) = harness("control");
    assert_eq!(
        ask(&mut harness, &["open", "README.md:1"]),
        Ok("Opened README.md:1".to_string())
    );
    assert_eq!(
        open_tabs(&harness),
        [vec![Tab::Review(None), file("README.md")]]
    );
    assert!(ask(&mut harness, &["open", "missing.rs"]).is_err());

    harness.key_press(Key::Escape);
    harness.state_mut().action = Some(Action::Reveal(Reveal {
        path: "README.md".to_string(),
        start: Pos::new(0, 2),
        end: Pos::new(0, 8),
        focus: true,
    }));
    harness.run();
    let context = ask(&mut harness, &["context"]).unwrap();
    for part in [
        "Open tabs:\n- review of no file yet\n- file README.md",
        "Selection: README.md:1:3 to 1:9\nReadme",
        "Review: 0/2 changed lines reviewed",
        "- src/main.rs: 0/1",
        "Review comments: 0",
    ] {
        assert!(context.contains(part), "{part:?} not in:\n{context}");
    }

    assert_eq!(
        ask(&mut harness, &["review"]),
        Ok("Showing the review of src/main.rs".to_string())
    );
    assert_eq!(
        open_tabs(&harness),
        [vec![review("src/main.rs"), file("README.md")]]
    );
    assert!(
        ask(&mut harness, &["what"])
            .unwrap_err()
            .starts_with("Unknown command: what")
    );
}

#[test]
fn agents_see_the_hunks_and_stage_the_ones_of_one_commit() {
    let (mut harness, _dir) = harness("stage-hunks");
    harness.get_by_label("Changes").click();
    harness.run();
    let hunks = ask(&mut harness, &["hunks"]).unwrap();
    assert!(
        hunks.contains("=== README.md\n--- hunk 1 @@ -0,0 +1,1 @@\n   1 +# Readme\n"),
        "{hunks}"
    );
    assert!(ask(&mut harness, &["stage", "README.md:1"]).is_ok());
    wait_for(&mut harness, "1/2 reviewed");
    assert!(ask(&mut harness, &["stage", "src/main.rs"]).is_ok());
    wait_for(&mut harness, "1/2 reviewed");
    assert_eq!(
        harness
            .state()
            .repo
            .changes()
            .unwrap()
            .staged
            .lines()
            .next(),
        Some("diff --git a/src/main.rs b/src/main.rs")
    );
}

#[test]
fn commits_all_changes_from_the_sidebar_with_the_message_an_agent_wrote() {
    let (mut harness, dir) = harness("commit");
    harness.get_by_label("Changes").click();
    harness.run();
    harness.get_by_label("All").click();
    wait_for(&mut harness, "2/2 reviewed");
    assert!(ask(&mut harness, &["commit-message", "Add", "the project"]).is_ok());
    for _ in 0..400 {
        harness.step();
        if harness.query_by_label("No changes").is_some() {
            break;
        }
        harness.get_by_label("Commit").click();
        std::thread::sleep(Duration::from_millis(5));
    }
    harness.get_by_label("No changes");
    let log = std::process::Command::new("git")
        .args(["log", "--format=%s"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&log.stdout), "Add the project\n");

    let branch = Repo::open(dir.path()).unwrap().branch();
    assert_eq!(harness.state().titled_branch, Some(branch));
    assert_eq!(
        window_title(Path::new("/work/shop"), "main"),
        "Shepherd — /work/shop — ⎇ main"
    );
}

#[test]
fn finds_the_lines_in_a_place() {
    assert_eq!(parse_place("src/a.rs:12-14"), ("src/a.rs", Some((12, 14))));
    assert_eq!(parse_place("a.rs:3"), ("a.rs", Some((3, 3))));
    assert_eq!(parse_place("a:b.rs"), ("a:b.rs", None));
}

#[test]
fn saves_the_layout_when_it_changed() {
    let (mut harness, _dir) = harness("save-layout");
    harness.state_mut().action = Some(Action::Open(file("README.md"), 0));
    harness.run();
    harness.state_mut().save_layout();

    let restored = Workspace::load(&harness.state().layout_path);
    assert_eq!(restored.windows[0].tabs().len(), 2);
}

fn run_until(harness: &mut Harness<'static, App>, done: impl Fn(&Harness<'static, App>) -> bool) {
    let start = Instant::now();
    while !done(harness) {
        assert!(start.elapsed() < Duration::from_secs(5), "timed out");
        harness.run();
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn shows_what_the_language_server_knows_about_the_code() {
    use serde_json::json;

    let (mut harness, dir) = harness("language-server");
    let (server, mut peer) = crate::lsp::tests::connect(dir.path());
    harness
        .state_mut()
        .content
        .open_files
        .connect_language_server("Rust", server);
    harness.state_mut().action = Some(Action::Open(Tab::File("src/main.rs".to_string()), 0));
    harness.run();
    assert!(
        harness
            .query_by_label("Starting the Rust language server…")
            .is_some()
    );
    let editor_while_starting = harness.get_by_label("Editor").rect();
    peer.answer_initialize();
    run_until(&mut harness, |harness| {
        harness
            .query_by_label("Starting the Rust language server…")
            .is_none()
    });
    assert_eq!(peer.next()["method"], "initialized");
    assert_eq!(harness.get_by_label("Editor").rect(), editor_while_starting);
    let opened = peer.next();
    assert_eq!(opened["method"], "textDocument/didOpen");
    let uri = opened["params"]["textDocument"]["uri"].clone();

    let range = |line: u32, from: u32, to: u32| json!({"start": {"line": line, "character": from}, "end": {"line": line, "character": to}});
    peer.send(json!({"jsonrpc": "2.0", "method": "textDocument/publishDiagnostics", "params": {
        "uri": uri, "diagnostics": [{"range": range(0, 0, 12), "severity": 1, "message": "Something is off"}],
    }}));
    run_until(&mut harness, |harness| {
        harness.state().content.open_files.problem_count() == 1
    });
    assert_eq!(
        harness
            .state_mut()
            .answer(&["diagnostics".to_string()], &egui::Context::default()),
        Ok("src/main.rs:1:1: error: Something is off".to_string())
    );

    let editor = harness.get_by_label("Editor").rect();
    let digit = harness.ctx.fonts_mut(|fonts| {
        fonts
            .layout_no_wrap(
                "0".into(),
                egui::FontId::monospace(13.0),
                egui::Color32::WHITE,
            )
            .size()
            .x
    });
    let on_main = egui::pos2(editor.left() + digit * 13.0, editor.top() + 8.0);
    harness.event(egui::Event::PointerMoved(on_main));
    harness.run_steps(4);
    assert!(harness.query_by_label("Something is off").is_some());
    let hover = peer.next();
    assert_eq!(hover["method"], "textDocument/hover");
    assert_eq!(hover["params"]["position"]["line"], 0);
    peer.send(json!({"jsonrpc": "2.0", "id": hover["id"], "result": {"contents": "fn main()"}}));
    run_until(&mut harness, |harness| {
        harness.query_by_label("fn main()").is_some()
    });

    for pressed in [true, false] {
        harness.event_modifiers(
            egui::Event::PointerButton {
                pos: on_main,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Modifiers::COMMAND,
            },
            Modifiers::COMMAND,
        );
        harness.run();
    }
    let definition = peer.next();
    assert_eq!(definition["method"], "textDocument/definition");
    assert_eq!(definition["params"]["position"]["line"], 0);

    harness.key_press_modifiers(Modifiers::COMMAND, Key::B);
    harness.run();
    let definition = std::iter::repeat_with(|| peer.next())
        .find(|message| message["method"] != "textDocument/hover")
        .unwrap();
    assert_eq!(definition["method"], "textDocument/definition");
    peer.send(json!({"jsonrpc": "2.0", "id": definition["id"], "result": {
        "uri": crate::lsp::uri(&dir.path().join("README.md")), "range": range(0, 2, 8),
    }}));
    run_until(&mut harness, |harness| {
        harness
            .state()
            .workspace
            .windows
            .iter()
            .flat_map(|window| window.tabs())
            .any(|tab| *tab == Tab::File("README.md".to_string()))
    });
    let selection = harness.state().content.open_files.selection().unwrap();
    assert_eq!(
        (selection.path.as_str(), selection.start),
        ("README.md", Pos::new(0, 2))
    );
}

#[test]
fn opens_pdf_files_in_another_app() {
    let (mut harness, _dir) = harness("pdf");
    let root = harness.state().content.root.clone();
    let tabs = open_tabs(&harness);
    harness.state_mut().action = Some(Action::Open(file("docs/Manual.pdf"), 0));
    harness.run();
    harness.state_mut().action = Some(Action::Reveal(Reveal {
        path: "guide.PDF".to_string(),
        start: Pos::default(),
        end: Pos::default(),
        focus: true,
    }));
    harness.run();
    assert_eq!(
        harness.state().content.opened_outside,
        [root.join("docs/Manual.pdf"), root.join("guide.PDF")]
    );
    assert_eq!(open_tabs(&harness), tabs);
}

#[test]
fn shows_what_images_look_like() {
    let (mut harness, dir) = harness("images");
    dir.write("pixel.png", crate::editor::image_view::tests::png(2, 1));
    dir.write(
        "logo.svg",
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="12"/>"#,
    );
    harness.state_mut().action = Some(Action::Open(file("pixel.png"), 0));
    run_until(&mut harness, |harness| {
        harness.query_by_label("2 × 1 px").is_some()
    });
    let path = dir.write("pixel.png", crate::editor::image_view::tests::png(4, 3));
    std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(std::time::SystemTime::now() + Duration::from_secs(5))
        .unwrap();
    harness.state_mut().content.open_files.reload_changed();
    run_until(&mut harness, |harness| {
        harness.query_by_label("4 × 3 px").is_some()
    });

    harness.state_mut().action = Some(Action::Open(file("logo.svg"), 0));
    run_until(&mut harness, |harness| {
        harness.query_by_label("24 × 12 px").is_some()
    });
    let svg = |harness: &mut Harness<'static, App>, text: &str| {
        let (buffer, _) = harness
            .state_mut()
            .content
            .open_files
            .text("logo.svg", false)
            .unwrap();
        buffer.set_text(text);
    };
    svg(
        &mut harness,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="48" height="12"/>"#,
    );
    run_until(&mut harness, |harness| {
        harness.query_by_label("48 × 12 px").is_some()
    });
    svg(&mut harness, "<svg");
    run_until(&mut harness, |harness| {
        harness
            .query_by_label_contains("Cannot show the SVG")
            .is_some()
    });
}

#[test]
fn closes_other_tabs_from_the_tab_menu_and_stops_their_terminals() {
    let (mut harness, _dir) = harness("close-others");
    harness.state_mut().content.terminals.run("/bin/cat", &[]);
    harness.state_mut().action = Some(Action::Open(file("README.md"), 0));
    harness.run();
    harness.key_press_modifiers(Modifiers::COMMAND, Key::T);
    harness.run();
    harness.state_mut().action = Some(Action::Open(file("src/main.rs"), 0));
    harness.run();
    assert!(harness.state().content.terminals.is_open(1));

    harness.get_by_label("main.rs").click_secondary();
    harness.run();
    harness
        .get_by_label("Close others, keep agents and terminals")
        .click();
    harness.run();
    assert_eq!(
        open_tabs(&harness),
        [vec![Tab::Terminal(1), file("src/main.rs")]]
    );
    assert!(harness.state().content.terminals.is_open(1));

    harness.get_by_label("main.rs").click_secondary();
    harness.run();
    harness.get_by_label("Close others").click();
    harness.run();
    assert_eq!(open_tabs(&harness), [vec![file("src/main.rs")]]);
    assert!(!harness.state().content.terminals.is_open(1));
}
