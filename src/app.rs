use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::egui::{self, Key, Modifiers, RichText, ViewportBuilder, ViewportCommand, ViewportId};
use egui_tiles::{SimplificationOptions, TileId, Tiles, UiResponse};

use crate::file_tree::FileTree;
use crate::file_view::OpenFiles;
use crate::git::Repo;
use crate::quick_open::{Outcome, QuickOpen};
use crate::review::{Review, ReviewState};
use crate::watcher::{Snapshot, Watcher};
use crate::workspace::{Sidebar, Tab, Window, Workspace};

/// How often a changed layout is written to disk.
const SAVE_INTERVAL: Duration = Duration::from_secs(1);

const DEFAULT_WINDOW_SIZE: egui::Vec2 = egui::vec2(1200.0, 800.0);

pub struct App {
    watcher: Watcher,
    content: Content,
    workspace: Workspace,
    /// The workspace as it was last written to disk.
    saved: Workspace,
    layout_path: PathBuf,
    last_save: Instant,
    /// How each popped out window was created. Kept as it was, so that a window isn't moved
    /// or resized by the editor while the user does the same.
    viewports: HashMap<u64, ViewportBuilder>,
    /// The quick open popup and the id of the window it is shown in.
    quick_open: Option<(u64, QuickOpen)>,
    /// What the user asked for during this frame; done once all windows are drawn.
    action: Option<Action>,
}

/// What the tabs and the sidebar show.
struct Content {
    root: PathBuf,
    review: Review,
    open_files: OpenFiles,
    file_tree: FileTree,
    files: Vec<String>,
    error: Option<String>,
}

enum Action {
    /// Show the tab, opening it in the given window if it isn't open yet.
    Open(Tab, usize),
    /// Move a tab to another window, or to a new one.
    Move { from: usize, tile: TileId, to: Option<usize> },
    CloseWindow(usize),
}

/// The title and placement of a window as it is first shown.
pub fn viewport(root: &Path, window: &Window) -> ViewportBuilder {
    let builder = ViewportBuilder::default()
        .with_title(format!("editor — {}", root.display()))
        .with_inner_size(window.size.unwrap_or(DEFAULT_WINDOW_SIZE));
    match window.position {
        Some(position) => builder.with_position(position),
        None => builder,
    }
}

fn viewport_id(window: &Window) -> ViewportId {
    if window.id == 0 { ViewportId::ROOT } else { ViewportId::from_hash_of(("window", window.id)) }
}

impl App {
    pub fn new(repo: Repo, workspace: Workspace, ctx: egui::Context) -> Self {
        let content = Content {
            review: Review::new(ReviewState::load(repo.review_state_path())),
            open_files: OpenFiles::new(repo.root.clone()),
            file_tree: FileTree::default(),
            files: Vec::new(),
            error: None,
            root: repo.root.clone(),
        };
        Self {
            content,
            saved: workspace.clone(),
            viewports: workspace.windows.iter().map(|window| (window.id, viewport(&repo.root, window))).collect(),
            workspace,
            layout_path: repo.layout_path(),
            last_save: Instant::now(),
            quick_open: None,
            action: None,
            watcher: Watcher::spawn(repo, ctx),
        }
    }

    fn apply(&mut self, snapshot: Result<Snapshot, String>) {
        let content = &mut self.content;
        match snapshot {
            Ok(snapshot) => {
                content.error = None;
                content.review.set_diff(&snapshot.diff);
                content.file_tree.set_files(snapshot.files.clone(), &self.workspace.expanded_dirs);
                content.files = snapshot.files;
                content.open_files.drop_changed();
            }
            Err(error) => content.error = Some(error),
        }
    }

