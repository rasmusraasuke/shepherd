mod bars;
mod ctl;
mod tabs;
mod window;
pub mod workspace;

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui::{self, Key, Modifiers, ViewportBuilder, ViewportCommand, ViewportId};
use egui_tiles::TileId;

use crate::agents::{self, Agent, Model, Tuning};
use crate::app::workspace::{Closing, Sidebar, Tab, Workspace};
use crate::control::{self, Control};
use crate::editor::file_view::OpenFiles;
use crate::git::watcher::{Snapshot, Watcher};
use crate::git::{CommitRange, Repo};
use crate::github;
use crate::lsp::install::Installer;
use crate::panels::file_tree::FileTree;
use crate::panels::flow::{self, Flow};
use crate::panels::history::{History, Outcome as HistoryOutcome};
use crate::panels::keys::{self, DoubleTap};
use crate::panels::projects::Projects;
use crate::panels::quick_chat::QuickChat;
use crate::panels::quick_open::QuickOpen;
use crate::panels::search::{ProjectSearch, Reveal};
use crate::review::{self, Review};
use crate::settings::Settings;
use crate::terminal::{Prompt, Terminals};
use crate::ui::icons::FileType;
use crate::ui::theme;
use crate::update;

pub use window::viewport;
use window::{viewport_id, window_title};
const SAVE_INTERVAL: Duration = Duration::from_secs(1);

const EXIT_GRACE: Duration = Duration::from_secs(2);

const ACTIVITY_WIDTH: f32 = 44.0;

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
    quick_chat: Option<(u64, QuickChat)>,
    agents: Vec<Agent>,
    models: HashMap<Agent, Vec<Model>>,
    loading_models: Option<std::sync::mpsc::Receiver<HashMap<Agent, Vec<Model>>>>,
    search_in: Option<u64>,
    double_shift: DoubleTap,
    double_ctrl: DoubleTap,
    show_keys: bool,
    settings: Settings,
    settings_path: Option<PathBuf>,
    drag: Option<Drag>,
    action: Option<Action>,
    titled_branch: Option<String>,
    updates: update::Updates,
    build: update::Build,
    checks: github::Checks,
    restarting: bool,
    projects: Projects,
}

struct Content {
    root: PathBuf,
    repo: Repo,
    review: Review,
    commits: HashMap<CommitRange, Review>,
    history: History,
    open_files: OpenFiles,
    file_tree: FileTree,
    files: Vec<String>,
    search: ProjectSearch,
    terminals: Terminals,
    flow: Flow,
    error: Option<String>,
    opened_outside: Vec<PathBuf>,
}

enum Action {
    Open(Tab, usize),
    NewTerminal(usize, Option<TileId>),
    NewAgent(usize, Option<TileId>, Agent),
    NewReview(usize, Option<TileId>),
    Review {
        path: String,
        new_tab: bool,
    },
    OpenCommit {
        range: CommitRange,
        label: String,
        file: Option<String>,
        new_tab: bool,
    },
    FileHistory(String),
    ShowCommit(String),
    ReviewLine {
        path: String,
        line: usize,
    },
    Send {
        agent: Option<u64>,
        text: String,
        window: usize,
        images: Vec<PathBuf>,
    },
    Draft {
        text: String,
    },
    Search(u64),
    Reveal(Reveal),
    Move {
        from: usize,
        tile: TileId,
        to: Option<usize>,
    },
    Drop(Drag),
    CloseWindow(usize),
    CloseTabs {
        window: usize,
        tile: TileId,
        closing: Closing,
    },
}

impl Content {
    fn close(&mut self, tab: &Tab) {
        match tab {
            Tab::File(path) => self.open_files.close(path),
            Tab::Terminal(id) | Tab::Agent { id, .. } => self.terminals.close(*id),
            _ => {}
        }
    }

