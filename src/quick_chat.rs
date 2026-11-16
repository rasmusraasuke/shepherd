use eframe::egui::{self, Align2, Key, Modifiers};

use crate::agents::{self, Agent};

pub struct QuickChat {
    message: String,
    send: bool,
}

pub enum Outcome {
    Send(Agent, String),
    Closed,
}

impl QuickChat {
    pub fn new() -> Self {
        Self {
            message: String::new(),
            send: false,
        }
    }

    pub fn ui(
        &mut self,
        ctx: &egui::Context,
        found: &[Agent],
        chosen: &mut Option<Agent>,
    ) -> Option<Outcome> {
        let agent = agents::chosen(found, *chosen);
        if ctx.input_mut(|i| !i.modifiers.shift && i.consume_key(Modifiers::NONE, Key::Enter)) {
            self.send = true;
        }
        let id = egui::Id::unique("quick_chat");
        let area = egui::Modal::default_area(id).anchor(Align2::CENTER_TOP, [0.0, 120.0]);
        let modal = egui::Modal::new(id).area(area).show(ctx, |ui| {
            ui.set_width(440.0_f32.min(ctx.content_rect().width() - 40.0));
            let selected = agent.map_or("no agent found", Agent::name);
            ui.menu_button(selected, |ui| {
                for agent in found {
                    if ui
                        .selectable_label(
                            agents::chosen(found, *chosen) == Some(*agent),
                            agent.name(),
                        )
                        .clicked()
                    {
                        *chosen = Some(*agent);
                        ui.close();
                    }
                }
            });
            let edit = egui::TextEdit::multiline(&mut self.message)
                .hint_text("What should the agent do?")
                .desired_rows(3)
                .desired_width(f32::INFINITY);
            ui.add(edit).request_focus();
            ui.weak(if found.is_empty() {
                "No agent was found: install Claude Code or Opencode".to_string()
            } else {
                "Enter sends, Shift+Enter makes a new line, Esc closes".to_string()
            });
        });
        let send = std::mem::take(&mut self.send);
        let message = self.message.trim().to_string();
        match (send, agent) {
            (true, Some(agent)) if !message.is_empty() => Some(Outcome::Send(agent, message)),
            _ if modal.should_close() => Some(Outcome::Closed),
            _ => None,
        }
    }
}
