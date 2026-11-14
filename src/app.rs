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
use crate::search::{ProjectSearch, Reveal};
use crate::watcher::{Snapshot, Watcher};
use crate::workspace::{Sidebar, Tab, Window, Workspace};

/// How often a changed layout is written to disk.
const SAVE_INTERVAL: Duration = Duration::from_secs(1);

const DEFAULT_WINDOW_SIZE: egui::Vec2 = egui::vec2(1200.0, 800.0);

/// Room around the title of a dragged tab for its shadow.
const DRAG_TITLE_SHADOW: i8 = 16;

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
    /// The tab that is being dragged outside of its window.
    drag: Option<Drag>,
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
    search: ProjectSearch,
    error: Option<String>,
}

enum Action {
    /// Show the tab, opening it in the given window if it isn't open yet.
    Open(Tab, usize),
    /// Search across the project, in the sidebar of the main window.
    Search,
    /// Open the file and select a place in it.
    Reveal(Reveal),
    /// Move a tab to another window, or to a new one.
    Move { from: usize, tile: TileId, to: Option<usize> },
    /// Drop a tab that was dragged out of its window.
    Drop(Drag),
    CloseWindow(usize),
}

struct Drag {
    /// Index of the window the tab is dragged from.
    from: usize,
    tile: TileId,
    /// Where the pointer is on the screen.
    point: egui::Pos2,
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

/// The frame around the title of a dragged tab.
fn drag_title_frame(style: &egui::Style) -> egui::Frame {
    let mut frame = egui::Frame::popup(style).outer_margin(DRAG_TITLE_SHADOW);
    frame.fill = frame.fill.gamma_multiply(0.5); // see-through
    frame
}

/// The title of a dragged tab as it follows the pointer.
fn drag_title(ui: &mut egui::Ui, title: &str) {
    drag_title_frame(ui.style()).show(ui, |ui| ui.add(egui::Label::new(title).extend()));
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
            search: ProjectSearch::default(),
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
            drag: None,
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
                content.open_files.reload_changed();
                content.search.files_changed();
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
            ui.weak("Cmd+P open file · Cmd+F find in file · Cmd+Shift+F search in files · j/k move · space mark reviewed · n next unreviewed · r refresh");
        });
        if let Some(error) = &self.content.error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
    }

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.workspace.sidebar, Sidebar::Files, "Files");
            ui.selectable_value(&mut self.workspace.sidebar, Sidebar::Changes, "Changes");
            if ui.selectable_value(&mut self.workspace.sidebar, Sidebar::Search, "Search").clicked() {
                self.content.search.focus(None);
            }
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
            Sidebar::Search => {
                if let Some(reveal) = self.content.search.ui(ui, &self.content.root, &self.content.files) {
                    self.action = Some(Action::Reveal(reveal));
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
        window.screen_rect = inner.or(window.screen_rect);
        if close_requested && index > 0 {
            self.action = Some(Action::CloseWindow(index));
        }
        // before the tabs, where Cmd+F would take it
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::F)) {
            self.action = Some(Action::Search);
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::P)) {
            self.quick_open = Some((window_id, QuickOpen::default()));
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::S)) {
            self.content.open_files.save_all();
        }
        if ctx.memory(|m| m.focused().is_none()) && ctx.input(|i| i.modifiers.is_none() && i.key_pressed(Key::R)) {
            self.watcher.refresh_now();
        }

        // a tab dragged out of the window is dropped where the pointer is on the screen
        let pointer = ctx.input(|i| i.pointer.interact_pos());
        if let (Some(tile), Some(pointer)) = (window.tree.dragged_id(&ctx), pointer) {
            let point = inner.map(|inner| inner.min + pointer.to_vec2()).filter(|point| inner.is_some_and(|inner| !inner.contains(*point)));
            self.drag = point.map(|point| Drag { from: index, tile, point });
            if ctx.input(|i| i.pointer.any_released()) && let Some(drag) = self.drag.take() {
                self.action = Some(Action::Drop(drag));
            } else if self.drag.is_none() && let Some(tab) = window.tree.tiles.get_pane(&tile) {
                // not kept inside the window, so that the title leaves it together with the pointer
                let area = egui::Area::new(egui::Id::unique("drag title")).order(egui::Order::Tooltip).interactable(false).constrain(false);
                area.pivot(egui::Align2::CENTER_CENTER).current_pos(pointer).show(&ctx, |ui| drag_title(ui, tab.title()));
            }
        } else if self.drag.as_ref().is_some_and(|drag| drag.from == index) {
            self.drag = None;
        }
        let dragged_out = self.drag.as_ref().is_some_and(|drag| drag.from == index);

        let panel = egui::CentralPanel::no_frame().frame(egui::Frame::new().fill(ui.visuals().panel_fill)).show(ui, |ui| {
            if window.tabs().is_empty() {
                ui.centered_and_justified(|ui| ui.weak("Cmd+P to open a file"));
                return;
            }
            let mut behavior =
                TabBehavior { content: &mut self.content, window: index, window_count, dragged_out, focused: &mut window.focused, action: &mut self.action };
            window.tree.ui(&mut behavior, ui);
        });

        // show where a tab dragged here from another window would land
        if let Some(drag) = self.drag.as_ref().filter(|drag| self.workspace.window_at(drag.point, drag.from) == Some(index)) {
            let rect = self.workspace.windows[index].drop_target(drag.point).map_or(panel.response.rect, |target| target.rect);
            let stroke = ui.visuals().selection.stroke;
            ui.painter().rect(rect, 1.0, stroke.color.gamma_multiply(0.5), stroke, egui::StrokeKind::Inside);
        }

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
                if let Tab::File(path) = &tab {
                    self.content.open_files.focus(path);
                }
                let window = self.workspace.open(tab, window);
                self.focus(window, ctx);
            }
            Action::Reveal(Reveal { path, start, end, focus }) => {
                self.content.open_files.reveal(&path, start, end, focus);
                let window = self.workspace.open(Tab::File(path), 0);
                if focus {
                    self.focus(window, ctx);
                }
            }
            Action::Search => {
                self.workspace.sidebar = Sidebar::Search;
                let selected = self.content.open_files.selected_text();
                self.content.search.focus(selected);
                self.focus(0, ctx);
            }
            Action::Move { from, tile, to } => self.workspace.move_tab(from, tile, to),
            Action::Drop(Drag { from, tile, point }) => {
                let window = self.workspace.drop_tab(from, tile, point);
                self.focus(window, ctx);
            }
            Action::CloseWindow(index) => self.workspace.close_window(index),
        }
    }

    /// Brings the window to the front.
    fn focus(&self, window: usize, ctx: &egui::Context) {
        ctx.send_viewport_cmd_to(viewport_id(&self.workspace.windows[window]), ViewportCommand::Focus);
    }

    /// Shows the title of the tab that is dragged outside of its window at the pointer.
    /// It is a small window of its own, as nothing can be drawn outside of a window.
    fn drag_title(&self, ctx: &egui::Context) {
        let Some(drag) = &self.drag else { return };
        let Some(tab) = self.workspace.windows[drag.from].tree.tiles.get_pane(&drag.tile) else { return };
        let style = ctx.global_style();
        let text_size = ctx.fonts_mut(|fonts| fonts.layout_no_wrap(tab.title().to_string(), egui::TextStyle::Body.resolve(&style), egui::Color32::PLACEHOLDER)).size();
        let size = text_size + drag_title_frame(&style).total_margin().sum();
        let builder = ViewportBuilder::default()
            .with_position(drag.point - size / 2.0)
            .with_inner_size(size)
            .with_decorations(false)
            .with_transparent(true)
            .with_has_shadow(false)
            .with_resizable(false)
            .with_taskbar(false)
            .with_always_on_top()
            // the window with the dragged tab keeps the pointer and the keyboard
            .with_active(false)
            .with_mouse_passthrough(true);
        ctx.show_viewport_immediate(ViewportId::from_hash_of("drag title"), builder, |ui, _| {
            egui::CentralPanel::no_frame().show(ui, |ui| drag_title(ui, tab.title()));
        });
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
    /// Whether a tab of the window is being dragged outside of it.
    dragged_out: bool,
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

    fn drag_ui(&mut self, _tiles: &Tiles<Tab>, _ui: &mut egui::Ui, _tile_id: TileId) {
        // the title of the dragged tab is shown by the window, also outside of it
    }

    fn paint_drag_preview(&self, visuals: &egui::Visuals, painter: &egui::Painter, parent_rect: Option<egui::Rect>, preview_rect: egui::Rect) {
        if self.dragged_out {
            return; // the tab isn't going to land in this window
        }
        let stroke = self.drag_preview_stroke(visuals);
        if let Some(parent_rect) = parent_rect {
            painter.rect_stroke(parent_rect, 1.0, stroke, egui::StrokeKind::Inside);
        }
        painter.rect(preview_rect, 1.0, self.drag_preview_color(visuals), stroke, egui::StrokeKind::Inside);
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

        self.drag_title(&ctx);
        self.content.open_files.save_edited(&ctx);

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

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        // nothing behind what is drawn, for the see-through title of a dragged tab
        [0.0; 4]
    }

    fn on_exit(&mut self) {
        self.content.open_files.save_all();
        self.save_layout();
    }
}

