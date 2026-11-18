use super::choice::{choose_model, efforts, model_name};
use super::claude::{parse_claude_models, session_hook};
use super::opencode::{opencode_plugins, parse_opencode_models};
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
            r#"{"instructions":["/guide.md"],"model":"deepseek/deepseek-flash#max"}"#.to_string()
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
    let claude = Agent::Claude.background_command("write it", None, &tuning("haiku", Some("low")));
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
