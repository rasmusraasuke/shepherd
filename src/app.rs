use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::egui::{self, Key, Modifiers, RichText, ViewportBuilder, ViewportCommand, ViewportId};
use egui_tiles::{SimplificationOptions, TileId, Tiles, UiResponse};

use crate::buffer::Pos;
use crate::comments;
use crate::control::{self, Control};
use crate::file_tree::FileTree;
use crate::file_view::OpenFiles;
use crate::git::Repo;
use crate::quick_open::{Outcome, QuickOpen};
use crate::review::{self, Review};
use crate::search::{ProjectSearch, Reveal};
use crate::terminal::{Activity, Terminals};
use crate::watcher::{Snapshot, Watcher};
use crate::workspace::{Sidebar, Tab, Window, Workspace};

const SAVE_INTERVAL: Duration = Duration::from_secs(1);

const DEFAULT_WINDOW_SIZE: egui::Vec2 = egui::vec2(1200.0, 800.0);

const DRAG_TITLE_SHADOW: i8 = 16;

pub struct App {
    repo: Repo,
    watcher: Watcher,
    control: Option<Control>,
    content: Content,
    workspace: Workspace,
    saved: Workspace,
    layout_path: PathBuf,
    last_save: Instant,
    viewports: HashMap<u64, ViewportBuilder>,
    quick_open: Option<(u64, QuickOpen)>,
    drag: Option<Drag>,
    action: Option<Action>,
}

struct Content {
    root: PathBuf,
    review: Review,
    open_files: OpenFiles,
    file_tree: FileTree,
    files: Vec<String>,
    search: ProjectSearch,
    terminals: Terminals,
    error: Option<String>,
}

enum Action {
    Open(Tab, usize),
    NewTerminal(usize, Option<TileId>),
    NewReview(usize, Option<TileId>),
    Review { path: String, new_tab: bool },
    ReviewLine { path: String, line: usize },
    Send { terminal: u64, text: String },
    Search,
    Reveal(Reveal),
    Move { from: usize, tile: TileId, to: Option<usize> },
    Drop(Drag),
    CloseWindow(usize),
}

struct Drag {
    from: usize,
    tile: TileId,
    point: egui::Pos2,
}

const CONTROL_HELP: &str = "\
editor ctl: ask the editor what the user sees, or have it show something.

  editor ctl context              All of the below in one go
  editor ctl selection            The place and the text selected in the file edited last
  editor ctl comments             The user's review comments on the uncommitted changes
  editor ctl review-state         Which changed lines are reviewed, by file; reviewed means staged
  editor ctl terminals            The terminals and what their programs are doing
  editor ctl terminal <id>        What a terminal shows
  editor ctl open <path>[:<line>[-<line>]]
                                  Open a file, selecting the lines
  editor ctl review [<path>]      Show the changes of a file for review, or of the next file to review";

fn parse_place(place: &str) -> (&str, Option<(usize, usize)>) {
    let Some((path, lines)) = place.rsplit_once(':') else { return (place, None) };
    let (first, last) = lines.split_once('-').unwrap_or((lines, lines));
    match (first.parse(), last.parse()) {
        (Ok(first), Ok(last)) => (path, Some((first, last))),
        _ => (place, None),
    }
}

pub fn viewport(root: &Path, window: &Window) -> ViewportBuilder {
    let builder = ViewportBuilder::default()
        .with_title(format!("editor — {}", root.display()))
        .with_inner_size(window.size.unwrap_or(DEFAULT_WINDOW_SIZE));
    match window.position {
        Some(position) => builder.with_position(position),
        None => builder,
    }
}

fn drag_title_frame(style: &egui::Style) -> egui::Frame {
    let mut frame = egui::Frame::popup(style).outer_margin(DRAG_TITLE_SHADOW);
    frame.fill = frame.fill.gamma_multiply(0.5);
    frame
}

fn drag_title(ui: &mut egui::Ui, title: &str) {
    drag_title_frame(ui.style()).show(ui, |ui| ui.add(egui::Label::new(title).extend()));
}

fn viewport_id(window: &Window) -> ViewportId {
    if window.id == 0 { ViewportId::ROOT } else { ViewportId::from_hash_of(("window", window.id)) }
}