#[cfg(test)]
mod tests {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable as _;

    use super::*;
    use crate::buffer::Pos;
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

    /// Runs frames until a label shows up, for what is done in the background.
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
    fn drops_a_dragged_tab_into_the_tabs_under_the_pointer_or_into_a_new_window() {
        let (mut harness, _dir) = harness("drop");
        harness.state_mut().action = Some(Action::Open(file("README.md"), 0));
        harness.run();
        let window = &mut harness.state_mut().workspace.windows[0];
        let on_screen = egui::vec2(100.0, 50.0);
        window.screen_rect = Some(egui::Rect::from_min_size(on_screen.to_pos2(), egui::vec2(1000.0, 700.0)));
        let tile = window.tree.tiles.find_pane(&file("README.md")).unwrap();

        // outside of every window
        let point = egui::pos2(1500.0, 300.0);
        harness.state_mut().action = Some(Action::Drop(Drag { from: 0, tile, point }));
        harness.run();
        assert_eq!(open_tabs(&harness), [vec![Tab::Review], vec![file("README.md")]]);

        // onto the review in the main window
        let windows = &harness.state().workspace.windows;
        let review = windows[0].tree.tiles.find_pane(&Tab::Review).unwrap();
        let point = windows[0].tree.tiles.rect(review).unwrap().center() + on_screen;
        let tile = windows[1].tree.tiles.find_pane(&file("README.md")).unwrap();
        harness.state_mut().action = Some(Action::Drop(Drag { from: 1, tile, point }));
        harness.run();
        assert_eq!(open_tabs(&harness), [vec![Tab::Review, file("README.md")]]);
        let tiles = &harness.state().workspace.windows[0].tree.tiles;
        assert_eq!(tiles.parent_of(tiles.find_pane(&file("README.md")).unwrap()), tiles.parent_of(review));
    }

