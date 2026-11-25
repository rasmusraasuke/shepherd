use super::{AGENT_VARIABLE, CLAUDE, Command, GUIDE, Model, Start, Tuning, random, texts};
use std::collections::HashMap;
use std::io::Write as _;
use std::path::Path;

const CLAUDE_INITIALIZE: &str =
    r#"{"type":"control_request","request_id":"models","request":{"subtype":"initialize"}}"#;

pub(super) fn claude_models(env: &HashMap<String, String>) -> Vec<Model> {
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

pub(super) fn parse_claude_models(output: &str) -> Vec<Model> {
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

pub(super) fn claude_session() -> String {
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

pub(super) fn claude_command(
    session: &str,
    start: Start,
    guide: Option<&Path>,
    tuning: &Tuning,
) -> Command {
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

pub(super) fn session_hook() -> String {
    let command = format!("shepherd ctl agent-session \"${AGENT_VARIABLE}\" \"$(cat)\" >/dev/null");
    serde_json::json!({
        "hooks": {
            "SessionStart": [{"hooks": [{"type": "command", "command": command}]}]
        }
    })
    .to_string()
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

pub(super) fn claude_background(message: &str, guide: Option<&Path>, tuning: &Tuning) -> Command {
    let text = |text: &str| text.to_string();
    let mut args = vec![
        text("-p"),
        text("--permission-mode"),
        text("dontAsk"),
        text("--allowedTools"),
        text("Read"),
        text("Grep"),
        text("Glob"),
        text("Bash(git diff:*)"),
        text("Bash(git log:*)"),
        text("Bash(git show:*)"),
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