    fn run(&mut self, op: flow::Op, ctx: &egui::Context) {
        self.open_files.save_all();
        self.flow.run(op, ctx);
    }

    fn open_outside(&mut self, path: &str) {
        let path = self.root.join(path);
        if cfg!(test) {
            self.opened_outside.push(path);
            return;
        }
        let opener = if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        match std::process::Command::new(opener).arg(&path).spawn() {
            Ok(mut child) => {
                std::thread::spawn(move || child.wait());
            }
            Err(e) => self.error = Some(format!("Cannot open {}: {e}", path.display())),
        }
    }

    fn commit_review(&mut self, range: &CommitRange, label: &str) -> &mut Review {
        self.commits.entry(range.clone()).or_insert_with(|| {
            Review::for_commits(self.repo.clone(), range.clone(), label.to_string())
        })
    }
}

struct Drag {
    from: usize,
    tile: TileId,
    point: egui::Pos2,
}
impl App {
    pub fn new(repo: Repo, workspace: Workspace, ctx: egui::Context) -> Self {
        Self::with_settings(repo, workspace, Settings::load(), Settings::path(), ctx)
    }

    fn with_settings(
        repo: Repo,
        workspace: Workspace,
        settings: Settings,
        settings_path: Option<PathBuf>,
        ctx: egui::Context,
    ) -> Self {
        theme::apply(&ctx, settings.system_fonts);
        let control = Control::start(ctx.clone())
            .inspect_err(|e| eprintln!("{e}"))
            .ok();
        let path = agents::search_path();
        let agents = agents::found(&path);
        let mut agent_env = HashMap::from([("PATH".to_string(), path.clone())]);
        if let Some(control) = &control {
            agent_env.insert(
                control::SOCKET_VARIABLE.to_string(),
                control.socket().display().to_string(),
            );
        }
        let loading_models =
            cfg!(not(test)).then(|| agents::load_models(agents.clone(), agent_env, ctx.clone()));
        let mut content = Content {
            review: Review::new(repo.clone()),
            repo: repo.clone(),
            commits: HashMap::new(),
            history: History::new(repo.clone()),
            open_files: OpenFiles::new(repo.root.clone()),
            file_tree: FileTree::default(),
            files: Vec::new(),
            search: ProjectSearch::default(),
            terminals: Terminals::new(repo.root.clone()),
            flow: Flow::new(repo.clone()),
            error: None,
            opened_outside: Vec::new(),
            root: repo.root.clone(),
        };
        if cfg!(not(test)) {
            content.open_files.start_language_servers(
                ctx.clone(),
                Installer::new(
                    crate::dirs::config()
                        .unwrap_or_default()
                        .join("language-servers"),
                    path.clone(),
                    settings.download_language_servers,
                ),
            );
        }
        content.terminals.set_system_fonts(settings.system_fonts);
        content.terminals.set_env("PATH", path.clone());
        if let Some(control) = &control {
            content
                .terminals
                .set_env("PATH", format!("{}:{path}", control.bin_dir().display()));
            content.terminals.set_env(
                control::SOCKET_VARIABLE,
                control.socket().display().to_string(),
            );
        }
        if !agents.is_empty() {
            let guide = repo.layout_path().with_file_name("shepherd-agent-guide.md");
            if agents::write_guide(&guide) {
                content.terminals.set_guide(guide);
            }
        }
        let updates = update::Updates::new(ctx.clone());
        #[cfg(not(test))]
        updates.start();
        Self {
            control,
            content,
            saved: workspace.clone(),
            viewports: workspace
                .windows
                .iter()
                .map(|window| (window.id, viewport(&repo.root, window)))
                .collect(),
            workspace,
            layout_path: repo.layout_path(),
            last_save: Instant::now(),
            quick_open: None,
            quick_chat: None,
            agents,
            models: HashMap::new(),
            loading_models,
            updates,
            build: update::Build::watch(),
            checks: github::Checks::new(repo.origin().as_deref().and_then(github::repository)),
            search_in: None,
            double_shift: DoubleTap::shift(),
            double_ctrl: DoubleTap::ctrl(),
            show_keys: false,
            projects: Projects::new(
                settings_path
                    .as_ref()
                    .and_then(|path| path.parent())
                    .map(|dir| dir.join("projects")),
                repo.root.clone(),
            ),
            settings,
            settings_path,
            drag: None,
            action: None,
            titled_branch: None,
            watcher: Watcher::spawn(repo.clone(), ctx),
            restarting: false,
            repo,
        }
    }