    #[test]
    fn drops_a_dragged_tab_on_the_side_of_the_tabs_under_the_pointer_as_a_split() {
        let (mut harness, _dir) = harness("drop-split");
        let window = &mut harness.state_mut().workspace.windows[0];
        window.screen_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 700.0)));
        for (name, towards) in [("README.md", egui::vec2(0.4, 0.0)), ("src/main.rs", egui::vec2(0.0, 0.4))] {
            harness.state_mut().action = Some(Action::Open(file(name), 0));
            harness.run();
            let tile = harness.state().workspace.windows[0].tree.tiles.find_pane(&file(name)).unwrap();
            harness.state_mut().action = Some(Action::Move { from: 0, tile, to: None });
            harness.run();

            // dropped near the right and then the bottom edge of the review
            let windows = &harness.state().workspace.windows;
            let review = windows[0].tree.tiles.find_pane(&Tab::Review).unwrap();
            let rect = windows[0].tree.tiles.rect(review).unwrap();
            let point = rect.center() + towards * rect.size();
            let tile = windows[1].tree.tiles.find_pane(&file(name)).unwrap();
            harness.state_mut().action = Some(Action::Drop(Drag { from: 1, tile, point }));
            harness.run();
        }

        assert_eq!(open_tabs(&harness), [vec![Tab::Review, file("README.md"), file("src/main.rs")]]);
        let tiles = &harness.state().workspace.windows[0].tree.tiles;
        let rect = |tab: &Tab| tiles.rect(tiles.find_pane(tab).unwrap()).unwrap();
        let (review, readme, main) = (rect(&Tab::Review), rect(&file("README.md")), rect(&file("src/main.rs")));
        assert!(review.right() <= readme.left() && review.top() == readme.top());
        assert!(review.bottom() <= main.top() && review.left() == main.left() && main.right() <= readme.left());
    }

    #[test]
    fn shows_the_title_of_a_dragged_tab_at_the_pointer() {
        let (mut harness, _dir) = harness("drag-title");
        harness.state_mut().action = Some(Action::Open(file("README.md"), 0));
        harness.run();
        let titles = |harness: &Harness<'_, App>| harness.query_all_by_label("README.md").count();
        let before = titles(&harness);

        let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        let tab = harness.query_all_by_label("README.md").last().unwrap().rect().center();
        let pointer = tab + egui::vec2(100.0, 200.0);
        harness.event(egui::Event::PointerMoved(tab));
        harness.event(button(tab, true));
        harness.run();
        harness.event(egui::Event::PointerMoved(pointer));
        harness.run();
        assert_eq!(titles(&harness), before + 1);

        // giving up the drag leaves the tab where it was
        harness.key_press(Key::Escape);
        harness.run();
        assert_eq!(titles(&harness), before);
        harness.event(button(pointer, false));
        harness.run();
        assert_eq!(open_tabs(&harness), [vec![Tab::Review, file("README.md")]]);
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

        // the keys of the review don't reach it while a file is edited
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
        // far to the right of the first line: the cursor goes to its end
        let end_of_line = egui::pos2(editor.right() - 20.0, editor.top() + 5.0);
        harness.event(egui::Event::PointerMoved(end_of_line));
        for pressed in [true, false] {
            harness.event(egui::Event::PointerButton { pos: end_of_line, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        }
        harness.run();
        harness.event(egui::Event::Text("!".to_string()));
        harness.run();
        harness.state_mut().content.open_files.save_all();
        assert_eq!(fs::read_to_string(dir.path().join("README.md")).unwrap(), "# Readme!\n");
    }

    #[test]
    fn edits_a_highlighted_file() {
        let (mut harness, dir) = harness("edit-highlighted");
        harness.state_mut().action = Some(Action::Open(file("src/main.rs"), 0));
        harness.run();
        // lines come and go while the colors of the visible ones are kept
        for text in ["// one", "\n", "let s = \"two", "\n\n", "\";"] {
            harness.event(egui::Event::Text(text.to_string()));
            harness.run();
        }
        // the line breaks and what was typed after them are undone as one word
        harness.key_press_modifiers(Modifiers::COMMAND, Key::Z);
        harness.run();
        harness.state_mut().content.open_files.save_all();
        assert_eq!(fs::read_to_string(dir.path().join("src/main.rs")).unwrap(), "// one\nlet s = \"twofn main() {}\n");
    }

    #[test]
    fn keeps_the_line_numbers_over_text_scrolled_sideways() {
        let (mut harness, dir) = harness("gutter");
        let long_line = "x".repeat(300);
        fs::write(dir.path().join("README.md"), &long_line).unwrap();
        harness.state_mut().action = Some(Action::Open(file("README.md"), 0));
        harness.run();
        // to the end of the line, which scrolls the text to the left
        harness.key_press(Key::End);
        harness.run();

        // a click on the line numbers goes to the first character that can be seen, not under them
        // the editor itself now starts outside of the window; the tabs start where its pane does
        let pane_left = harness.get_by_role_and_label(egui::Role::Button, "Review").rect().left();
        let editor = harness.get_by_role(egui::Role::MultilineTextInput).rect();
        let gutter = egui::pos2(pane_left + 10.0, editor.top() + 5.0);
        harness.event(egui::Event::PointerMoved(gutter));
        for pressed in [true, false] {
            harness.event(egui::Event::PointerButton { pos: gutter, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
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

        harness.query_all_by_role(egui::Role::TextInput).last().unwrap().click();
        harness.run();
        harness.event(egui::Event::Text("2".to_string()));
        harness.run();
        harness.get_by_label("Replace").click();
        harness.run();
        harness.get_by_label("1/1");

        // closing the bar gives the keyboard back to the text, where the last found place is selected
        harness.key_press(Key::Escape);
        harness.run();
        harness.event(egui::Event::Text("3".to_string()));
        harness.run();
        harness.state_mut().content.open_files.save_all();
        assert_eq!(fs::read_to_string(dir.path().join("README.md")).unwrap(), "one 3\n2 one\n");
    }

    #[test]
    fn searches_the_project_and_opens_a_place_found() {
        let (mut harness, dir) = harness("project-search");
        fs::write(dir.path().join("src/main.rs"), "fn main() {\n    println!(\"hello\");\n}\n").unwrap();
        // from the word selected in a file
        let readme = Reveal { path: "README.md".to_string(), start: Pos::new(0, 2), end: Pos::new(0, 8), focus: true };
        harness.state_mut().action = Some(Action::Reveal(readme));
        harness.run();
        harness.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::F);
        harness.run();
        assert_eq!(harness.get_by_role(egui::Role::TextInput).value().as_deref(), Some("Readme"));
        wait_for(&mut harness, "1 found in 1 files");

        // a query of its own; capitals make the letter case matter
        let search_for = |harness: &mut Harness<'_, App>, query: &str, expected: &str| {
            harness.key_press_modifiers(Modifiers::COMMAND, Key::A);
            harness.event(egui::Event::Text(query.to_string()));
            wait_for(harness, expected);
        };
        search_for(&mut harness, "HELLO", "0 found in 0 files");
        search_for(&mut harness, "hello", "1 found in 1 files");
        // Down shows the place in its file while the keyboard stays in the search, Enter goes there
        harness.key_press(Key::ArrowDown);
        harness.run();
        assert_eq!(open_tabs(&harness), [vec![Tab::Review, file("README.md"), file("src/main.rs")]]);
        assert!(harness.get_by_role(egui::Role::TextInput).is_focused());
        harness.key_press(Key::Enter);
        harness.run();

        // the place found is selected: typing replaces it
        harness.event(egui::Event::Text("bye".to_string()));
        harness.run();
        harness.state_mut().content.open_files.save_all();
        assert_eq!(fs::read_to_string(dir.path().join("src/main.rs")).unwrap(), "fn main() {\n    println!(\"bye\");\n}\n");
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