    fn top_bar(&self, ui: &mut egui::Ui) {
        let (done, total) = self.content.review.progress();
        ui.horizontal(|ui| {
            ui.label(RichText::new(self.content.root.display().to_string()).strong());
            ui.separator();
            ui.label(format!("{done}/{total} hunks reviewed"));
            ui.separator();
            ui.weak("Cmd+P open file · j/k move · space mark reviewed · n next unreviewed · r refresh");
        });
        if let Some(error) = &self.content.error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
    }

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.workspace.sidebar, Sidebar::Files, "Files");
            ui.selectable_value(&mut self.workspace.sidebar, Sidebar::Changes, "Changes");
        });
        ui.separator();
        match self.workspace.sidebar {
            Sidebar::Files => {
                if let Some(path) = self.content.file_tree.ui(ui, &mut self.workspace.expanded_dirs) {
                    self.action = Some(Action::Open(Tab::File(path), 0));
                }
            }
            Sidebar::Changes => {
                if self.content.review.file_list(ui) {
                    self.action = Some(Action::Open(Tab::Review, 0));
                }
            }
        }
    }

    /// Shows the tabs of one window, filling the given ui.
    fn window_ui(&mut self, ui: &mut egui::Ui, index: usize) {
        let ctx = ui.ctx().clone();
        let window_count = self.workspace.windows.len();
        let window = &mut self.workspace.windows[index];
        let window_id = window.id;

        let (outer, inner, close_requested) = ctx.input(|i| (i.viewport().outer_rect, i.viewport().inner_rect, i.viewport().close_requested()));
        window.position = outer.map(|rect| rect.min).or(window.position);
        window.size = inner.map(|rect| rect.size()).or(window.size);
        if close_requested && index > 0 {
            self.action = Some(Action::CloseWindow(index));
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::P)) {
            self.quick_open = Some((window_id, QuickOpen::default()));
        }
        if ctx.memory(|m| m.focused().is_none()) && ctx.input(|i| i.modifiers.is_none() && i.key_pressed(Key::R)) {
            self.watcher.refresh_now();
        }

        egui::CentralPanel::no_frame().frame(egui::Frame::new().fill(ui.visuals().panel_fill)).show(ui, |ui| {
            if window.tabs().is_empty() {
                ui.centered_and_justified(|ui| ui.weak("Cmd+P to open a file"));
                return;
            }
            let mut behavior = TabBehavior { content: &mut self.content, window: index, window_count, focused: &mut window.focused, action: &mut self.action };
            window.tree.ui(&mut behavior, ui);
        });

        if let Some((_, quick_open)) = self.quick_open.as_mut().filter(|(id, _)| *id == window_id) {
            match quick_open.ui(&ctx, &self.content.files) {
                Some(Outcome::Open(path)) => {
                    self.action = Some(Action::Open(Tab::File(path), index));
                    self.quick_open = None;
                }
                Some(Outcome::Closed) => self.quick_open = None,
                None => {}
            }
        }
    }

    fn perform(&mut self, action: Action, ctx: &egui::Context) {
        match action {
            Action::Open(tab, window) => {
                let window = self.workspace.open(tab, window);
                ctx.send_viewport_cmd_to(viewport_id(&self.workspace.windows[window]), ViewportCommand::Focus);
            }
            Action::Move { from, tile, to } => self.workspace.move_tab(from, tile, to),
            Action::CloseWindow(index) => self.workspace.close_window(index),
        }
    }

    fn save_layout(&mut self) {
        self.last_save = Instant::now();
        if self.workspace == self.saved {
            return;
        }
        if let Err(e) = fs::write(&self.layout_path, self.workspace.to_json()) {
            eprintln!("failed to save layout to {}: {e}", self.layout_path.display());
        }
        self.saved = self.workspace.clone();
    }
}

/// Tells the tiles of a window how to show its tabs.
struct TabBehavior<'a> {
    content: &'a mut Content,
    /// Index of the window the tabs are in.
    window: usize,
    window_count: usize,
    focused: &'a mut Option<TileId>,
    action: &'a mut Option<Action>,
}

impl egui_tiles::Behavior<Tab> for TabBehavior<'_> {
    fn pane_ui(&mut self, ui: &mut egui::Ui, tile_id: TileId, tab: &mut Tab) -> UiResponse {
        if ui.rect_contains_pointer(ui.max_rect()) && ui.input(|i| i.pointer.any_pressed()) {
            *self.focused = Some(tile_id);
        }
        match tab {
            Tab::Review => {
                egui::Frame::new().inner_margin(8.0).show(ui, |ui| self.content.review.ui(ui));
            }
            Tab::File(path) => self.content.open_files.ui(ui, path),
        }
        UiResponse::None
    }

    fn tab_title_for_pane(&mut self, tab: &Tab) -> egui::WidgetText {
        tab.title().into()
    }

    fn is_tab_closable(&self, _tiles: &Tiles<Tab>, _tile_id: TileId) -> bool {
        true
    }

    fn on_tab_close(&mut self, tiles: &mut Tiles<Tab>, tile_id: TileId) -> bool {
        if let Some(Tab::File(path)) = tiles.get_pane(&tile_id) {
            self.content.open_files.close(path);
        }
        true
    }

    fn on_tab_button(&mut self, tiles: &mut Tiles<Tab>, tile_id: TileId, response: egui::Response) -> egui::Response {
        let Some(tab) = tiles.get_pane(&tile_id) else { return response };
        if response.clicked() {
            *self.focused = Some(tile_id);
        }
        response.context_menu(|ui| {
            let mut move_to = |ui: &mut egui::Ui, label: String, to: Option<usize>| {
                if ui.button(label).clicked() {
                    *self.action = Some(Action::Move { from: self.window, tile: tile_id, to });
                    ui.close();
                }
            };
            move_to(ui, "Move to new window".to_string(), None);
            for to in (0..self.window_count).filter(|&to| to != self.window) {
                let label = if to == 0 { "Move to main window".to_string() } else { format!("Move to window {}", to + 1) };
                move_to(ui, label, Some(to));
            }
        });
        match tab {
            Tab::Review => response,
            Tab::File(path) => response.on_hover_text(path),
        }
    }

    fn simplification_options(&self) -> SimplificationOptions {
        SimplificationOptions { all_panes_must_have_tabs: true, ..Default::default() }
    }
}