    fn chosen_agent(&self) -> Option<Agent> {
        self.workspace.agent.agent(&self.agents)
    }

    fn draft_chosen_agent(&self) -> Option<Agent> {
        self.workspace.draft_agent.agent(&self.agents)
    }

    fn review_agents(&self) -> review::Agents {
        review::Agents {
            open: self.content.terminals.agents(),
            new: self.chosen_agent().map(|agent| agent.name().to_string()),
        }
    }

    fn draft_agents(&self) -> review::Agents {
        review::Agents {
            open: Vec::new(),
            new: self
                .draft_chosen_agent()
                .map(|agent| agent.name().to_string()),
        }
    }

    fn open_agent(
        &mut self,
        window: usize,
        tabs: Option<TileId>,
        agent: Agent,
        tuning: Tuning,
        prompt: Prompt,
        ctx: &egui::Context,
    ) {
        let session = agent.new_session();
        let id = self
            .workspace
            .open_agent(window, tabs, agent, session, tuning);
        self.content.terminals.start(id, prompt);
        self.focus(window, ctx);
    }

    fn apply(&mut self, snapshot: Result<Snapshot, String>) {
        let content = &mut self.content;
        match snapshot {
            Ok(snapshot) => {
                content.error = None;
                self.checks
                    .set_commit(&snapshot.status.branch, &snapshot.head);
                content
                    .review
                    .set_changes(&snapshot.changes, &self.workspace.collapsed_changes);
                content
                    .file_tree
                    .set_files(snapshot.files.clone(), &self.workspace.expanded_dirs);
                content.files = snapshot.files;
                content.flow.update(snapshot.status, &snapshot.changes);
                content.history.set_commits(snapshot.commits);
                content.open_files.set_head(&snapshot.head);
                content.open_files.reload_changed();
                content.search.files_changed();
            }
            Err(error) => content.error = Some(error),
        }
    }
    fn perform(&mut self, action: Action, ctx: &egui::Context) {
        match action {
            Action::Open(Tab::File(path), _) | Action::Reveal(Reveal { path, .. })
                if FileType::of(&path).opens_outside() =>
            {
                self.content.open_outside(&path)
            }
            Action::Open(tab, window) => {
                if let Tab::File(path) = &tab {
                    self.content.open_files.focus(path);
                }
                let window = self.workspace.open(tab, window);
                self.focus(window, ctx);
            }
            Action::Reveal(Reveal {
                path,
                start,
                end,
                focus,
            }) => {
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
            Action::NewAgent(window, tabs, agent) => {
                let tuning = self.workspace.agent.tuning(agent);
                self.open_agent(window, tabs, agent, tuning, Prompt::default(), ctx);
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
            Action::OpenCommit {
                range,
                label,
                file,
                new_tab,
            } => {
                let path = {
                    let review = self.content.commit_review(&range, &label);
                    file.filter(|path| review.has_file(path))
                        .or_else(|| review.first_file())
                };
                let window = self
                    .workspace
                    .show_commit_review(range, path, label, new_tab);
                self.focus(window, ctx);
            }
            Action::FileHistory(path) => {
                self.content.history.show_file(Some(path));
                self.workspace.sidebar = Sidebar::History;
                self.workspace.sidebar_open = true;
            }
            Action::ShowCommit(hash) => {
                self.workspace.sidebar = Sidebar::History;
                self.workspace.sidebar_open = true;
                if let Some(HistoryOutcome::Open {
                    range,
                    label,
                    file,
                    new_tab,
                }) = self.content.history.show_commit(&hash)
                {
                    self.perform(
                        Action::OpenCommit {
                            range,
                            label,
                            file,
                            new_tab,
                        },
                        ctx,
                    );
                }
            }
            Action::ReviewLine { path, line } => {
                self.content.review.show_line(&path, line);
                self.perform(
                    Action::Review {
                        path,
                        new_tab: false,
                    },
                    ctx,
                );
            }
            Action::Send {
                agent,
                text,
                window,
                images,
            } => match agent {
                Some(id) => self.content.terminals.send(id, &text),
                None => {
                    if let Some(agent) = self.chosen_agent() {
                        let tuning = self.workspace.agent.tuning(agent);
                        self.open_agent(
                            window,
                            None,
                            agent,
                            tuning,
                            Prompt {
                                message: Some(text),
                                images,
                            },
                            ctx,
                        );
                    }
                }
            },
            Action::Draft { text } => {
                let result = match self.draft_chosen_agent() {
                    Some(agent) => {
                        let tuning = self.workspace.draft_agent.tuning(agent);
                        self.content.terminals.run_background(agent, &tuning, &text)
                    }
                    None => Err("No agent was found: install Claude Code or Opencode".to_string()),
                };
                if let Err(error) = result {
                    self.content.flow.fail(error);
                }
            }
            Action::Search(window_id) => {
                self.search_in = Some(window_id);
                let selected = self.content.open_files.selected_text();
                self.content.search.start_with(selected);
            }
            Action::Move { from, tile, to } => self.workspace.move_tab(from, tile, to),
            Action::Drop(Drag { from, tile, point }) => {
                let window = self.workspace.drop_tab(from, tile, point);
                self.focus(window, ctx);
            }
            Action::CloseWindow(index) => self.workspace.close_window(index),
            Action::CloseTabs {
                window,
                tile,
                closing,
            } => {
                for tab in self.workspace.close_tabs(window, tile, closing) {
                    self.content.close(&tab);
                }
            }
        }
    }
    fn save_layout(&mut self) {
        self.last_save = Instant::now();
        if self.workspace == self.saved {
            return;
        }
        if let Err(e) = fs::write(&self.layout_path, self.workspace.to_json()) {
            eprintln!(
                "failed to save layout to {}: {e}",
                self.layout_path.display()
            );
        }
        self.saved = self.workspace.clone();
    }

    fn apply_fonts(&mut self, ctx: &egui::Context) {
        theme::apply(ctx, self.settings.system_fonts);
        self.content
            .terminals
            .set_system_fonts(self.settings.system_fonts);
    }

    fn save_settings(&self) {
        let Some(path) = &self.settings_path else {
            return;
        };
        if let Err(e) = self.settings.save_to(path) {
            eprintln!("failed to save settings to {}: {e}", path.display());
        }
    }
}
impl App {
    fn show(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if let Some(snapshot) = self.watcher.changed() {
            self.apply(snapshot);
        }
        if let Some(models) = self
            .loading_models
            .as_ref()
            .and_then(|loading| loading.try_recv().ok())
        {
            self.models = models;
            self.loading_models = None;
        }
        self.content.terminals.update(&ctx);
        if self.content.flow.finished(&ctx) {
            self.watcher.refresh_now();
            self.checks.refresh();
        }
        self.checks.poll(&ctx);
        let branch = &self.content.flow.status().branch;
        if self.titled_branch.as_ref() != Some(branch) {
            self.titled_branch = Some(branch.clone());
            let title = window_title(&self.content.root, branch);
            for builder in self.viewports.values_mut() {
                builder.title = Some(title.clone());
            }
            ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Title(title));
        }
        for request in self
            .control
            .as_ref()
            .map(Control::requests)
            .unwrap_or_default()
        {
            let answer = self.answer(&request.args, &ctx);
            request.answer(answer);
        }

        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Slash)) {
            self.show_keys = !self.show_keys;
        }
        self.title_bar(ui);
        egui::Panel::left("activity")
            .resizable(false)
            .exact_size(ACTIVITY_WIDTH)
            .frame(egui::Frame::new().fill(ui.visuals().panel_fill))
            .show(ui, |ui| self.activity_bar(ui));
        let mut sidebar_open = self.workspace.sidebar_open;
        egui::Panel::left("sidebar")
            .resizable(true)
            .default_size(260.0)
            .frame(
                egui::Frame::new()
                    .inner_margin(egui::Margin::symmetric(12, 8))
                    .fill(ui.visuals().panel_fill),
            )
            .show_collapsible(ui, &mut sidebar_open, |ui| self.sidebar(ui));
        self.workspace.sidebar_open = sidebar_open;
        self.update_bar(ui);
        self.window_ui(ui, 0);
        for index in 1..self.workspace.windows.len() {
            let window = &self.workspace.windows[index];
            let builder = self
                .viewports
                .entry(window.id)
                .or_insert_with(|| {
                    let title = window_title(
                        &self.content.root,
                        self.titled_branch.as_deref().unwrap_or_default(),
                    );
                    viewport(&self.content.root, window).with_title(title)
                })
                .clone();
            ctx.show_viewport_immediate(viewport_id(window), builder, |ui, _| {
                self.window_ui(ui, index)
            });
        }

