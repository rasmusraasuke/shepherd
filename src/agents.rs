use std::collections::HashMap;
use std::collections::hash_map::RandomState;
use std::fs;
use std::hash::{BuildHasher as _, Hasher as _};
use std::io::Write as _;
use std::path::{Path, PathBuf};

use eframe::egui::{self, containers::menu::MenuButton};
use serde::{Deserialize, Serialize};

use crate::icons::{self, Icon};
use crate::theme;

pub const GUIDE: &str = "You run in the Shepherd editor, in which the user reviews the changes \
you make. `shepherd ctl help` lists what you can ask the editor: what the user has open and \
selected, their review comments and what they reviewed. It also shows them files and changes.";

pub const AGENT_VARIABLE: &str = "SHEPHERD_AGENT";

const DEFAULT_MODEL: &str = "default";

const CLAUDE_INITIALIZE: &str =
    r#"{"type":"control_request","request_id":"models","request":{"subtype":"initialize"}}"#;

const OPENCODE_PLUGIN: &str = include_str!("../assets/opencode-plugin.js");

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

fn claude_models(env: &HashMap<String, String>) -> Vec<Model> {
    let child = std::process::Command::new(CLAUDE.program)
        .args([
            "-p",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--verbose",
            "--no-session-persistence",
        ])
        .envs(env)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn();
    let Ok(mut child) = child else {
        return Vec::new();
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = writeln!(stdin, "{CLAUDE_INITIALIZE}");
    }
    child
        .wait_with_output()
        .map(|output| parse_claude_models(&String::from_utf8_lossy(&output.stdout)))
        .unwrap_or_default()
}

fn parse_claude_models(output: &str) -> Vec<Model> {
    let models = output
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .find(|message| message["type"] == "control_response")
        .and_then(|message| {
            message["response"]["response"]["models"]
                .as_array()
                .cloned()
        })
        .unwrap_or_default();
    models
        .iter()
        .filter_map(|model| {
            Some(Model {
                name: model["displayName"].as_str()?.to_string(),
                value: model["value"].as_str()?.to_string(),
                efforts: texts(&model["supportedEffortLevels"]),
            })
        })
        .collect()
}

