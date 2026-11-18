use std::collections::HashMap;

use eframe::egui::{self, containers::menu::MenuButton};

use super::{Agent, Choice, DEFAULT_MODEL, Model, Tuning};
use crate::ui::icons::{self, Icon};
use crate::ui::theme;

pub(super) fn efforts<'a>(models: Option<&'a Vec<Model>>, tuning: &Tuning) -> &'a [String] {
    let value = tuning.model().unwrap_or(DEFAULT_MODEL);
    models
        .into_iter()
        .flatten()
        .find(|model| model.value == value)
        .map_or(&[], |model| &model.efforts)
}

pub(super) fn model_name(models: Option<&Vec<Model>>, tuning: &Tuning) -> Option<String> {
    let value = tuning.model()?;
    let name = models
        .into_iter()
        .flatten()
        .find(|model| model.value == value)
        .map_or(value, |model| &model.name);
    Some(name.to_string())
}

pub fn choice_ui(
    ui: &mut egui::Ui,
    found: &[Agent],
    models: &HashMap<Agent, Vec<Model>>,
    choice: &mut Choice,
) {
    let agent = choice.agent(found);
    let name = agent.map_or("No agent found", Agent::name);
    let (response, _) = MenuButton::from_button(theme::dropdown(name))
        .ui(ui, |ui| agent_options(ui, found, choice));
    theme::caret(ui, &response);
    let Some(agent) = agent else {
        return;
    };
    let models = models.get(&agent);
    let tuning = choice.tunings.entry(agent).or_default();
    let selected = model_name(models, tuning).unwrap_or("Default model".to_string());
    let (response, _) = MenuButton::from_button(theme::dropdown(selected))
        .ui(ui, |ui| model_options(ui, models, tuning));
    theme::caret(ui, &response);
    response.on_hover_text("Model for new agent sessions");
    let efforts = efforts(models, tuning);
    if !efforts.is_empty() {
        let selected = tuning.effort().unwrap_or("Default effort").to_string();
        let (response, _) = MenuButton::from_button(theme::dropdown(selected))
            .ui(ui, |ui| effort_options(ui, efforts, tuning));
        theme::caret(ui, &response);
        response.on_hover_text("Reasoning effort for new agent sessions");
    }
}

pub fn choice_menu(
    ui: &mut egui::Ui,
    found: &[Agent],
    models: &HashMap<Agent, Vec<Model>>,
    choice: &mut Choice,
) {
    let agent = choice.agent(found);
    let more = egui::Atom::custom(egui::IdSalt::new("more"), egui::vec2(14.0, 14.0));
    let (response, _) = MenuButton::from_button(theme::quiet(more)).ui(ui, |ui| {
        let name = agent.map_or("No agent found", Agent::name);
        ui.menu_button(format!("Harness: {name}"), |ui| {
            agent_options(ui, found, choice)
        });
        if let Some(agent) = agent {
            let models = models.get(&agent);
            let tuning = choice.tunings.entry(agent).or_default();
            let model = model_name(models, tuning).unwrap_or("Default".to_string());
            ui.menu_button(format!("Model: {model}"), |ui| {
                model_options(ui, models, tuning)
            });
            let efforts = efforts(models, tuning);
            if !efforts.is_empty() {
                let effort = tuning.effort().unwrap_or("Default").to_string();
                ui.menu_button(format!("Effort: {effort}"), |ui| {
                    effort_options(ui, efforts, tuning)
                });
            }
        }
    });
    let color = ui.style().interact(&response).fg_stroke.color;
    icons::paint(
        ui.painter(),
        response.rect.center(),
        14.0,
        Icon::More,
        color,
    );
    response.on_hover_text("Harness and model of the agent");
}

fn agent_options(ui: &mut egui::Ui, found: &[Agent], choice: &mut Choice) {
    let agent = choice.agent(found);
    for found_agent in found {
        if ui
            .selectable_label(agent == Some(*found_agent), found_agent.name())
            .clicked()
        {
            choice.agent = Some(*found_agent);
            ui.close();
        }
    }
}

fn model_options(ui: &mut egui::Ui, models: Option<&Vec<Model>>, tuning: &mut Tuning) {
    if ui
        .selectable_label(tuning.model().is_none(), "Default")
        .clicked()
    {
        choose_model(models, tuning, None);
        ui.close();
    }
    let Some(options) = models else {
        return;
    };
    egui::ScrollArea::vertical()
        .max_height(300.0)
        .show(ui, |ui| {
            for model in options.iter().filter(|model| model.value != DEFAULT_MODEL) {
                let chosen = tuning.model() == Some(model.value.as_str());
                if ui.selectable_label(chosen, &model.name).clicked() {
                    choose_model(models, tuning, Some(model.value.clone()));
                    ui.close();
                }
            }
        });
}

pub(super) fn choose_model(
    models: Option<&Vec<Model>>,
    tuning: &mut Tuning,
    model: Option<String>,
) {
    tuning.model = model;
    if !efforts(models, tuning)
        .iter()
        .any(|effort| Some(effort.as_str()) == tuning.effort())
    {
        tuning.effort = None;
    }
}

fn effort_options(ui: &mut egui::Ui, efforts: &[String], tuning: &mut Tuning) {
    if ui
        .selectable_label(tuning.effort().is_none(), "Default")
        .clicked()
    {
        tuning.effort = None;
        ui.close();
    }
    for effort in efforts {
        let chosen = tuning.effort() == Some(effort.as_str());
        if ui.selectable_label(chosen, effort).clicked() {
            tuning.effort = Some(effort.clone());
            ui.close();
        }
    }
}