        let mut check = false;
        if self.show_keys {
            let mut system_fonts = self.settings.system_fonts;
            let modal = egui::Modal::new(egui::Id::unique("keys")).show(&ctx, |ui| {
                check = keys::ui(ui, &mut system_fonts);
            });
            if modal.should_close() {
                self.show_keys = false;
            }
            if system_fonts != self.settings.system_fonts {
                self.settings.system_fonts = system_fonts;
                self.apply_fonts(&ctx);
                self.save_settings();
            }
        }
        if check {
            self.updates.start();
        }
        self.drag_title(&ctx);
        if self.content.review.take_changed() {
            self.watcher.refresh_now();
        }
        for path in self.content.review.take_rolled_back() {
            self.content.open_files.reload(&path);
        }
        self.content.open_files.save_edited(&ctx);
        self.content.open_files.poll_language_servers();
        if let Some(hash) = self.content.open_files.take_commit_to_open() {
            self.action = Some(Action::ShowCommit(hash));
        }
        if let Some(reveal) = self.content.open_files.take_reveal() {
            self.action = Some(Action::Reveal(reveal));
        }
        if let Some(text) = self.checks.take_prompt() {
            self.action = Some(Action::Send {
                agent: None,
                text,
                window: 0,
                images: Vec::new(),
            });
        }

        if let Some(action) = self.action.take() {
            self.perform(action, &ctx);
            ctx.request_repaint();
        }
        self.workspace.prune();
        let ranges: HashSet<CommitRange> = self
            .workspace
            .windows
            .iter()
            .flat_map(|window| window.tabs())
            .filter_map(|tab| match tab {
                Tab::CommitReview { range, .. } => Some(range.clone()),
                _ => None,
            })
            .collect();
        self.content
            .commits
            .retain(|range, _| ranges.contains(range));
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
        std::thread::spawn(|| {
            std::thread::sleep(EXIT_GRACE);
            std::process::exit(0);
        });
        self.content.open_files.save_all();
        self.content.open_files.stop_language_servers();
        self.save_layout();
        self.content.terminals.shutdown();
        if let Some(control) = &self.control
            && !self.restarting
        {
            control::stop_leftovers(&control.socket());
        }
        self.quick_chat = None;
        self.control = None;
        std::process::exit(0);
    }
}

#[cfg(test)]
mod tests;