impl App {
    /// Shows the main window in the given ui, and the popped out windows next to it.
    fn show(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if let Some(snapshot) = self.watcher.changed() {
            self.apply(snapshot);
        }

        egui::Panel::top("top").show(ui, |ui| self.top_bar(ui));
        egui::Panel::left("sidebar").resizable(true).default_size(280.0).show(ui, |ui| self.sidebar(ui));
        self.window_ui(ui, 0);
        for index in 1..self.workspace.windows.len() {
            let window = &self.workspace.windows[index];
            let builder = self.viewports.entry(window.id).or_insert_with(|| viewport(&self.content.root, window)).clone();
            ctx.show_viewport_immediate(viewport_id(window), builder, |ui, _| self.window_ui(ui, index));
        }

        if let Some(action) = self.action.take() {
            self.perform(action, &ctx);
            ctx.request_repaint();
        }
        self.workspace.prune();
        if self.last_save.elapsed() >= SAVE_INTERVAL {
            self.save_layout();
        }
    }

}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.show(ui);
    }

    fn on_exit(&mut self) {
        self.save_layout();
    }
}

#[cfg(test)]
mod tests {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable as _;

    use super::*;
    use crate::git::tests::temp_repo;
    use crate::testing::TempDir;

    fn harness(name: &str) -> (Harness<'static, App>, TempDir) {
        let (repo, dir) = temp_repo(name, &[("src/main.rs", "fn main() {}\n"), ("README.md", "# Readme\n")]);
        let snapshot = Snapshot::take(&repo);
        let mut app = App::new(repo, Workspace::default(), egui::Context::default());
        app.apply(snapshot);
        let mut harness = Harness::builder().with_size(egui::vec2(1000.0, 700.0)).build_ui_state(|ui, app: &mut App| app.show(ui), app);
        harness.run();
        (harness, dir)
    }

    fn open_tabs(harness: &Harness<'_, App>) -> Vec<Vec<Tab>> {
        harness.state().workspace.windows.iter().map(|window| window.tabs().into_iter().cloned().collect()).collect()
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

        assert_eq!(open_tabs(&harness), [vec![Tab::Review, file("src/main.rs")]]);
        // once in the tree and once as the title of the tab
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

        assert_eq!(open_tabs(&harness), [vec![Tab::Review, file("README.md")]]);
        assert!(harness.state().quick_open.is_none());
    }

    #[test]
    fn shows_changed_files_in_the_sidebar_and_opens_the_review_from_there() {
        let (mut harness, _dir) = harness("changes");
        let review = harness.state().workspace.windows[0].tree.tiles.find_pane(&Tab::Review).unwrap();
        harness.state_mut().workspace.windows[0].tree.remove_recursively(review);
        harness.get_by_label("Changes").click();
        harness.run();
        harness.get_by_label_contains("README.md").click();
        harness.run();

        assert_eq!(open_tabs(&harness), [vec![Tab::Review]]);
    }

    #[test]
    fn pops_a_tab_out_into_its_own_window_and_brings_it_back_when_the_window_is_emptied() {
        let (mut harness, _dir) = harness("pop-out");
        harness.state_mut().action = Some(Action::Open(file("README.md"), 0));
        harness.run();
        let tile = harness.state().workspace.windows[0].tree.tiles.find_pane(&file("README.md")).unwrap();
        harness.state_mut().action = Some(Action::Move { from: 0, tile, to: None });
        harness.run();

        assert_eq!(open_tabs(&harness), [vec![Tab::Review], vec![file("README.md")]]);
        // both windows are drawn: the file is in the tree and is the title of a tab in the new window
        harness.get_by_label("Review");
        assert_eq!(harness.query_all_by_role_and_label(egui::Role::Button, "README.md").count(), 2);

        let tile = harness.state().workspace.windows[1].tree.tiles.find_pane(&file("README.md")).unwrap();
        harness.state_mut().action = Some(Action::Move { from: 1, tile, to: Some(0) });
        harness.run();
        assert_eq!(open_tabs(&harness).len(), 1);
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
}