fn opencode_models(env: &HashMap<String, String>) -> Vec<Model> {
    for attempt in 0..3 {
        if attempt > 0 {
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        let output = std::process::Command::new(OPENCODE.program)
            .args(["api", "GET", "/api/model"])
            .envs(env)
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output();
        if let Some(models) = output
            .ok()
            .and_then(|output| parse_opencode_models(&String::from_utf8_lossy(&output.stdout)))
        {
            return models;
        }
    }
    Vec::new()
}

fn parse_opencode_models(output: &str) -> Option<Vec<Model>> {
    let response = serde_json::from_str::<serde_json::Value>(output).ok()?;
    let mut models: Vec<Model> = response["data"]
        .as_array()?
        .iter()
        .filter(|model| model["enabled"] != false)
        .filter_map(|model| {
            let provider = model["providerID"].as_str()?;
            let name = model["name"].as_str()?;
            Some(Model {
                name: format!("{name} ({provider})"),
                value: format!("{provider}/{}", model["id"].as_str()?),
                efforts: model["variants"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|variant| variant["id"].as_str().map(str::to_string))
                    .collect(),
            })
        })
        .collect();
    models.sort_by(|a, b| a.value.cmp(&b.value));
    Some(models)
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

fn claude_session() -> String {
    let (a, b) = (random(), random());
    format!(
        "{:08x}-{:04x}-4{:03x}-{:04x}-{:012x}",
        a >> 32,
        a >> 16 & 0xffff,
        a & 0xfff,
        0x8000 | b >> 48 & 0x3fff,
        b & 0xffff_ffff_ffff
    )
}

fn opencode_session() -> String {
    let (a, b) = (random(), random());
    format!("ses_{a:016x}{:010x}", b & 0xff_ffff_ffff)
}

fn claude_command(session: &str, start: Start, guide: Option<&Path>, tuning: &Tuning) -> Command {
    let text = |text: &str| text.to_string();
    let session_flag = match start {
        Start::New(_) => "--session-id",
        Start::Continue => "--resume",
    };
    let mut args = vec![text(session_flag), text(session)];
    args.extend(claude_tuning(tuning));
    if guide.is_some() {
        args.extend([
            text("--append-system-prompt"),
            text(GUIDE),
            text("--settings"),
            session_hook(),
        ]);
    }
    if let Start::New(Some(message)) = start {
        args.extend([text("--"), message]);
    }
    Command {
        program: CLAUDE.program,
        args,
        env: Vec::new(),
    }
}

fn session_hook() -> String {
    let command = format!("shepherd ctl agent-session \"${AGENT_VARIABLE}\" \"$(cat)\" >/dev/null");
    serde_json::json!({
        "hooks": {
            "SessionStart": [{"hooks": [{"type": "command", "command": command}]}]
        }
    })
    .to_string()
}

fn opencode_command(session: &str, start: Start, guide: Option<&Path>, tuning: &Tuning) -> Command {
    let text = |text: &str| text.to_string();
    let mut args = vec![text("--session"), text(session)];
    if let Start::New(Some(message)) = start {
        args.extend([text("--prompt"), message]);
    }
    let mut env = opencode_env(guide, tuning);
    if let Some(guide) = guide {
        let user = fs::read_to_string(opencode_cli_config()).ok();
        env.push((
            "OPENCODE_CLI_CONFIG_CONTENT".to_string(),
            opencode_plugins(&opencode_plugin(guide), user.as_deref()),
        ));
    }
    Command {
        program: OPENCODE.program,
        args,
        env,
    }
}

pub fn write_guide(guide: &Path) -> bool {
    let plugin = opencode_plugin(guide);
    fs::write(guide, GUIDE).is_ok()
        && fs::create_dir_all(&plugin).is_ok()
        && fs::write(plugin.join("tui.js"), OPENCODE_PLUGIN).is_ok()
}

fn opencode_plugin(guide: &Path) -> PathBuf {
    guide.with_file_name("shepherd-opencode")
}

fn opencode_cli_config() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .unwrap_or_default()
        .join("opencode/cli.json")
}

fn opencode_plugins(plugin: &Path, user: Option<&str>) -> String {
    let mut plugins = user
        .and_then(|user| serde_json::from_str::<serde_json::Value>(user).ok())
        .and_then(|user| user["plugins"].as_array().cloned())
        .unwrap_or_default();
    plugins.push(serde_json::json!(plugin));
    serde_json::json!({ "plugins": plugins }).to_string()
}

fn claude_tuning(tuning: &Tuning) -> Vec<String> {
    let mut args = Vec::new();
    if let Some(model) = tuning.model() {
        args.extend(["--model".to_string(), model.to_string()]);
    }
    if let Some(effort) = tuning.effort() {
        args.extend(["--effort".to_string(), effort.to_string()]);
    }
    args
}

fn claude_background(message: &str, guide: Option<&Path>, tuning: &Tuning) -> Command {
    let text = |text: &str| text.to_string();
    let mut args = vec![
        text("-p"),
        text("--permission-mode"),
        text("dontAsk"),
        text("--allowedTools"),
        text("Bash(git diff:*)"),
        text("Bash(shepherd ctl:*)"),
    ];
    args.extend(claude_tuning(tuning));
    if guide.is_some() {
        args.extend([text("--append-system-prompt"), text(GUIDE)]);
    }
    args.extend([text("--"), text(message)]);
    Command {
        program: CLAUDE.program,
        args,
        env: Vec::new(),
    }
}

fn opencode_background(message: &str, guide: Option<&Path>, tuning: &Tuning) -> Command {
    let text = |text: &str| text.to_string();
    let mut args = vec![text("run"), text("--auto")];
    if let Some(model) = tuning.opencode_model() {
        args.extend([text("--model"), model]);
    }
    args.push(text(message));
    Command {
        program: OPENCODE.program,
        args,
        env: opencode_env(guide, tuning),
    }
}

fn opencode_env(guide: Option<&Path>, tuning: &Tuning) -> Vec<(String, String)> {
    let mut config = serde_json::Map::new();
    if let Some(guide) = guide {
        config.insert("instructions".to_string(), serde_json::json!([guide]));
    }
    if let Some(model) = tuning.opencode_model() {
        config.insert("model".to_string(), serde_json::json!(model));
    }
    if config.is_empty() {
        Vec::new()
    } else {
        vec![(
            "OPENCODE_CONFIG_CONTENT".to_string(),
            serde_json::Value::Object(config).to_string(),
        )]
    }
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

fn efforts<'a>(models: Option<&'a Vec<Model>>, tuning: &Tuning) -> &'a [String] {
    let value = tuning.model().unwrap_or(DEFAULT_MODEL);
    models
        .into_iter()
        .flatten()
        .find(|model| model.value == value)
        .map_or(&[], |model| &model.efforts)
}

