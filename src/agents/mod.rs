mod choice;
mod claude;
pub mod images;
mod opencode;

use std::collections::HashMap;
use std::collections::hash_map::RandomState;
use std::fs;
use std::hash::{BuildHasher as _, Hasher as _};
use std::path::Path;

use eframe::egui;
use serde::{Deserialize, Serialize};

pub use choice::{choice_menu, choice_ui};
use claude::{claude_background, claude_command, claude_models, claude_session};
use opencode::{
    OPENCODE_PLUGIN, opencode_background, opencode_command, opencode_models, opencode_plugin,
    opencode_session,
};

pub const GUIDE: &str = "You run in the Shepherd editor, in which the user reviews the changes \
you make. `shepherd ctl help` lists what you can ask the editor: what the user has open and \
selected, their review comments and what they reviewed. It also shows them files and changes.";

pub const AGENT_VARIABLE: &str = "SHEPHERD_AGENT";

const DEFAULT_MODEL: &str = "default";

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum Agent {
    Claude,
    Opencode,
}

#[derive(Clone, PartialEq, Debug)]
pub struct Model {
    pub name: String,
    pub value: String,
    pub efforts: Vec<String>,
}

#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
pub struct Tuning {
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
}

impl Tuning {
    pub fn model(&self) -> Option<&str> {
        self.model.as_deref().filter(|model| !model.is_empty())
    }

    pub fn effort(&self) -> Option<&str> {
        self.effort.as_deref().filter(|effort| !effort.is_empty())
    }

    fn opencode_model(&self) -> Option<String> {
        let model = self.model()?;
        Some(match self.effort() {
            Some(effort) => format!("{model}#{effort}"),
            None => model.to_string(),
        })
    }
}

#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
pub struct Choice {
    #[serde(default)]
    pub agent: Option<Agent>,
    #[serde(default)]
    pub tunings: HashMap<Agent, Tuning>,
}

impl Choice {
    pub fn agent(&self, found: &[Agent]) -> Option<Agent> {
        chosen(found, self.agent)
    }

    pub fn tuning(&self, agent: Agent) -> Tuning {
        self.tunings.get(&agent).cloned().unwrap_or_default()
    }
}

pub enum Start {
    New(Option<String>),
    Continue,
}

pub struct Command {
    pub program: &'static str,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

struct Spec {
    name: &'static str,
    program: &'static str,
    session: fn() -> String,
    command: fn(&str, Start, Option<&Path>, &Tuning) -> Command,
    background: fn(&str, Option<&Path>, &Tuning) -> Command,
    image: fn(&Path) -> String,
}

static CLAUDE: Spec = Spec {
    name: "Claude Code",
    program: "claude",
    session: claude_session,
    command: claude_command,
    background: claude_background,
    image: image_path,
};

static OPENCODE: Spec = Spec {
    name: "Opencode",
    program: "opencode",
    session: opencode_session,
    command: opencode_command,
    background: opencode_background,
    image: image_path,
};

impl Agent {
    const ALL: [Agent; 2] = [Agent::Claude, Agent::Opencode];

    fn spec(self) -> &'static Spec {
        match self {
            Agent::Claude => &CLAUDE,
            Agent::Opencode => &OPENCODE,
        }
    }

    pub fn name(self) -> &'static str {
        self.spec().name
    }

    fn program(self) -> &'static str {
        self.spec().program
    }

    pub fn new_session(self) -> String {
        (self.spec().session)()
    }

    pub fn command(
        self,
        session: &str,
        start: Start,
        guide: Option<&Path>,
        tuning: &Tuning,
    ) -> Command {
        (self.spec().command)(session, start, guide, tuning)
    }

    pub fn background_command(
        self,
        message: &str,
        guide: Option<&Path>,
        tuning: &Tuning,
    ) -> Command {
        (self.spec().background)(message, guide, tuning)
    }

    pub fn image(self, path: &Path) -> String {
        (self.spec().image)(path)
    }

    pub fn models(self, env: &HashMap<String, String>) -> Vec<Model> {
        match self {
            Agent::Claude => claude_models(env),
            Agent::Opencode => opencode_models(env),
        }
    }
}

fn texts(value: &serde_json::Value) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|text| text.as_str().map(str::to_string))
        .collect()
}

fn random() -> u64 {
    RandomState::new().build_hasher().finish()
}

fn image_path(path: &Path) -> String {
    path.display().to_string()
}

pub fn write_guide(guide: &Path) -> bool {
    let plugin = opencode_plugin(guide);
    fs::write(guide, GUIDE).is_ok()
        && fs::create_dir_all(&plugin).is_ok()
        && fs::write(plugin.join("tui.js"), OPENCODE_PLUGIN).is_ok()
}

pub fn found(path: &str) -> Vec<Agent> {
    let is_there =
        |agent: &Agent| std::env::split_paths(path).any(|dir| dir.join(agent.program()).is_file());
    Agent::ALL.into_iter().filter(is_there).collect()
}

pub fn search_path() -> String {
    let mut extra = vec![
        std::path::PathBuf::from("/opt/homebrew/bin"),
        std::path::PathBuf::from("/opt/homebrew/sbin"),
        std::path::PathBuf::from("/usr/local/bin"),
        std::path::PathBuf::from("/usr/local/sbin"),
    ];
    if let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) {
        extra.push(home.join(".local/bin"));
        extra.push(home.join(".cargo/bin"));
    }
    extend_path(&std::env::var("PATH").unwrap_or_default(), extra)
}

fn extend_path(current: &str, extra: impl IntoIterator<Item = std::path::PathBuf>) -> String {
    let mut dirs: Vec<std::path::PathBuf> = current
        .split(':')
        .filter(|dir| !dir.is_empty())
        .map(std::path::PathBuf::from)
        .collect();
    for dir in extra {
        if dir.is_dir() && !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    dirs.iter()
        .map(|dir| dir.to_string_lossy())
        .collect::<Vec<_>>()
        .join(":")
}

pub fn chosen(found: &[Agent], remembered: Option<Agent>) -> Option<Agent> {
    remembered
        .filter(|agent| found.contains(agent))
        .or_else(|| found.first().copied())
}

pub fn load_models(
    found: Vec<Agent>,
    env: HashMap<String, String>,
    ctx: egui::Context,
) -> std::sync::mpsc::Receiver<HashMap<Agent, Vec<Model>>> {
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let models = std::thread::scope(|scope| {
            let loading: Vec<_> = found
                .iter()
                .map(|agent| (*agent, scope.spawn(|| agent.models(&env))))
                .collect();
            loading
                .into_iter()
                .map(|(agent, models)| (agent, models.join().unwrap_or_default()))
                .collect()
        });
        if sender.send(models).is_ok() {
            ctx.request_repaint();
        }
    });
    receiver
}

#[cfg(test)]
pub mod tests;
