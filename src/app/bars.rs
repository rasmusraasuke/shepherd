use eframe::egui::{self, ViewportCommand};

use super::workspace::{Sidebar, Tab};
use super::{Action, App};
use crate::panels::file_tree::FileLook;
use crate::panels::flow;
use crate::review;
use crate::ui::icons::{self, Icon};
use crate::ui::status;
use crate::ui::theme;
use crate::update;

const TITLE_HEIGHT: f32 = 31.0;

pub(super) const TITLE_INSET: i8 = if cfg!(target_os = "macos") { 76 } else { 8 };

const ACTIVITY_ROW: f32 = 40.0;

const STATUS_WIDTH: f32 = 480.0;

const STATUS_GAP: f32 = 16.0;

impl App {
    pub(super) fn title_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("title")
            .resizable(false)
            .exact_size(TITLE_HEIGHT)
            .frame(
                egui::Frame::new()
                    .fill(ui.visuals().panel_fill)
                    .inner_margin(egui::Margin {
                        left: TITLE_INSET,
                        ..Default::default()
                    }),
            )
            .show(ui, |ui| {
                let drag = ui.interact(
                    ui.max_rect(),
                    egui::Id::unique("title drag"),
                    egui::Sense::click_and_drag(),
                );
                if drag.drag_started_by(egui::PointerButton::Primary) {
                    ui.send_viewport_cmd(ViewportCommand::StartDrag);
                }
                if drag.double_clicked() {
                    let maximized = ui.input(|i| i.viewport().maximized.unwrap_or(false));
                    ui.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
                }
                let bar = ui.max_rect();
                let (project, op, left) = ui
                    .horizontal_centered(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        let project = self.projects.ui(ui);
                        let op = self.content.flow.branch_ui(ui);
                        (project, op, ui.min_rect().right())
                    })
                    .inner;
                let right = self.checks.ui(ui, bar);
                self.status_ui(ui, bar, left, right);
                if let Some(op) = op {
                    self.content.run(op, ui.ctx());
                }
                if let Some(project) = project
                    && let Err(error) = update::launch(&project)
                {
                    self.projects.fail(error);
                }
            });
    }

    fn statuses(&mut self, ctx: &egui::Context) -> (Vec<status::Item>, bool) {
        let git = self.content.flow.activity(ctx);
        let first_is_git = git.is_some();
        let mut items: Vec<status::Item> = git.into_iter().collect();
        if let update::Phase::Downloading(release) = self.updates.phase() {
            items.push(status::Item::busy(format!(
                "Downloading Shepherd {}…",
                release.version()
            )));
        }
        items.extend(self.content.open_files.statuses());
        (items, first_is_git)
    }

    fn status_ui(&mut self, ui: &mut egui::Ui, bar: egui::Rect, left: f32, right: f32) {
        let (items, first_is_git) = self.statuses(ui.ctx());
        let middle = egui::pos2(ui.ctx().content_rect().center().x, bar.center().y);
        let centered = egui::Rect::from_center_size(middle, egui::vec2(STATUS_WIDTH, bar.height()));
        let start = left + STATUS_GAP;
        let end = right - STATUS_GAP;
        let area = if centered.left() >= start && centered.right() <= end {
            centered
        } else {
            egui::Rect::from_x_y_ranges(start..=(start + STATUS_WIDTH).min(end), bar.y_range())
        };
        if status::ui(ui, &items, area) == Some(0) && first_is_git {
            self.content.flow.dismiss();
        }
    }

    pub(super) fn update_bar(&mut self, ui: &mut egui::Ui) {
        let phase = self.updates.phase();
        let build = self.build.changed();
        if !build {
            ui.ctx().request_repaint_after(update::CHECK);
        }
        if matches!(
            phase,
            update::Phase::Idle | update::Phase::Checking | update::Phase::Downloading(_)
        ) && !build
        {
            return;
        }
        egui::Panel::bottom("update")
            .resizable(false)
            .show_separator_line(false)
            .frame(
                egui::Frame::new()
                    .fill(ui.visuals().panel_fill)
                    .inner_margin(egui::Margin::symmetric(12, 6)),
            )
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| match &phase {
                    update::Phase::Ready(release) => {
                        let install = update::bundled().is_some();
                        ui.label(if install {
                            format!("Shepherd {} is ready to install.", release.version())
                        } else {
                            format!("Shepherd {} has been downloaded.", release.version())
                        });
                        if install {
                            if ui.add(theme::primary(ui, "Restart and update")).clicked() {
                                match self.updates.install() {
                                    Ok(()) => self.restart_now(ui.ctx()),
                                    Err(e) => self.updates.fail(e),
                                }
                            }
                        } else if ui.add(theme::primary(ui, "Show the download")).clicked()
                            && let Err(e) = self.updates.reveal()
                        {
                            self.updates.fail(e);
                        }
                        if ui.add(theme::quiet("Later")).clicked() {
                            self.updates.dismiss();
                        }
                    }
                    update::Phase::Failed(error) => {
                        ui.colored_label(ui.visuals().error_fg_color, error);
                        if ui.add(theme::quiet("Dismiss")).clicked() {
                            self.updates.dismiss();
                        }
                    }
                    update::Phase::Idle
                    | update::Phase::Checking
                    | update::Phase::Downloading(_) => {
                        if build {
                            ui.label("A new build of Shepherd is on disk.");
                            if ui
                                .add(theme::primary(ui, "Restart"))
                                .on_hover_text(
                                    "Files and the layout are saved. Terminals and running agents close.",
                                )
                                .clicked()
                            {
                                self.restart_now(ui.ctx());
                            }
                            if ui.add(theme::quiet("Later")).clicked() {
                                self.build.dismiss();
                            }
                        }
                    }
                });
            });
    }

    fn restart_now(&mut self, ctx: &egui::Context) {
        self.restarting = true;
        self.content.open_files.save_all();
        self.save_layout();
        match update::launch(&self.content.root) {
            Ok(()) => ctx.send_viewport_cmd(ViewportCommand::Close),
            Err(e) => self.updates.fail(e),
        }
    }

    pub(super) fn activity_bar(&mut self, ui: &mut egui::Ui) {
        let width = ui.available_width();
        let button = |ui: &mut egui::Ui, icon: Icon, label: &str, chosen: bool| {
            let (rect, response) =
                ui.allocate_exact_size(egui::vec2(width, ACTIVITY_ROW), egui::Sense::click());
            response.widget_info(|| {
                egui::WidgetInfo::selected(egui::Role::Button, true, chosen, label)
            });
            let visuals = ui.visuals();
            let color = if chosen || response.hovered() {
                visuals.strong_text_color()
            } else {
                visuals.weak_text_color()
            };
            if chosen {
                let bar = egui::Rect::from_min_size(
                    rect.left_top() + egui::vec2(0.0, 8.0),
                    egui::vec2(2.0, rect.height() - 16.0),
                );
                ui.painter()
                    .rect_filled(bar, 1.0, theme::palette(visuals.dark_mode).accent);
            }
            icons::paint(ui.painter(), rect.center(), icons::BAR_SIZE, icon, color);
            response.on_hover_text(label).clicked()
        };
        ui.add_space(6.0);
        for (sidebar, icon, label) in [
            (Sidebar::Files, Icon::Files, "Files"),
            (Sidebar::Changes, Icon::Review, "Changes"),
            (Sidebar::History, Icon::History, "History"),
        ] {
            let chosen = self.workspace.sidebar_open && self.workspace.sidebar == sidebar;
            if button(ui, icon, label, chosen) {
                self.workspace.toggle_sidebar(sidebar);
            }
        }
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
            ui.add_space(6.0);
            if button(ui, Icon::Keys, "Keyboard shortcuts", self.show_keys) {
                self.show_keys = true;
            }
        });
    }

    pub(super) fn sidebar(&mut self, ui: &mut egui::Ui) {
        ui.set_min_width(ui.available_width());
        ui.add_space(8.0);
        if let Some(error) = &self.content.error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        match self.workspace.sidebar {
            Sidebar::Files => {
                let dark_mode = ui.visuals().dark_mode;
                let review = &self.content.review;
                let look = |path: &str| FileLook {
                    color: review
                        .status(path)
                        .map(|status| review::status_color(status, dark_mode)),
                    ..Default::default()
                };
                let mut history = None;
                let mut menu = |ui: &mut egui::Ui, path: &str| {
                    if ui.button("History").clicked() {
                        history = Some(path.to_string());
                        ui.close();
                    }
                };
                if let Some(path) = self.content.file_tree.ui(
                    ui,
                    &mut self.workspace.expanded_dirs,
                    None,
                    look,
                    Some(&mut menu),
                    None,
                ) {
                    self.action = Some(Action::Open(Tab::File(path), 0));
                }
                if let Some(path) = history {
                    self.action = Some(Action::FileHistory(path));
                }
            }
            Sidebar::Changes => {
                let agents = self.draft_agents();
                let choice = &mut self.workspace.draft_agent;
                let content = &mut self.content;
                match content.flow.ui(
                    ui,
                    content.review.progress(),
                    &agents,
                    &self.agents,
                    &self.models,
                    choice,
                ) {
                    Some(flow::Outcome::Run(op)) => content.run(op, ui.ctx()),
                    Some(flow::Outcome::Draft { text }) => {
                        self.action = Some(Action::Draft { text })
                    }
                    Some(flow::Outcome::Mark(reviewed)) => {
                        content.review.set_reviewed("", reviewed, ui.ctx())
                    }
                    None => {}
                }
                let selected = self
                    .workspace
                    .last_review
                    .as_ref()
                    .and_then(Tab::working_path);
                if let Some(path) = self.content.review.tree_ui(
                    ui,
                    &mut self.workspace.collapsed_changes,
                    selected,
                    true,
                ) {
                    let new_tab = ui.input(|i| i.modifiers.command);
                    self.action = Some(Action::Review { path, new_tab });
                }
            }
            Sidebar::History => self.history_sidebar(ui),
        }
    }

    fn history_sidebar(&mut self, ui: &mut egui::Ui) {
        use crate::panels::history::Outcome;
        if let Some(outcome) = self.content.history.ui(ui) {
            let Outcome::Open {
                range,
                label,
                file,
                new_tab,
            } = outcome;
            self.action = Some(Action::OpenCommit {
                range,
                label,
                file,
                new_tab,
            });
        }
        let selected = self.content.history.selected();
        let Some((range, label)) =
            selected.map(|(range, label)| (range.clone(), label.to_string()))
        else {
            return;
        };
        ui.add_space(6.0);
        ui.separator();
        ui.add_space(4.0);
        self.content.history.details_ui(ui);
        let Some(review) = self.content.commits.get_mut(&range) else {
            return;
        };
        ui.add_space(6.0);
        ui.separator();
        ui.add_space(4.0);
        let current = match &self.workspace.last_review {
            Some(Tab::CommitReview {
                path, range: shown, ..
            }) if *shown == range => path.as_deref(),
            _ => None,
        };
        if let Some(path) =
            review.tree_ui(ui, &mut self.workspace.collapsed_changes, current, false)
        {
            let new_tab = ui.input(|i| i.modifiers.command);
            self.action = Some(Action::OpenCommit {
                range,
                label,
                file: Some(path),
                new_tab,
            });
        }
    }
}