fn model_name(models: Option<&Vec<Model>>, tuning: &Tuning) -> Option<String> {
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

fn choose_model(models: Option<&Vec<Model>>, tuning: &mut Tuning, model: Option<String>) {
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

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::testing::TempDir;

    fn args(agent: Agent, start: Start, guide: Option<&Path>) -> Vec<String> {
        agent.command("s1", start, guide, &Tuning::default()).args
    }

    #[test]
    fn finds_the_agents_on_the_path() {
        let (with, without) = (TempDir::new("agents-with"), TempDir::new("agents-without"));
        with.write("opencode", "");
        let path = std::env::join_paths([without.path(), with.path()]).unwrap();
        assert_eq!(found(path.to_str().unwrap()), [Agent::Opencode]);
        assert!(found("").is_empty());
    }

    #[test]
    fn adds_the_places_agents_live_when_started_from_finder() {
        let (dir, missing) = (TempDir::new("agents-extra"), TempDir::new("agents-missing"));
        let gone = missing.path().join("gone");
        let path = extend_path(
            "/usr/bin:/bin",
            [dir.path().to_path_buf(), gone, dir.path().to_path_buf()],
        );
        let pieces: Vec<&str> = path.split(':').collect();
        assert_eq!(pieces, ["/usr/bin", "/bin", dir.path().to_str().unwrap()]);
    }

    #[test]
    fn uses_the_remembered_agent_while_it_is_there() {
        let both = [Agent::Claude, Agent::Opencode];
        assert_eq!(chosen(&both, None), Some(Agent::Claude));
        assert_eq!(chosen(&both, Some(Agent::Opencode)), Some(Agent::Opencode));
        assert_eq!(
            chosen(&[Agent::Claude], Some(Agent::Opencode)),
            Some(Agent::Claude)
        );
        assert_eq!(chosen(&[], Some(Agent::Claude)), None);
    }

    #[test]
    fn starts_claude_with_a_session_a_first_message_and_the_guide() {
        let message = || Start::New(Some("fix it".to_string()));
        assert_eq!(
            args(Agent::Claude, message(), None),
            ["--session-id", "s1", "--", "fix it"]
        );
        assert_eq!(
            args(
                Agent::Claude,
                Start::New(None),
                Some(Path::new("/guide.md"))
            ),
            [
                "--session-id",
                "s1",
                "--append-system-prompt",
                GUIDE,
                "--settings",
                &session_hook()
            ]
        );
        assert_eq!(
            args(Agent::Claude, Start::Continue, None),
            ["--resume", "s1"]
        );
    }

    #[test]
    fn starts_opencode_with_a_session_a_first_message_and_the_guide() {
        let message = Start::New(Some("fix it".to_string()));
        let command = Agent::Opencode.command(
            "s1",
            message,
            Some(Path::new("/guide.md")),
            &Tuning::default(),
        );
        assert_eq!(command.program, "opencode");
        assert_eq!(command.args, ["--session", "s1", "--prompt", "fix it"]);
        assert_eq!(
            command.env[0],
            (
                "OPENCODE_CONFIG_CONTENT".to_string(),
                r#"{"instructions":["/guide.md"]}"#.to_string()
            )
        );
        assert_eq!(command.env[1].0, "OPENCODE_CLI_CONFIG_CONTENT");
        assert_eq!(
            args(Agent::Opencode, Start::Continue, None),
            ["--session", "s1"]
        );
    }

    fn tuning(model: &str, effort: Option<&str>) -> Tuning {
        Tuning {
            model: Some(model.to_string()),
            effort: effort.map(str::to_string),
        }
    }

    #[test]
    fn starts_with_the_chosen_model_and_effort() {
        assert_eq!(
            Agent::Claude
                .command("s1", Start::New(None), None, &tuning("haiku", None))
                .args,
            ["--session-id", "s1", "--model", "haiku"]
        );
        let effort = Tuning {
            model: None,
            effort: Some("high".to_string()),
        };
        assert_eq!(
            Agent::Claude
                .command("s1", Start::New(None), None, &effort)
                .args,
            ["--session-id", "s1", "--effort", "high"]
        );
        let command = Agent::Opencode.command(
            "s1",
            Start::New(None),
            Some(Path::new("/guide.md")),
            &tuning("deepseek/deepseek-flash", Some("max")),
        );
        assert_eq!(
            command.env[0],
            (
                "OPENCODE_CONFIG_CONTENT".to_string(),
                r#"{"instructions":["/guide.md"],"model":"deepseek/deepseek-flash#max"}"#
                    .to_string()
            )
        );
        assert!(
            Agent::Opencode
                .command("s1", Start::New(None), None, &effort)
                .env
                .is_empty()
        );
    }

    #[test]
    fn adds_the_session_plugin_to_the_opencode_plugins_of_the_user() {
        let plugin = Path::new("/repo/.git/shepherd-opencode");
        assert_eq!(
            opencode_plugins(plugin, None),
            r#"{"plugins":["/repo/.git/shepherd-opencode"]}"#
        );
        assert_eq!(
            opencode_plugins(plugin, Some(r#"{"theme":{},"plugins":["mine"]}"#)),
            r#"{"plugins":["mine","/repo/.git/shepherd-opencode"]}"#
        );
        assert_eq!(
            opencode_plugins(plugin, Some("not json")),
            r#"{"plugins":["/repo/.git/shepherd-opencode"]}"#
        );
    }

    #[test]
    fn writes_the_guide_and_the_opencode_plugin_beside_it() {
        let dir = TempDir::new("guide");
        let guide = dir.path().join("guide.md");
        assert!(write_guide(&guide));
        assert_eq!(fs::read_to_string(&guide).unwrap(), GUIDE);
        assert_eq!(
            fs::read_to_string(opencode_plugin(&guide).join("tui.js")).unwrap(),
            OPENCODE_PLUGIN
        );
    }

    #[test]
    fn starts_a_background_agent_without_a_terminal() {
        let claude =
            Agent::Claude.background_command("write it", None, &tuning("haiku", Some("low")));
        assert_eq!(
            claude.args,
            [
                "-p",
                "--permission-mode",
                "dontAsk",
                "--allowedTools",
                "Bash(git diff:*)",
                "Bash(shepherd ctl:*)",
                "--model",
                "haiku",
                "--effort",
                "low",
                "--",
                "write it"
            ]
        );
        let opencode = Agent::Opencode.background_command(
            "write it",
            Some(Path::new("/guide.md")),
            &tuning("deepseek/deepseek-flash", None),
        );
        assert_eq!(
            opencode.args,
            [
                "run",
                "--auto",
                "--model",
                "deepseek/deepseek-flash",
                "write it"
            ]
        );
        assert_eq!(
            opencode.env,
            [(
                "OPENCODE_CONFIG_CONTENT".to_string(),
                r#"{"instructions":["/guide.md"],"model":"deepseek/deepseek-flash"}"#.to_string()
            )]
        );
    }

    pub fn claude_sample() -> Vec<Model> {
        parse_claude_models(CLAUDE_MODELS)
    }

    const CLAUDE_MODELS: &str = r#"{"type":"system","subtype":"init"}
{"type":"control_response","response":{"subtype":"success","request_id":"models","response":{"models":[{"value":"default","displayName":"Default (recommended)","supportedEffortLevels":["low","high"]},{"value":"opus","displayName":"Opus 5.5","supportedEffortLevels":["low","medium","high","xhigh","max"]},{"value":"haiku","displayName":"Haiku","supportedEffortLevels":["low","high"]},{"value":"claude-haiku-4-5-20251001","displayName":"Haiku 4.5"}]}}}"#;

    const OPENCODE_MODELS: &str = r#"{"location":{"directory":"/repo"},"data":[{"id":"mistral-small-latest","providerID":"mistral","name":"Mistral Small","enabled":true,"variants":[{"id":"none","settings":{}},{"id":"high","settings":{}}]},{"id":"deepseek-flash","providerID":"deepseek","name":"DeepSeek Flash","enabled":true,"variants":[]},{"id":"gone","providerID":"deepseek","name":"Gone","enabled":false,"variants":[]}]}"#;

    #[test]
    fn reads_the_models_claude_offers_with_their_efforts() {
        let models = claude_sample();
        let values: Vec<&str> = models.iter().map(|model| model.value.as_str()).collect();
        assert_eq!(
            values,
            ["default", "opus", "haiku", "claude-haiku-4-5-20251001"]
        );
        assert_eq!(models[1].name, "Opus 5.5");
        assert_eq!(models[1].efforts, ["low", "medium", "high", "xhigh", "max"]);
        assert!(models[3].efforts.is_empty());
        assert!(parse_claude_models("not json\n").is_empty());
    }

    #[test]
    fn reads_the_enabled_models_opencode_offers_with_their_variants() {
        let models = parse_opencode_models(OPENCODE_MODELS).unwrap();
        assert_eq!(
            models,
            [
                Model {
                    name: "DeepSeek Flash (deepseek)".to_string(),
                    value: "deepseek/deepseek-flash".to_string(),
                    efforts: Vec::new(),
                },
                Model {
                    name: "Mistral Small (mistral)".to_string(),
                    value: "mistral/mistral-small-latest".to_string(),
                    efforts: vec!["none".to_string(), "high".to_string()],
                },
            ]
        );
        assert_eq!(parse_opencode_models("ServiceUnavailableError"), None);
    }

    #[test]
    fn offers_the_efforts_of_the_chosen_model() {
        let models = claude_sample();
        let mut tuning = tuning("opus", Some("xhigh"));
        assert_eq!(efforts(Some(&models), &tuning).len(), 5);
        assert_eq!(
            model_name(Some(&models), &tuning).as_deref(),
            Some("Opus 5.5")
        );
        choose_model(Some(&models), &mut tuning, Some("haiku".to_string()));
        assert_eq!(tuning.effort(), None);
        tuning.effort = Some("high".to_string());
        choose_model(Some(&models), &mut tuning, None);
        assert_eq!(tuning.effort(), Some("high"));
        assert_eq!(efforts(Some(&models), &tuning), ["low", "high"]);
        choose_model(
            Some(&models),
            &mut tuning,
            Some("claude-haiku-4-5-20251001".to_string()),
        );
        assert_eq!(tuning.effort(), None);
        assert!(efforts(Some(&models), &tuning).is_empty());
        assert!(efforts(None, &Tuning::default()).is_empty());
    }

    #[test]
    fn remembers_the_model_and_effort_of_every_harness() {
        let mut choice = Choice {
            agent: Some(Agent::Opencode),
            tunings: HashMap::from([(
                Agent::Opencode,
                tuning("deepseek/deepseek-flash", Some("max")),
            )]),
        };
        choice.agent = Some(Agent::Claude);
        choice
            .tunings
            .insert(Agent::Claude, tuning("haiku", Some("low")));
        choice.agent = Some(Agent::Opencode);
        assert_eq!(
            choice.tuning(Agent::Opencode),
            tuning("deepseek/deepseek-flash", Some("max"))
        );
        assert_eq!(choice.tuning(Agent::Claude), tuning("haiku", Some("low")));

        let json = serde_json::to_string(&choice).unwrap();
        assert_eq!(serde_json::from_str::<Choice>(&json).unwrap(), choice);
        let older: Choice = serde_json::from_str(r#"{"agent":"Claude","model":"haiku"}"#).unwrap();
        assert_eq!(older.agent, Some(Agent::Claude));
        assert_eq!(older.tuning(Agent::Claude), Tuning::default());
    }

    #[test]
    fn gives_every_agent_its_own_name_and_program() {
        for (index, agent) in Agent::ALL.iter().enumerate() {
            assert_eq!(
                agent
                    .command("s1", Start::Continue, None, &Tuning::default())
                    .program,
                agent.program()
            );
            let before = &Agent::ALL[..index];
            assert!(before.iter().all(|other| other.name() != agent.name()));
            assert!(
                before
                    .iter()
                    .all(|other| other.program() != agent.program())
            );
        }
    }

    #[test]
    fn hands_every_agent_its_pictures_the_way_it_takes_them() {
        for agent in Agent::ALL {
            assert_eq!(
                agent.image(Path::new("/tmp/shepherd-images/image-1-0.png")),
                "/tmp/shepherd-images/image-1-0.png"
            );
        }
    }

    #[test]
    fn names_sessions_the_way_each_agent_wants_them() {
        let claude = Agent::Claude.new_session();
        let lengths: Vec<usize> = claude.split('-').map(str::len).collect();
        assert_eq!(lengths, [8, 4, 4, 4, 12]);
        assert_ne!(claude, Agent::Claude.new_session());
        let opencode = Agent::Opencode.new_session();
        assert!(opencode.starts_with("ses_") && opencode.len() == 30);
    }
}
