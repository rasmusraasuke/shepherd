use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher as _, Hasher as _};
use std::path::Path;

use serde::{Deserialize, Serialize};

pub const GUIDE: &str = "You run in a tab of the Shepherd editor, in which the user reviews the \
changes you make. `shepherd ctl help` lists what you can ask the editor: what the user has open \
and selected, their review comments and what they reviewed. It also shows them files and changes.";

#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub enum Agent {
    Claude,
    Opencode,
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
    command: fn(&str, Start, Option<&Path>) -> Command,
}

static CLAUDE: Spec = Spec {
    name: "Claude Code",
    program: "claude",
    session: claude_session,
    command: claude_command,
};

static OPENCODE: Spec = Spec {
    name: "Opencode",
    program: "opencode",
    session: opencode_session,
    command: opencode_command,
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

    pub fn command(self, session: &str, start: Start, guide: Option<&Path>) -> Command {
        (self.spec().command)(session, start, guide)
    }
}

fn random() -> u64 {
    RandomState::new().build_hasher().finish()
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

fn claude_command(session: &str, start: Start, guide: Option<&Path>) -> Command {
    let text = |text: &str| text.to_string();
    let session_flag = match start {
        Start::New(_) => "--session-id",
        Start::Continue => "--resume",
    };
    let mut args = vec![text(session_flag), text(session)];
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

fn opencode_command(session: &str, start: Start, guide: Option<&Path>) -> Command {
    let text = |text: &str| text.to_string();
    let mut args = vec![text("--session"), text(session)];
    if let Start::New(Some(message)) = start {
        args.extend([text("--prompt"), message]);
    }
    let mut env = Vec::new();
    if let Some(guide) = guide {
        let config = serde_json::json!({ "instructions": [guide] });
        env.push((text("OPENCODE_CONFIG_CONTENT"), config.to_string()));
    }
    Command {
        program: OPENCODE.program,
        args,
        env,
    }
}

pub fn found(path: &str) -> Vec<Agent> {
    let is_there =
        |agent: &Agent| std::env::split_paths(path).any(|dir| dir.join(agent.program()).is_file());
    Agent::ALL.into_iter().filter(is_there).collect()
}

pub fn chosen(found: &[Agent], remembered: Option<Agent>) -> Option<Agent> {
    remembered
        .filter(|agent| found.contains(agent))
        .or_else(|| found.first().copied())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    fn args(agent: Agent, start: Start, guide: Option<&Path>) -> Vec<String> {
        agent.command("s1", start, guide).args
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
        let command = Agent::Opencode.command("s1", message, Some(Path::new("/guide.md")));
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
    fn gives_every_agent_its_own_name_and_program() {
        for (index, agent) in Agent::ALL.iter().enumerate() {
            assert_eq!(
                agent.command("s1", Start::Continue, None).program,
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
    fn names_sessions_the_way_each_agent_wants_them() {
        let claude = Agent::Claude.new_session();
        let lengths: Vec<usize> = claude.split('-').map(str::len).collect();
        assert_eq!(lengths, [8, 4, 4, 4, 12]);
        assert_ne!(claude, Agent::Claude.new_session());
        let opencode = Agent::Opencode.new_session();
        assert!(opencode.starts_with("ses_") && opencode.len() == 30);
    }
}