impl App {
    pub fn new(repo: Repo, workspace: Workspace, ctx: egui::Context) -> Self {
        let control = Control::start(ctx.clone()).inspect_err(|e| eprintln!("{e}")).ok();
        let mut content = Content {
            review: Review::new(repo.clone()),
            open_files: OpenFiles::new(repo.root.clone()),
            file_tree: FileTree::default(),
            files: Vec::new(),
            search: ProjectSearch::default(),
            terminals: Terminals::new(repo.root.clone()),
            error: None,
            root: repo.root.clone(),
        };
        if let Some(control) = &control {
            let path = std::env::var("PATH").unwrap_or_default();
            content.terminals.set_env("PATH", format!("{}:{path}", control.bin_dir().display()));
            content.terminals.set_env(control::SOCKET_VARIABLE, control.socket().display().to_string());
        }
        Self {
            control,
            content,
            saved: workspace.clone(),
            viewports: workspace.windows.iter().map(|window| (window.id, viewport(&repo.root, window))).collect(),
            workspace,
            layout_path: repo.layout_path(),
            last_save: Instant::now(),
            quick_open: None,
            drag: None,
            action: None,
            watcher: Watcher::spawn(repo.clone(), ctx),
            repo,
        }
    }

    fn apply(&mut self, snapshot: Result<Snapshot, String>) {
        let content = &mut self.content;
        match snapshot {
            Ok(snapshot) => {
                content.error = None;
                content.review.set_changes(&snapshot.changes, &self.workspace.collapsed_changes);
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
            ui.label(format!("{done}/{total} lines reviewed"));
            ui.separator();
            ui.weak("Cmd+P open file · Cmd+F find in file · Cmd+Shift+F search in files · Cmd+T terminal · Cmd+click a change for a new review · j/k move · space mark reviewed · n next unreviewed · o open in file · Cmd+Shift+R review the line of a file · r refresh");
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
                if let Some(path) = self.content.file_tree.ui(ui, &mut self.workspace.expanded_dirs, None, |_| Default::default(), None) {
                    self.action = Some(Action::Open(Tab::File(path), 0));
                }
            }
            Sidebar::Changes => {
                let selected = self.workspace.last_review.as_ref().and_then(Tab::path);
                if let Some(path) = self.content.review.tree_ui(ui, &mut self.workspace.collapsed_changes, selected) {
                    let new_tab = ui.input(|i| i.modifiers.command);
                    self.action = Some(Action::Review { path, new_tab });
                }
            }
            Sidebar::Search => {
                if let Some(reveal) = self.content.search.ui(ui, &self.content.root, &self.content.files) {
                    self.action = Some(Action::Reveal(reveal));
                }
            }
        }
    }

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
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::F)) {
            self.action = Some(Action::Search);
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::T)) {
            self.action = Some(Action::NewTerminal(index, None));
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

        let pointer = ctx.input(|i| i.pointer.interact_pos());
        if let (Some(tile), Some(pointer)) = (window.tree.dragged_id(&ctx), pointer) {
            let point = inner.map(|inner| inner.min + pointer.to_vec2()).filter(|point| inner.is_some_and(|inner| !inner.contains(*point)));
            self.drag = point.map(|point| Drag { from: index, tile, point });
            if ctx.input(|i| i.pointer.any_released()) && let Some(drag) = self.drag.take() {
                self.action = Some(Action::Drop(drag));
            } else if self.drag.is_none() && let Some(tab) = window.tree.tiles.get_pane(&tile) {
                let area = egui::Area::new(egui::Id::unique("drag title")).order(egui::Order::Tooltip).interactable(false).constrain(false);
                area.pivot(egui::Align2::CENTER_CENTER).current_pos(pointer).show(&ctx, |ui| drag_title(ui, &tab.title()));
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
            let agents = self.content.terminals.agents();
            let mut behavior = TabBehavior {
                font: egui::TextStyle::Button.resolve(ui.style()),
                agents: &agents,
                content: &mut self.content,
                window: index,
                window_count,
                dragged_out,
                focused: &mut window.focused,
                last_review: &mut self.workspace.last_review,
                action: &mut self.action,
            };
            window.tree.ui(&mut behavior, ui);
        });

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
            Action::NewTerminal(window, tabs) => {
                let id = self.workspace.open_terminal(window, tabs);
                self.content.terminals.focus(id);
                self.focus(window, ctx);
            }
            Action::NewReview(window, tabs) => {
                self.workspace.add_tab(window, tabs, Tab::Review(None));
                self.workspace.last_review = Some(Tab::Review(None));
                self.focus(window, ctx);
            }
            Action::Review { path, new_tab } => {
                let window = self.workspace.show_review(&path, new_tab);
                self.focus(window, ctx);
            }
            Action::ReviewLine { path, line } => {
                self.content.review.show_line(&path, line);
                self.perform(Action::Review { path, new_tab: false }, ctx);
            }
            Action::Send { terminal, text } => self.content.terminals.send(terminal, &text),
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

    fn answer(&mut self, args: &[String], ctx: &egui::Context) -> Result<String, String> {
        let arg = |index: usize| args.get(index).map(String::as_str);
        let content = &self.content;
        match (arg(0), arg(1)) {
            (None | Some("help"), _) => Ok(CONTROL_HELP.to_string()),
            (Some("context"), None) => Ok(self.context()),
            (Some("selection"), None) => Ok(self.selection()),
            (Some("comments"), None) => match content.review.comments() {
                [] => Ok("No review comments".to_string()),
                all => Ok(comments::feedback(all)),
            },
            (Some("review-state"), None) => Ok(self.review_state()),
            (Some("terminals"), None) => Ok(self.terminals()),
            (Some("terminal"), Some(id)) => id.parse().ok().and_then(|id| content.terminals.screen(id)).ok_or_else(|| format!("No terminal {id}")),
            (Some("open"), Some(place)) => {
                let (path, lines) = parse_place(place);
                if !content.files.iter().any(|file| file == path) {
                    return Err(format!("No file {path} in the project"));
                }
                let (start, end) = match lines {
                    Some((first, last)) => (Pos::new(first.saturating_sub(1), 0), Pos::new(last.saturating_sub(1), usize::MAX)),
                    None => (Pos::default(), Pos::default()),
                };
                self.perform(Action::Reveal(Reveal { path: path.to_string(), start, end, focus: false }), ctx);
                Ok(format!("Opened {place}"))
            }
            (Some("review"), path) => {
                let Some(path) = path.map(str::to_string).or_else(|| content.review.first_unreviewed()) else { return Ok("Everything is reviewed".to_string()) };
                if !content.review.files_progress().iter().any(|(file, _)| *file == path) {
                    return Err(format!("No changes in {path}"));
                }
                self.perform(Action::Review { path: path.clone(), new_tab: false }, ctx);
                Ok(format!("Showing the review of {path}"))
            }
            _ => Err(format!("Unknown command: {}\n\n{CONTROL_HELP}", args.join(" "))),
        }
    }

    fn context(&self) -> String {
        let tabs: Vec<String> = self.workspace.windows.iter().flat_map(|window| window.tabs()).map(|tab| match tab {
            Tab::Review(path) => format!("- review of {}", path.as_deref().unwrap_or("no file yet")),
            Tab::File(path) => format!("- file {path}"),
            Tab::Terminal(id) => format!("- terminal {id}"),
        }).collect();
        let comments = self.content.review.comments().len();
        [
            format!("Repository: {} (branch {})", self.content.root.display(), self.repo.branch()),
            format!("Open tabs:\n{}", tabs.join("\n")),
            format!("Selection: {}", self.selection()),
            self.review_state(),
            format!("Review comments: {comments}{}", if comments > 0 { " (`editor ctl comments` shows them)" } else { "" }),
            self.terminals(),
        ]
        .join("\n\n")
    }

    fn selection(&self) -> String {
        match self.content.open_files.selection() {
            None => "nothing, no file was edited yet".to_string(),
            Some(selection) => {
                let place = |pos: Pos| format!("{}:{}", pos.line + 1, pos.col + 1);
                let mut out = format!("{}:{}", selection.path, place(selection.start));
                if selection.end != selection.start {
                    out.push_str(&format!(" to {}\n{}", place(selection.end), selection.text));
                }
                out
            }
        }
    }

    fn review_state(&self) -> String {
        let review = &self.content.review;
        let (done, total) = review.progress();
        let files: Vec<String> = review.files_progress().into_iter().map(|(path, (done, total))| format!("- {path}: {done}/{total}")).collect();
        let files = if files.is_empty() { "No uncommitted changes".to_string() } else { files.join("\n") };
        format!("Review: {done}/{total} changed lines reviewed. Reviewed lines are staged in git, the others are not.\n{files}")
    }

    fn terminals(&self) -> String {
        let terminals = &self.content.terminals;
        let lines: Vec<String> = self
            .workspace
            .windows
            .iter()
            .flat_map(|window| window.tabs())
            .filter_map(|tab| if let Tab::Terminal(id) = tab { Some(*id) } else { None })
            .map(|id| {
                let activity = match terminals.activity(id) {
                    Some(Activity::Working) => "working",
                    Some(Activity::Waiting) => "stopped, waiting",
                    Some(Activity::Idle) => "idle",
                    Some(Activity::Ended) => "ended",
                    None => "not started",
                };
                format!("- terminal {id}: {} ({activity})", terminals.title(id).unwrap_or_else(|| "Terminal".to_string()))
            })
            .collect();
        if lines.is_empty() { "Terminals: none".to_string() } else { format!("Terminals (`editor ctl terminal <id>` shows one):\n{}", lines.join("\n")) }
    }

    fn focus(&self, window: usize, ctx: &egui::Context) {
        ctx.send_viewport_cmd_to(viewport_id(&self.workspace.windows[window]), ViewportCommand::Focus);
    }

    fn drag_title(&self, ctx: &egui::Context) {
        let Some(drag) = &self.drag else { return };
        let Some(tab) = self.workspace.windows[drag.from].tree.tiles.get_pane(&drag.tile) else { return };
        let style = ctx.global_style();
        let text_size = ctx.fonts_mut(|fonts| fonts.layout_no_wrap(tab.title(), egui::TextStyle::Body.resolve(&style), egui::Color32::PLACEHOLDER)).size();
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
            .with_active(false)
            .with_mouse_passthrough(true);
        ctx.show_viewport_immediate(ViewportId::from_hash_of("drag title"), builder, |ui, _| {
            egui::CentralPanel::no_frame().show(ui, |ui| drag_title(ui, &tab.title()));
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

struct TabBehavior<'a> {
    content: &'a mut Content,
    agents: &'a [review::Agent],
    font: egui::FontId,
    window: usize,
    window_count: usize,
    dragged_out: bool,
    focused: &'a mut Option<TileId>,
    last_review: &'a mut Option<Tab>,
    action: &'a mut Option<Action>,
}

impl egui_tiles::Behavior<Tab> for TabBehavior<'_> {
    fn pane_ui(&mut self, ui: &mut egui::Ui, tile_id: TileId, tab: &mut Tab) -> UiResponse {
        if ui.rect_contains_pointer(ui.max_rect()) && ui.input(|i| i.pointer.any_pressed()) {
            *self.focused = Some(tile_id);
            if matches!(tab, Tab::Review(_)) {
                *self.last_review = Some(tab.clone());
            }
        }
        let active = self.last_review.as_ref().is_none_or(|last| last == tab);
        match tab {
            Tab::Review(path) => {
                let outcome = egui::Frame::new().inner_margin(8.0).show(ui, |ui| self.content.review.ui(ui, path.as_deref(), active, self.agents, &mut self.content.open_files)).inner;
                match outcome {
                    Some(review::Outcome::Show(path)) => *self.action = Some(Action::Review { path, new_tab: false }),
                    Some(review::Outcome::Send { terminal, text }) => *self.action = Some(Action::Send { terminal, text }),
                    Some(review::Outcome::Open { path, line }) => {
                        let place = Pos::new(line, 0);
                        *self.action = Some(Action::Reveal(Reveal { path, start: place, end: place, focus: true }));
                    }
                    None => {}
                }
            }
            Tab::File(path) => {
                if let Some(line) = self.content.open_files.ui(ui, path) {
                    *self.action = Some(Action::ReviewLine { path: path.clone(), line });
                }
            }
            Tab::Terminal(id) => self.content.terminals.ui(ui, *id),
        }
        UiResponse::None
    }

    fn tab_title_for_pane(&mut self, tab: &Tab) -> egui::WidgetText {
        let Tab::Terminal(id) = tab else { return tab.title().into() };
        let terminals = &self.content.terminals;
        let title = terminals.title(*id).unwrap_or_else(|| tab.title());
        let dot = match terminals.activity(*id) {
            Some(Activity::Working) => egui::Color32::from_rgb(90, 150, 230),
            Some(Activity::Waiting) => egui::Color32::from_rgb(230, 150, 50),
            _ => return title.into(),
        };
        let mut job = egui::text::LayoutJob::default();
        job.append("● ", 0.0, egui::TextFormat::simple(self.font.clone(), dot));
        job.append(&title, 0.0, egui::TextFormat::simple(self.font.clone(), egui::Color32::PLACEHOLDER));
        job.into()
    }

    fn top_bar_right_ui(&mut self, _tiles: &Tiles<Tab>, ui: &mut egui::Ui, tile_id: TileId, _tabs: &egui_tiles::Tabs, _scroll_offset: &mut f32) {
        ui.menu_button("+", |ui| {
            if ui.button("New terminal (Cmd+T)").clicked() {
                *self.action = Some(Action::NewTerminal(self.window, Some(tile_id)));
            }
            if ui.button("New review").clicked() {
                *self.action = Some(Action::NewReview(self.window, Some(tile_id)));
            }
        });
    }

    fn is_tab_closable(&self, _tiles: &Tiles<Tab>, _tile_id: TileId) -> bool {
        true
    }

    fn on_tab_close(&mut self, tiles: &mut Tiles<Tab>, tile_id: TileId) -> bool {
        match tiles.get_pane(&tile_id) {
            Some(Tab::File(path)) => self.content.open_files.close(path),
            Some(Tab::Terminal(id)) => self.content.terminals.close(*id),
            _ => {}
        }
        true
    }

    fn on_tab_button(&mut self, tiles: &mut Tiles<Tab>, tile_id: TileId, response: egui::Response) -> egui::Response {
        let Some(tab) = tiles.get_pane(&tile_id) else { return response };
        if response.clicked() {
            *self.focused = Some(tile_id);
            if matches!(tab, Tab::Review(_)) {
                *self.last_review = Some(tab.clone());
            }
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
        match tab.path() {
            Some(path) => response.on_hover_text(path),
            None => response,
        }
    }

    fn drag_ui(&mut self, _tiles: &Tiles<Tab>, _ui: &mut egui::Ui, _tile_id: TileId) {
    }

    fn paint_drag_preview(&self, visuals: &egui::Visuals, painter: &egui::Painter, parent_rect: Option<egui::Rect>, preview_rect: egui::Rect) {
        if self.dragged_out {
            return;
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
    fn show(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if let Some(snapshot) = self.watcher.changed() {
            self.apply(snapshot);
        }
        self.content.terminals.update(&ctx);
        for request in self.control.as_ref().map(Control::requests).unwrap_or_default() {
            let answer = self.answer(&request.args, &ctx);
            request.answer(answer);
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
        if self.content.review.take_changed() {
            self.watcher.refresh_now();
        }
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

        assert_eq!(open_tabs(&harness), [vec![Tab::Review(None), file("src/main.rs")]]);
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

        assert_eq!(open_tabs(&harness), [vec![Tab::Review(None), file("README.md")]]);
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
        harness.get_by_label("main.rs").click_modifiers(Modifiers::COMMAND);
        harness.run();
        assert_eq!(open_tabs(&harness), [vec![review("README.md"), review("src/main.rs")]]);

        harness.state_mut().action = Some(Action::NewReview(0, None));
        harness.run();
        harness.get_by_label("Choose a file under Changes to review it");
        harness.get_by_label("README.md").click();
        harness.run();
        assert_eq!(open_tabs(&harness), [vec![review("README.md"), review("src/main.rs"), Tab::Review(None)]]);
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
        assert_eq!(open_tabs(&harness), [vec![review("src/main.rs"), file("src/main.rs")]]);
        assert!(harness.ctx.memory(|m| m.focused().is_none()), "the keys are for the review");

        harness.key_press(Key::O);
        harness.run();
        assert!(harness.ctx.memory(|m| m.focused().is_some()), "the keys are for the file");
        assert_eq!(harness.state().content.open_files.selection().map(|selection| (selection.path, selection.start)), Some(("src/main.rs".to_string(), Pos::new(0, 0))));
    }

    #[test]
    fn shows_more_of_a_changed_file_than_its_changes_when_asked() {
        let (mut harness, dir) = harness("review-context");
        let lines: Vec<String> = (1..=60).map(|n| format!("line {n}")).collect();
        fs::write(dir.path().join("README.md"), lines.join("\n") + "\n").unwrap();
        crate::git::tests::commit_all(&Repo::open(dir.path()).unwrap());
        fs::write(dir.path().join("README.md"), lines.join("\n").replace("line 30\n", "changed\n") + "\n").unwrap();
        harness.state_mut().action = Some(Action::Review { path: "README.md".to_string(), new_tab: false });
        harness.state().watcher.refresh_now();
        wait_for(&mut harness, "Show 26 hidden lines");
        harness.get_by_label("Review: README.md");
        harness.get_by_label("Show 26 hidden lines").click();
        harness.run();
        assert!(harness.query_by_label("Show 26 hidden lines").is_none());
        harness.get_by_label("Show 27 hidden lines");
    }

    #[test]
    fn pops_a_tab_out_into_its_own_window_and_brings_it_back_when_the_window_is_emptied() {
        let (mut harness, _dir) = harness("pop-out");
        harness.state_mut().action = Some(Action::Open(file("README.md"), 0));
        harness.run();
        let tile = harness.state().workspace.windows[0].tree.tiles.find_pane(&file("README.md")).unwrap();
        harness.state_mut().action = Some(Action::Move { from: 0, tile, to: None });
        harness.run();

        assert_eq!(open_tabs(&harness), [vec![Tab::Review(None)], vec![file("README.md")]]);
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

        let point = egui::pos2(1500.0, 300.0);
        harness.state_mut().action = Some(Action::Drop(Drag { from: 0, tile, point }));
        harness.run();
        assert_eq!(open_tabs(&harness), [vec![Tab::Review(None)], vec![file("README.md")]]);

        let windows = &harness.state().workspace.windows;
        let review = windows[0].tree.tiles.find_pane(&Tab::Review(None)).unwrap();
        let point = windows[0].tree.tiles.rect(review).unwrap().center() + on_screen;
        let tile = windows[1].tree.tiles.find_pane(&file("README.md")).unwrap();
        harness.state_mut().action = Some(Action::Drop(Drag { from: 1, tile, point }));
        harness.run();
        assert_eq!(open_tabs(&harness), [vec![Tab::Review(None), file("README.md")]]);
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

            let windows = &harness.state().workspace.windows;
            let review = windows[0].tree.tiles.find_pane(&Tab::Review(None)).unwrap();
            let rect = windows[0].tree.tiles.rect(review).unwrap();
            let point = rect.center() + towards * rect.size();
            let tile = windows[1].tree.tiles.find_pane(&file(name)).unwrap();
            harness.state_mut().action = Some(Action::Drop(Drag { from: 1, tile, point }));
            harness.run();
        }

        assert_eq!(open_tabs(&harness), [vec![Tab::Review(None), file("README.md"), file("src/main.rs")]]);
        let tiles = &harness.state().workspace.windows[0].tree.tiles;
        let rect = |tab: &Tab| tiles.rect(tiles.find_pane(tab).unwrap()).unwrap();
        let (review, readme, main) = (rect(&Tab::Review(None)), rect(&file("README.md")), rect(&file("src/main.rs")));
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

        harness.key_press(Key::Escape);
        harness.run();
        assert_eq!(titles(&harness), before);
        harness.event(button(pointer, false));
        harness.run();
        assert_eq!(open_tabs(&harness), [vec![Tab::Review(None), file("README.md")]]);
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
        for text in ["// one", "\n", "let s = \"two", "\n\n", "\";"] {
            harness.event(egui::Event::Text(text.to_string()));
            harness.run();
        }
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
        harness.key_press(Key::End);
        harness.run();

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
        let readme = Reveal { path: "README.md".to_string(), start: Pos::new(0, 2), end: Pos::new(0, 8), focus: true };
        harness.state_mut().action = Some(Action::Reveal(readme));
        harness.run();
        harness.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::F);
        harness.run();
        assert_eq!(harness.get_by_role(egui::Role::TextInput).value().as_deref(), Some("Readme"));
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
        assert_eq!(open_tabs(&harness), [vec![Tab::Review(None), file("README.md"), file("src/main.rs")]]);
        assert!(harness.get_by_role(egui::Role::TextInput).is_focused());
        harness.key_press(Key::Enter);
        harness.run();

        harness.event(egui::Event::Text("bye".to_string()));
        harness.run();
        harness.state_mut().content.open_files.save_all();
        assert_eq!(fs::read_to_string(dir.path().join("src/main.rs")).unwrap(), "fn main() {\n    println!(\"bye\");\n}\n");
    }

    #[test]
    fn runs_a_program_in_a_new_terminal_that_takes_the_keyboard() {
        let (mut harness, _dir) = harness("terminal-tab");
        harness.state_mut().content.terminals.run("/bin/cat", &[]);
        harness.key_press_modifiers(Modifiers::COMMAND, Key::T);
        harness.run();
        assert_eq!(open_tabs(&harness), [vec![Tab::Review(None), Tab::Terminal(1)]]);
        assert!(harness.get_by_role(egui::Role::Terminal).is_focused());

        harness.event(egui::Event::Text("j hello".to_string()));
        harness.key_press(Key::Enter);
        let start = std::time::Instant::now();
        while harness.state().content.terminals.screen(1).unwrap() != "j hello\nj hello" {
            assert!(start.elapsed() < Duration::from_secs(5), "{:?}", harness.state().content.terminals.screen(1).unwrap());
            harness.run();
            std::thread::sleep(Duration::from_millis(10));
        }

        harness.get_by_label("+").click();
        harness.run();
        harness.get_by_label("New terminal (Cmd+T)").click();
        harness.run();
        assert_eq!(open_tabs(&harness), [vec![Tab::Review(None), Tab::Terminal(1), Tab::Terminal(2)]]);
        let tile = harness.state().workspace.windows[0].tree.tiles.find_pane(&Tab::Terminal(1)).unwrap();
        let state = harness.state_mut();
        let mut behavior = TabBehavior { content: &mut state.content, agents: &[], font: egui::FontId::default(), window: 0, window_count: 1, dragged_out: false, focused: &mut None, last_review: &mut None, action: &mut None };
        egui_tiles::Behavior::on_tab_close(&mut behavior, &mut state.workspace.windows[0].tree.tiles, tile);
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
    fn agents_ask_the_editor_what_it_shows_and_have_it_show_files() {
        let (mut harness, _dir) = harness("control");
        assert_eq!(ask(&mut harness, &["open", "README.md:1"]), Ok("Opened README.md:1".to_string()));
        assert_eq!(open_tabs(&harness), [vec![Tab::Review(None), file("README.md")]]);
        assert!(ask(&mut harness, &["open", "missing.rs"]).is_err());

        harness.key_press(Key::Escape);
        harness.state_mut().action = Some(Action::Reveal(Reveal { path: "README.md".to_string(), start: Pos::new(0, 2), end: Pos::new(0, 8), focus: true }));
        harness.run();
        let context = ask(&mut harness, &["context"]).unwrap();
        for part in ["Open tabs:\n- review of no file yet\n- file README.md", "Selection: README.md:1:3 to 1:9\nReadme", "Review: 0/2 changed lines reviewed", "- src/main.rs: 0/1", "Review comments: 0"] {
            assert!(context.contains(part), "{part:?} not in:\n{context}");
        }

        assert_eq!(ask(&mut harness, &["review"]), Ok("Showing the review of src/main.rs".to_string()));
        assert_eq!(open_tabs(&harness), [vec![review("src/main.rs"), file("README.md")]]);
        assert!(ask(&mut harness, &["what"]).unwrap_err().starts_with("Unknown command: what"));
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
}
