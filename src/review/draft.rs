use eframe::egui::containers::menu::MenuButton;
use eframe::egui::{self, Key, Modifiers};

use crate::ui::theme;

use super::{Agents, View};

pub(super) enum Draft {
    Comment,
    Ask(Option<u64>),
    Revert,
    Cancel,
}

pub(super) fn draft_ui(
    ui: &mut egui::Ui,
    id: egui::Id,
    view: &mut View,
    agents: &Agents,
) -> Option<Draft> {
    let focused = ui.memory(|m| m.has_focus(id));
    let submit = focused && ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Enter));
    ui.add_space(4.0);
    let edit = egui::TextEdit::multiline(&mut view.draft)
        .id(id)
        .desired_rows(3)
        .desired_width(f32::INFINITY)
        .hint_text("A comment on the selected lines, or what an agent should change there");
    let response = ui.add(edit);
    if std::mem::take(&mut view.focus_draft) {
        response.request_focus();
    }
    let ready = !view.draft.trim().is_empty();
    let mut draft = None;
    ui.horizontal(|ui| {
        if ui
            .add_enabled(ready, theme::primary(ui, "Comment"))
            .on_hover_text("Cmd+Enter")
            .clicked()
            || (ready && submit)
        {
            draft = Some(Draft::Comment);
        }
        if let Some(agent) = send_button(ui, "Ask agent", ready, false, agents) {
            draft = Some(Draft::Ask(agent));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add(theme::quiet("Cancel"))
                .on_hover_text("Esc")
                .clicked()
                || ((focused || response.lost_focus()) && ui.input(|i| i.key_pressed(Key::Escape)))
            {
                draft = Some(Draft::Cancel);
            }
            if ui
                .add(theme::quiet("Revert"))
                .on_hover_text("Take the selected changes back to how they were committed")
                .clicked()
            {
                draft = Some(Draft::Revert);
            }
        });
    });
    ui.add_space(4.0);
    draft
}

pub(super) fn send_button(
    ui: &mut egui::Ui,
    label: &str,
    enabled: bool,
    primary: bool,
    agents: &Agents,
) -> Option<Option<u64>> {
    let button = |ui: &egui::Ui| {
        if primary {
            theme::primary(ui, label)
        } else {
            egui::Button::new(label)
        }
    };
    match (agents.open.as_slice(), agents.new.as_deref()) {
        ([], None) => {
            ui.add_enabled(false, button(ui))
                .on_disabled_hover_text("No agent was found: install Claude Code or Opencode");
            None
        }
        ([], Some(name)) => ui
            .add_enabled(enabled, button(ui))
            .on_hover_text(format!("Start {name} with it"))
            .clicked()
            .then_some(None),
        ([(agent, title)], _) => ui
            .add_enabled(enabled, button(ui))
            .on_hover_text(format!("Send to {title}"))
            .clicked()
            .then_some(Some(*agent)),
        (open, _) => {
            let mut chosen = None;
            ui.add_enabled_ui(enabled, |ui| {
                MenuButton::from_button(button(ui)).ui(ui, |ui| {
                    for (agent, title) in open {
                        if ui.button(title).clicked() {
                            chosen = Some(Some(*agent));
                        }
                    }
                })
            });
            chosen
        }
    }
}

pub fn new_agent_button(
    ui: &mut egui::Ui,
    label: &str,
    enabled: bool,
    busy: Option<&str>,
    agents: &Agents,
) -> bool {
    let Some(name) = agents.new.as_deref() else {
        ui.add_enabled(false, egui::Button::new(label))
            .on_disabled_hover_text("No agent was found: install Claude Code or Opencode");
        return false;
    };
    let spinner = egui::IdSalt::new("spinner");
    let mut button = egui::Button::new(label);
    if busy.is_some() {
        button = button.left_text(egui::Atom::custom(spinner, egui::vec2(12.0, 12.0)));
    }
    let shown = ui
        .add_enabled_ui(enabled && busy.is_none(), |ui| button.atom_ui(ui))
        .inner;
    if let Some(rect) = shown.rect(spinner) {
        theme::spinner(ui, rect);
    }
    shown
        .response
        .on_hover_text(format!("Start {name} with it"))
        .on_disabled_hover_text(busy.unwrap_or(""))
        .clicked()
}
