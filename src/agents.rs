use std::collections::HashMap;
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher as _, Hasher as _};
use std::path::Path;

use eframe::egui;
use serde::{Deserialize, Serialize};

pub const GUIDE: &str = "You run in the Shepherd editor, in which the user reviews the changes \
you make. `shepherd ctl help` lists what you can ask the editor: what the user has open and \
selected, their review comments and what they reviewed. It also shows them files and changes.";

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum Agent {
    Claude,
    Opencode,
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Model {
    pub name: String,
    pub value: String,
}

#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
pub struct Choice {
    #[serde(default)]
    pub agent: Option<Agent>,
    #[serde(default)]
    pub model: Option<String>,
}

impl Choice {
    pub fn agent(&self, found: &[Agent]) -> Option<Agent> {
        chosen(found, self.agent)
    }

    pub fn model(&self) -> Option<&str> {
        self.model.as_deref().filter(|model| !model.is_empty())
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
    command: fn(&str, Start, Option<&Path>, Option<&str>) -> Command,
    background: fn(&str, Option<&Path>, Option<&str>) -> Command,
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
        model: Option<&str>,
    ) -> Command {
        (self.spec().command)(session, start, guide, model)
    }

    pub fn background_command(
        self,
        message: &str,
        guide: Option<&Path>,
        model: Option<&str>,
    ) -> Command {
        (self.spec().background)(message, guide, model)
    }

    pub fn image(self, path: &Path) -> String {
        (self.spec().image)(path)
    }

    pub fn models(self, env: &HashMap<String, String>) -> Vec<Model> {
        match self {
            Agent::Claude => [("Opus", "opus"), ("Sonnet", "sonnet"), ("Haiku", "haiku")]
                .into_iter()
                .map(|(name, value)| Model {
                    name: name.to_string(),
                    value: value.to_string(),
                })
                .collect(),
            Agent::Opencode => std::process::Command::new(self.program())
                .arg("models")
                .envs(env)
                .output()
                .ok()
                .map(|output| {
                    String::from_utf8_lossy(&output.stdout)
                        .lines()
                        .map(str::trim)
                        .filter(|line| !line.is_empty())
                        .map(|line| Model {
                            name: line.to_string(),
                            value: line.to_string(),
                        })
                        .collect()
                })
                .unwrap_or_default(),
        }
    }
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

fn claude_command(
    session: &str,
    start: Start,
    guide: Option<&Path>,
    model: Option<&str>,
) -> Command {
    let text = |text: &str| text.to_string();
    let session_flag = match start {
        Start::New(_) => "--session-id",
        Start::Continue => "--resume",
    };
    let mut args = vec![text(session_flag), text(session)];
    if let Some(model) = model {
        args.extend([text("--model"), text(model)]);
    }
    if guide.is_some() {
        args.extend([text("--append-system-prompt"), text(GUIDE)]);
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

fn opencode_command(
    session: &str,
    start: Start,
    guide: Option<&Path>,
    model: Option<&str>,
) -> Command {
    let text = |text: &str| text.to_string();
    let mut args = vec![text("--session"), text(session)];
    if let Start::New(Some(message)) = start {
        args.extend([text("--prompt"), message]);
    }
    let env = opencode_env(guide, model);
    Command {
        program: OPENCODE.program,
        args,
        env,
    }
}

fn claude_background(message: &str, guide: Option<&Path>, model: Option<&str>) -> Command {
    let text = |text: &str| text.to_string();
    let mut args = vec![
        text("-p"),
        text("--permission-mode"),
        text("dontAsk"),
        text("--allowedTools"),
        text("Bash(git diff:*)"),
        text("Bash(shepherd ctl:*)"),
    ];
    if let Some(model) = model {
        args.extend([text("--model"), text(model)]);
    }
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

fn opencode_background(message: &str, guide: Option<&Path>, model: Option<&str>) -> Command {
    let text = |text: &str| text.to_string();
    let mut args = vec![text("run"), text("--auto")];
    if let Some(model) = model {
        args.extend([text("--model"), text(model)]);
    }
    args.push(text(message));
    Command {
        program: OPENCODE.program,
        args,
        env: opencode_env(guide, model),
    }
}

fn opencode_env(guide: Option<&Path>, model: Option<&str>) -> Vec<(String, String)> {
    let mut config = serde_json::Map::new();
    if let Some(guide) = guide {
        config.insert("instructions".to_string(), serde_json::json!([guide]));
    }
    if let Some(model) = model {
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

pub fn models_for(found: &[Agent], env: &HashMap<String, String>) -> HashMap<Agent, Vec<Model>> {
    found
        .iter()
        .map(|agent| (*agent, agent.models(env)))
        .collect()
}

pub fn choice_ui(
    ui: &mut egui::Ui,
    found: &[Agent],
    models: &HashMap<Agent, Vec<Model>>,
    choice: &mut Choice,
) {
    let agent = choice.agent(found);
    let name = agent.map_or("No agent found", Agent::name);
    ui.menu_button(name, |ui| {
        for found_agent in found {
            if ui
                .selectable_label(agent == Some(*found_agent), found_agent.name())
                .clicked()
            {
                if choice.agent != Some(*found_agent) {
                    choice.model = None;
                }
                choice.agent = Some(*found_agent);
                ui.close();
            }
        }
    });
    let Some(agent) = agent else {
        return;
    };
    let selected = choice.model().unwrap_or("Default model").to_string();
    ui.menu_button(selected, |ui| {
        if ui
            .selectable_label(choice.model().is_none(), "Default")
            .clicked()
        {
            choice.model = None;
            ui.close();
        }
        if let Some(options) = models.get(&agent) {
            egui::ScrollArea::vertical()
                .max_height(300.0)
                .show(ui, |ui| {
                    for model in options {
                        let chosen = choice.model() == Some(model.value.as_str());
                        if ui.selectable_label(chosen, &model.name).clicked() {
                            choice.model = Some(model.value.clone());
                            ui.close();
                        }
                    }
                });
        }
    })
    .response
    .on_hover_text("Model for new agent sessions");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    fn args(agent: Agent, start: Start, guide: Option<&Path>) -> Vec<String> {
        agent.command("s1", start, guide, None).args
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
            ["--session-id", "s1", "--append-system-prompt", GUIDE]
        );
        assert_eq!(
            args(Agent::Claude, Start::Continue, None),
            ["--resume", "s1"]
        );
    }

    #[test]
    fn starts_opencode_with_a_session_a_first_message_and_the_guide() {
        let message = Start::New(Some("fix it".to_string()));
        let command = Agent::Opencode.command("s1", message, Some(Path::new("/guide.md")), None);
        assert_eq!(command.program, "opencode");
        assert_eq!(command.args, ["--session", "s1", "--prompt", "fix it"]);
        assert_eq!(
            command.env,
            [(
                "OPENCODE_CONFIG_CONTENT".to_string(),
                r#"{"instructions":["/guide.md"]}"#.to_string()
            )]
        );
        assert_eq!(
            args(Agent::Opencode, Start::Continue, None),
            ["--session", "s1"]
        );
    }

    #[test]
    fn starts_with_the_chosen_model() {
        assert_eq!(
            Agent::Claude
                .command("s1", Start::New(None), None, Some("haiku"))
                .args,
            ["--session-id", "s1", "--model", "haiku"]
        );
        let command = Agent::Opencode.command(
            "s1",
            Start::New(None),
            Some(Path::new("/guide.md")),
            Some("deepseek/deepseek-flash"),
        );
        assert_eq!(
            command.env,
            [(
                "OPENCODE_CONFIG_CONTENT".to_string(),
                r#"{"instructions":["/guide.md"],"model":"deepseek/deepseek-flash"}"#.to_string()
            )]
        );
    }

    #[test]
    fn starts_a_background_agent_without_a_terminal() {
        let claude = Agent::Claude.background_command("write it", None, Some("haiku"));
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
                "--",
                "write it"
            ]
        );
        let opencode = Agent::Opencode.background_command(
            "write it",
            Some(Path::new("/guide.md")),
            Some("deepseek/deepseek-flash"),
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

    #[test]
    fn knows_the_models_to_choose_from() {
        let claude: Vec<String> = Agent::Claude
            .models(&HashMap::new())
            .into_iter()
            .map(|model| model.value)
            .collect();
        assert_eq!(claude, ["opus", "sonnet", "haiku"]);
        let choice = Choice {
            agent: Some(Agent::Claude),
            model: Some("haiku".to_string()),
        };
        assert_eq!(choice.agent(&[Agent::Claude]), Some(Agent::Claude));
        assert_eq!(choice.model(), Some("haiku"));
        assert_eq!(Choice::default().model(), None);
    }

    #[test]
    fn gives_every_agent_its_own_name_and_program() {
        for (index, agent) in Agent::ALL.iter().enumerate() {
            assert_eq!(
                agent.command("s1", Start::Continue, None, None).program,
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
