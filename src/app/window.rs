use super::tabs::TabBehavior;
use std::path::Path;

use eframe::egui::{self, Key, Modifiers, ViewportBuilder, ViewportCommand, ViewportId};

use super::workspace::{Tab, Window};
use super::{Action, App, Drag};
use crate::panels::quick_chat::{self, QuickChat};
use crate::panels::quick_open::{Outcome, QuickOpen};

const DEFAULT_WINDOW_SIZE: egui::Vec2 = egui::vec2(1200.0, 800.0);

const DRAG_TITLE_SHADOW: i8 = 16;

pub(super) fn window_title(root: &Path, branch: &str) -> String {
    let title = format!("Shepherd — {}", root.display());
    if branch.is_empty() {
        title
    } else {
        format!("{title} — ⎇ {branch}")
    }
}

pub fn viewport(root: &Path, window: &Window) -> ViewportBuilder {
    let builder = ViewportBuilder::default()
        .with_title(window_title(root, ""))
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

fn empty_ui(ui: &mut egui::Ui) {
    let keys = [
        ("Open a file", "⌘P"),
        ("Search in files", "⇧⇧"),
        ("New terminal", "⌘T"),
    ];
    let (width, row) = (220.0, 26.0);
    let rect = ui.max_rect();
    let top = rect.center().y - keys.len() as f32 * row / 2.0;
    let font = egui::TextStyle::Body.resolve(ui.style());
    let color = ui.visuals().weak_text_color();
    for (index, (action, key)) in keys.into_iter().enumerate() {
        let y = top + index as f32 * row;
        for (x, align, text) in [
            (
                rect.center().x - width / 2.0,
                egui::Align2::LEFT_TOP,
                action,
            ),
            (rect.center().x + width / 2.0, egui::Align2::RIGHT_TOP, key),
        ] {
            ui.painter()
                .text(egui::pos2(x, y), align, text, font.clone(), color);
        }
    }
}

pub(super) fn viewport_id(window: &Window) -> ViewportId {
    if window.id == 0 {
        ViewportId::ROOT
    } else {
        ViewportId::from_hash_of(("window", window.id))
    }
}

impl App {
    pub(super) fn window_ui(&mut self, ui: &mut egui::Ui, index: usize) {
        let ctx = ui.ctx().clone();
        let window_count = self.workspace.windows.len();
        let agents = self.review_agents();
        let found = self.agents.clone();
        let window = &mut self.workspace.windows[index];
        let window_id = window.id;

        let (outer, inner, close_requested) = ctx.input(|i| {
            (
                i.viewport().outer_rect,
                i.viewport().inner_rect,
                i.viewport().close_requested(),
            )
        });
        window.position = outer.map(|rect| rect.min).or(window.position);
        window.size = inner.map(|rect| rect.size()).or(window.size);
        window.screen_rect = inner.or(window.screen_rect);
        if close_requested && index > 0 {
            self.action = Some(Action::CloseWindow(index));
        }
        let double_shift = ctx.input(|i| i.focused && self.double_shift.follow(&i.events, i.time));
        if double_shift
            || ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::F))
        {
            self.action = Some(Action::Search(window_id));
        }
        let double_ctrl = ctx.input(|i| i.focused && self.double_ctrl.follow(&i.events, i.time));
        if double_ctrl {
            self.quick_chat = Some((
                window_id,
                QuickChat::new(self.content.terminals.image_dir()),
            ));
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
        if ctx.memory(|m| m.focused().is_none())
            && ctx.input(|i| i.modifiers.is_none() && i.key_pressed(Key::R))
        {
            self.watcher.refresh_now();
        }

        let pointer = ctx.input(|i| i.pointer.interact_pos());
        if let (Some(tile), Some(pointer)) = (window.tree.dragged_id(&ctx), pointer) {
            let point = inner
                .map(|inner| inner.min + pointer.to_vec2())
                .filter(|point| inner.is_some_and(|inner| !inner.contains(*point)));
            self.drag = point.map(|point| Drag {
                from: index,
                tile,
                point,
            });
            if ctx.input(|i| i.pointer.any_released())
                && let Some(drag) = self.drag.take()
            {
                self.action = Some(Action::Drop(drag));
            } else if self.drag.is_none()
                && let Some(tab) = window.tree.tiles.get_pane(&tile)
            {
                let area = egui::Area::new(egui::Id::unique("drag title"))
                    .order(egui::Order::Tooltip)
                    .interactable(false)
                    .constrain(false);
                area.pivot(egui::Align2::CENTER_CENTER)
                    .current_pos(pointer)
                    .show(&ctx, |ui| drag_title(ui, &tab.title()));
            }
        } else if self.drag.as_ref().is_some_and(|drag| drag.from == index) {
            self.drag = None;
        }
        let dragged_out = self.drag.as_ref().is_some_and(|drag| drag.from == index);

        let panel = egui::CentralPanel::no_frame()
            .frame(egui::Frame::new().fill(ui.visuals().panel_fill))
            .show(ui, |ui| {
                if window.tabs().is_empty() {
                    empty_ui(ui);
                    return;
                }
                let mut behavior = TabBehavior {
                    font: egui::TextStyle::Button.resolve(ui.style()),
                    agents: &agents,
                    found: &found,
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

        if let Some(drag) = self
            .drag
            .as_ref()
            .filter(|drag| self.workspace.window_at(drag.point, drag.from) == Some(index))
        {
            let rect = self.workspace.windows[index]
                .drop_target(drag.point)
                .map_or(panel.response.rect, |target| target.rect);
            let stroke = ui.visuals().selection.stroke;
            ui.painter().rect(
                rect,
                1.0,
                stroke.color.gamma_multiply(0.5),
                stroke,
                egui::StrokeKind::Inside,
            );
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

        if self
            .quick_chat
            .as_ref()
            .is_some_and(|(id, _)| *id == window_id)
        {
            let outcome = self.quick_chat.as_mut().and_then(|(_, chat)| {
                chat.ui(&ctx, &self.agents, &self.models, &mut self.workspace.agent)
            });
            match outcome {
                Some(quick_chat::Outcome::Send {
                    agent,
                    text,
                    images,
                }) => {
                    self.workspace.agent.agent = Some(agent);
                    self.action = Some(Action::Send {
                        agent: None,
                        text,
                        window: index,
                        images,
                    });
                    self.quick_chat = None;
                }
                Some(quick_chat::Outcome::Closed) => self.quick_chat = None,
                None => {}
            }
        }

        if self.search_in == Some(window_id) {
            let content = &mut self.content;
            let (reveal, closed) = content.search.show(&ctx, &content.root, &content.files);
            if let Some(reveal) = reveal {
                self.action = Some(Action::Reveal(reveal));
            }
            if closed {
                self.search_in = None;
            }
        }
    }

    pub(super) fn focus(&self, window: usize, ctx: &egui::Context) {
        ctx.send_viewport_cmd_to(
            viewport_id(&self.workspace.windows[window]),
            ViewportCommand::Focus,
        );
    }

    pub(super) fn drag_title(&self, ctx: &egui::Context) {
        let Some(drag) = &self.drag else { return };
        let Some(tab) = self.workspace.windows[drag.from]
            .tree
            .tiles
            .get_pane(&drag.tile)
        else {
            return;
        };
        let style = ctx.global_style();
        let text_size = ctx
            .fonts_mut(|fonts| {
                fonts.layout_no_wrap(
                    tab.title(),
                    egui::TextStyle::Body.resolve(&style),
                    egui::Color32::PLACEHOLDER,
                )
            })
            .size();
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
}
