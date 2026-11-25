use eframe::egui;
use eframe::egui::containers::menu::MenuButton;
use egui_tiles::{SimplificationOptions, TileId, Tiles, UiResponse};

use super::workspace::{Closing, Tab};
use super::{Action, Content};
use crate::agents::Agent;
use crate::editor::buffer::Pos;
use crate::panels::search::Reveal;
use crate::review::{self, Review};
use crate::terminal::Activity;
use crate::ui::{cursor, theme};

const WAITING_COLOR: egui::Color32 = egui::Color32::from_rgb(214, 160, 92);

pub(super) struct TabBehavior<'a> {
    pub(super) content: &'a mut Content,
    pub(super) agents: &'a review::Agents,
    pub(super) found: &'a [Agent],
    pub(super) font: egui::FontId,
    pub(super) window: usize,
    pub(super) window_count: usize,
    pub(super) dragged_out: bool,
    pub(super) focused: &'a mut Option<TileId>,
    pub(super) last_review: &'a mut Option<Tab>,
    pub(super) action: &'a mut Option<Action>,
}

impl egui_tiles::Behavior<Tab> for TabBehavior<'_> {
    fn pane_ui(&mut self, ui: &mut egui::Ui, tile_id: TileId, tab: &mut Tab) -> UiResponse {
        if ui.rect_contains_pointer(ui.max_rect()) && ui.input(|i| i.pointer.any_pressed()) {
            *self.focused = Some(tile_id);
            if tab.is_review() {
                *self.last_review = Some(tab.clone());
            }
        }
        let active = self.last_review.as_ref().is_none_or(|last| last == tab);
        match tab {
            Tab::Review(path) => {
                let outcome = egui::Frame::new()
                    .inner_margin(8.0)
                    .show(ui, |ui| {
                        self.content.review.ui(
                            ui,
                            path.as_deref(),
                            active,
                            self.agents,
                            &mut self.content.open_files,
                        )
                    })
                    .inner;
                review_outcome(self, outcome, None);
            }
            Tab::CommitReview { path, range, label } => {
                let Content {
                    commits,
                    open_files,
                    repo,
                    ..
                } = &mut self.content;
                let review = commits.entry(range.clone()).or_insert_with(|| {
                    Review::for_commits(repo.clone(), range.clone(), label.clone())
                });
                let outcome = egui::Frame::new()
                    .inner_margin(8.0)
                    .show(ui, |ui| {
                        review.ui(ui, path.as_deref(), active, self.agents, open_files)
                    })
                    .inner;
                review_outcome(self, outcome, Some((range.clone(), label.clone())));
            }
            Tab::File(path) => {
                if let Some(line) = self.content.open_files.ui(ui, path) {
                    *self.action = Some(Action::ReviewLine {
                        path: path.clone(),
                        line,
                    });
                }
            }
            Tab::Terminal(id) => self.content.terminals.ui(ui, *id),
            Tab::Agent {
                id,
                agent,
                session,
                tuning,
            } => {
                if let Some(new) = self
                    .content
                    .terminals
                    .agent_ui(ui, *id, *agent, session, tuning)
                {
                    *session = new;
                }
            }
        }
        UiResponse::None
    }

    fn tab_title_for_pane(&mut self, tab: &Tab) -> egui::WidgetText {
        let id = match tab {
            Tab::Terminal(id) => *id,
            Tab::Agent { id, .. } => *id,
            _ => return tab.title().into(),
        };
        let terminals = &self.content.terminals;
        let title = terminals.title(id).unwrap_or_else(|| tab.title());
        let dot = match terminals.activity(id) {
            Some(Activity::Working) => review::NEW_COLOR,
            Some(Activity::Waiting) => WAITING_COLOR,
            _ => return title.into(),
        };
        let mut job = egui::text::LayoutJob::default();
        job.append("● ", 0.0, egui::TextFormat::simple(self.font.clone(), dot));
        job.append(
            &title,
            0.0,
            egui::TextFormat::simple(self.font.clone(), egui::Color32::PLACEHOLDER),
        );
        job.into()
    }

    fn top_bar_right_ui(
        &mut self,
        _tiles: &Tiles<Tab>,
        ui: &mut egui::Ui,
        tile_id: TileId,
        _tabs: &egui_tiles::Tabs,
        _scroll_offset: &mut f32,
    ) {
        MenuButton::from_button(theme::quiet("+")).ui(ui, |ui| {
            if ui.button("New terminal (Cmd+T)").clicked() {
                *self.action = Some(Action::NewTerminal(self.window, Some(tile_id)));
            }
            for agent in self.found {
                if ui.button(format!("New {} agent", agent.name())).clicked() {
                    *self.action = Some(Action::NewAgent(self.window, Some(tile_id), *agent));
                }
            }
            if ui.button("New review").clicked() {
                *self.action = Some(Action::NewReview(self.window, Some(tile_id)));
            }
        });
    }

    fn tab_bar_height(&self, _style: &egui::Style) -> f32 {
        34.0
    }

    fn tab_title_spacing(&self, _visuals: &egui::Visuals) -> f32 {
        12.0
    }

    fn close_button_outer_size(&self) -> f32 {
        10.0
    }

    fn close_button_inner_margin(&self) -> f32 {
        1.5
    }

    fn gap_width(&self, _style: &egui::Style) -> f32 {
        1.0
    }

    fn resize_stroke(
        &self,
        style: &egui::Style,
        resize_state: egui_tiles::ResizeState,
    ) -> egui::Stroke {
        match resize_state {
            egui_tiles::ResizeState::Idle => style.visuals.widgets.noninteractive.bg_stroke,
            _ => style.visuals.selection.stroke,
        }
    }

    fn tab_bar_color(&self, visuals: &egui::Visuals) -> egui::Color32 {
        visuals.panel_fill
    }

    fn tab_bg_color(
        &self,
        _visuals: &egui::Visuals,
        _tiles: &Tiles<Tab>,
        _tile_id: TileId,
        _state: &egui_tiles::TabState,
    ) -> egui::Color32 {
        egui::Color32::TRANSPARENT
    }

    fn tab_outline_stroke(
        &self,
        _visuals: &egui::Visuals,
        _tiles: &Tiles<Tab>,
        _tile_id: TileId,
        _state: &egui_tiles::TabState,
    ) -> egui::Stroke {
        egui::Stroke::NONE
    }

    fn tab_bar_hline_stroke(&self, visuals: &egui::Visuals) -> egui::Stroke {
        visuals.widgets.noninteractive.bg_stroke
    }

    fn tab_text_color(
        &self,
        visuals: &egui::Visuals,
        _tiles: &Tiles<Tab>,
        _tile_id: TileId,
        state: &egui_tiles::TabState,
    ) -> egui::Color32 {
        if state.active {
            visuals.strong_text_color()
        } else {
            visuals.weak_text_color()
        }
    }

    fn is_tab_closable(&self, _tiles: &Tiles<Tab>, _tile_id: TileId) -> bool {
        true
    }

    fn on_tab_close(&mut self, tiles: &mut Tiles<Tab>, tile_id: TileId) -> bool {
        if let Some(tab) = tiles.get_pane(&tile_id) {
            self.content.close(tab);
        }
        true
    }

    fn on_tab_button(
        &mut self,
        tiles: &mut Tiles<Tab>,
        tile_id: TileId,
        response: egui::Response,
    ) -> egui::Response {
        cursor::tab(&response);
        let Some(tab) = tiles.get_pane(&tile_id) else {
            return response;
        };
        if response.clicked() {
            *self.focused = Some(tile_id);
            if tab.is_review() {
                *self.last_review = Some(tab.clone());
            }
        }
        let history = match tab {
            Tab::File(path) => Some(path.clone()),
            _ => None,
        };
        response.context_menu(|ui| {
            if let Some(path) = &history
                && ui.button("File history").clicked()
            {
                *self.action = Some(Action::FileHistory(path.clone()));
                ui.close();
            }
            if history.is_some() {
                ui.separator();
            }
            let mut move_to = |ui: &mut egui::Ui, label: String, to: Option<usize>| {
                if ui.button(label).clicked() {
                    *self.action = Some(Action::Move {
                        from: self.window,
                        tile: tile_id,
                        to,
                    });
                    ui.close();
                }
            };
            move_to(ui, "Move to new window".to_string(), None);
            for to in (0..self.window_count).filter(|&to| to != self.window) {
                let label = if to == 0 {
                    "Move to main window".to_string()
                } else {
                    format!("Move to window {}", to + 1)
                };
                move_to(ui, label, Some(to));
            }
            ui.separator();
            let closings = [
                ("Close others", Closing::Others),
                (
                    "Close others, keep agents and terminals",
                    Closing::OthersButTerminals,
                ),
                ("Close all files", Closing::Files),
                ("Close all", Closing::All),
            ];
            for (label, closing) in closings {
                if ui.button(label).clicked() {
                    *self.action = Some(Action::CloseTabs {
                        window: self.window,
                        tile: tile_id,
                        closing,
                    });
                    ui.close();
                }
            }
        });
        match tab.path() {
            Some(path) => response.on_hover_text(path),
            None => response,
        }
    }

    fn drag_ui(&mut self, _tiles: &Tiles<Tab>, _ui: &mut egui::Ui, _tile_id: TileId) {}

    fn paint_drag_preview(
        &self,
        visuals: &egui::Visuals,
        painter: &egui::Painter,
        parent_rect: Option<egui::Rect>,
        preview_rect: egui::Rect,
    ) {
        if self.dragged_out {
            return;
        }
        let stroke = self.drag_preview_stroke(visuals);
        if let Some(parent_rect) = parent_rect {
            painter.rect_stroke(parent_rect, 1.0, stroke, egui::StrokeKind::Inside);
        }
        painter.rect(
            preview_rect,
            1.0,
            self.drag_preview_color(visuals),
            stroke,
            egui::StrokeKind::Inside,
        );
    }

    fn simplification_options(&self) -> SimplificationOptions {
        SimplificationOptions {
            all_panes_must_have_tabs: true,
            ..Default::default()
        }
    }
}

fn review_outcome(
    behavior: &mut TabBehavior<'_>,
    outcome: Option<review::Outcome>,
    commit: Option<(crate::git::CommitRange, String)>,
) {
    match outcome {
        Some(review::Outcome::Show(path)) => {
            *behavior.action = Some(match commit {
                Some((range, label)) => Action::OpenCommit {
                    range,
                    label,
                    file: Some(path),
                    new_tab: false,
                },
                None => Action::Review {
                    path,
                    new_tab: false,
                },
            });
        }
        Some(review::Outcome::Send { agent, text }) => {
            *behavior.action = Some(Action::Send {
                agent,
                text,
                window: behavior.window,
                images: Vec::new(),
            });
        }
        Some(review::Outcome::Open { path, line }) => {
            let place = Pos::new(line, 0);
            *behavior.action = Some(Action::Reveal(Reveal {
                path,
                start: place,
                end: place,
                focus: true,
            }));
        }
        None => {}
    }
}
