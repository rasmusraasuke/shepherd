use std::collections::HashMap;
use std::path::PathBuf;

use eframe::egui::{self, Align2, Key, Modifiers};

use crate::agents::{self, Agent, Choice, Model};
use crate::images;

pub struct QuickChat {
    message: String,
    send: bool,
    image_dir: PathBuf,
    images: Vec<PathBuf>,
}

pub enum Outcome {
    Send {
        agent: Agent,
        text: String,
        images: Vec<PathBuf>,
    },
    Closed,
}

impl QuickChat {
    pub fn new(image_dir: PathBuf) -> Self {
        Self {
            message: String::new(),
            send: false,
            image_dir,
            images: Vec::new(),
        }
    }

    pub fn ui(
        &mut self,
        ctx: &egui::Context,
        found: &[Agent],
        models: &HashMap<Agent, Vec<Model>>,
        choice: &mut Choice,
    ) -> Option<Outcome> {
        let agent = choice.agent(found);
        let field = egui::Id::unique("quick_chat_field");
        let typing = ctx.memory(|memory| memory.has_focus(field));
        self.add_images(ctx, agent, typing);
        if ctx.input_mut(|i| !i.modifiers.shift && i.consume_key(Modifiers::NONE, Key::Enter)) {
            self.send = true;
        }
        let id = egui::Id::unique("quick_chat");
        let area = egui::Modal::default_area(id).anchor(Align2::CENTER_TOP, [0.0, 120.0]);
        let modal = egui::Modal::new(id).area(area).show(ctx, |ui| {
            ui.set_width(440.0_f32.min(ctx.content_rect().width() - 40.0));
            agents::choice_ui(ui, found, models, choice);
            let edit = egui::TextEdit::multiline(&mut self.message)
                .id(field)
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
            (true, Some(agent)) if !message.is_empty() => Some(Outcome::Send {
                agent,
                text: message,
                images: std::mem::take(&mut self.images),
            }),
            _ if modal.should_close() => Some(Outcome::Closed),
            _ => None,
        }
    }

    fn add_images(&mut self, ctx: &egui::Context, agent: Option<Agent>, typing: bool) {
        let mut paths: Vec<PathBuf> = ctx.input(|input| {
            input
                .raw
                .dropped_files
                .iter()
                .map(|file| file.path().to_path_buf())
                .filter(|path| images::is_image(path))
                .collect()
        });
        if typing {
            for picture in images::take_pasted(ctx) {
                if let Ok(path) = images::save(&self.image_dir, &picture) {
                    paths.push(path.clone());
                    self.images.push(path);
                }
            }
        }
        if !paths.is_empty() {
            let text = images::hand(agent, &paths);
            ctx.input_mut(|i| i.events.push(egui::Event::Paste(text)));
        }
    }
}

impl Drop for QuickChat {
    fn drop(&mut self) {
        images::remove(&self.images);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use egui_kittest::Harness;

    use super::*;
    use crate::testing::{Dropped, TempDir};

    struct State {
        chat: QuickChat,
        choice: Choice,
        found: Vec<Agent>,
        models: HashMap<Agent, Vec<Model>>,
        outcome: Option<Outcome>,
    }

    fn picture() -> Arc<egui::ColorImage> {
        Arc::new(egui::ColorImage::new([1, 1], vec![egui::Color32::WHITE]))
    }

    fn harness(name: &str) -> (Harness<'static, State>, TempDir) {
        let dir = TempDir::new(name);
        let state = State {
            chat: QuickChat::new(dir.path().join("images")),
            choice: Choice {
                agent: Some(Agent::Claude),
                model: None,
            },
            found: vec![Agent::Claude],
            models: HashMap::from([(Agent::Claude, Agent::Claude.models(&HashMap::new()))]),
            outcome: None,
        };
        let mut harness = Harness::builder()
            .with_size(egui::vec2(600.0, 400.0))
            .build_ui_state(
                |ui, state: &mut State| {
                    if let Some(outcome) =
                        state
                            .chat
                            .ui(ui.ctx(), &state.found, &state.models, &mut state.choice)
                    {
                        state.outcome = Some(outcome);
                    }
                },
                state,
            );
        harness.run();
        (harness, dir)
    }

    fn pasted_images(harness: &Harness<'_, State>) -> Vec<PathBuf> {
        harness.state().chat.images.clone()
    }

    #[test]
    fn takes_a_pasted_picture_as_the_path_of_a_file() {
        let (mut harness, dir) = harness("quick-chat-image");
        harness.event(egui::Event::Text("look at ".to_string()));
        harness.run();
        harness.event(egui::Event::PasteImage(picture()));
        harness.run();

        let images = pasted_images(&harness);
        assert_eq!(images.len(), 1);
        assert!(images[0].starts_with(dir.path().join("images")));
        let message = harness.state().chat.message.clone();
        assert!(message.starts_with("look at "), "{message}");
        assert!(message.ends_with(images[0].to_str().unwrap()), "{message}");

        harness.key_press(Key::Enter);
        harness.run();
        let Some(Outcome::Send { text, images, .. }) = harness.state_mut().outcome.take() else {
            panic!("the chat didn't send");
        };
        assert_eq!(text, message.trim());
        assert_eq!(images.len(), 1);
        assert!(images[0].exists());
    }

    #[test]
    fn takes_a_dropped_picture_as_its_own_path() {
        let (mut harness, dir) = harness("quick-chat-image-drop");
        let picture = dir.write("shot.png", "picture");
        let notes = dir.write("notes.txt", "text");
        harness
            .input_mut()
            .dropped_files
            .push(Arc::new(Dropped(picture.clone())));
        harness
            .input_mut()
            .dropped_files
            .push(Arc::new(Dropped(notes)));
        harness.step();

        assert_eq!(harness.state().chat.message, picture.display().to_string());
        assert!(pasted_images(&harness).is_empty());
    }

    #[test]
    fn pastes_text_as_text_and_writes_no_picture() {
        let (mut harness, _dir) = harness("quick-chat-image-text");
        harness.event(egui::Event::Paste("hello".to_string()));
        harness.run();
        assert_eq!(harness.state().chat.message, "hello");
        assert!(pasted_images(&harness).is_empty());
    }

    #[test]
    fn removes_the_pictures_it_wrote_when_it_closes_unsent() {
        let (mut harness, _dir) = harness("quick-chat-image-close");
        harness.event(egui::Event::PasteImage(picture()));
        harness.run();
        let images = pasted_images(&harness);
        assert_eq!(images.len(), 1);
        assert!(images[0].exists());
        drop(harness);
        assert!(!images[0].exists());
    }
}
