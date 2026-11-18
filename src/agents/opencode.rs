use super::{Command, Model, OPENCODE, Start, Tuning, random};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

pub(super) const OPENCODE_PLUGIN: &str = include_str!("../../assets/opencode-plugin.js");

pub(super) fn opencode_models(env: &HashMap<String, String>) -> Vec<Model> {
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

pub(super) fn parse_opencode_models(output: &str) -> Option<Vec<Model>> {
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

pub(super) fn opencode_session() -> String {
    let (a, b) = (random(), random());
    format!("ses_{a:016x}{:010x}", b & 0xff_ffff_ffff)
}

pub(super) fn opencode_command(
    session: &str,
    start: Start,
    guide: Option<&Path>,
    tuning: &Tuning,
) -> Command {
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

pub(super) fn opencode_plugin(guide: &Path) -> PathBuf {
    guide.with_file_name("shepherd-opencode")
}

fn opencode_cli_config() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .unwrap_or_default()
        .join("opencode/cli.json")
}

pub(super) fn opencode_plugins(plugin: &Path, user: Option<&str>) -> String {
    let mut plugins = user
        .and_then(|user| serde_json::from_str::<serde_json::Value>(user).ok())
        .and_then(|user| user["plugins"].as_array().cloned())
        .unwrap_or_default();
    plugins.push(serde_json::json!(plugin));
    serde_json::json!({ "plugins": plugins }).to_string()
}

pub(super) fn opencode_background(message: &str, guide: Option<&Path>, tuning: &Tuning) -> Command {
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
