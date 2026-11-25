pub mod install;
#[cfg(test)]
mod install_tests;
mod server;
#[cfg(test)]
pub mod tests;
mod transport;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, TryRecvError};

use eframe::egui;
use serde_json::{Value, json};

use crate::editor::buffer::{Buffer, Pos};
use install::{Installer, Method, Progress};
pub use server::{Encoding, Message, Server};

pub struct Language {
    pub name: &'static str,
    files: &'static [(&'static str, &'static str)],
    program: &'static str,
    args: &'static [&'static str],
    check: &'static [&'static str],
    method: Method,
}

pub const LANGUAGES: &[Language] = &[
    Language {
        name: "Kotlin",
        files: &[("kt", "kotlin"), ("kts", "kotlin")],
        program: "kotlin-lsp",
        args: &["--stdio"],
        check: &[],
        method: Method::JetBrains,
    },
    Language {
        name: "Java",
        files: &[("java", "java")],
        program: "jdtls",
        args: &[],
        check: &[],
        method: Method::Manual,
    },
    Language {
        name: "Rust",
        files: &[("rs", "rust")],
        program: "rust-analyzer",
        args: &[],
        check: &["--version"],
        method: Method::Release {
            repo: "rust-lang/rust-analyzer",
            asset: "rust-analyzer",
        },
    },
    Language {
        name: "TypeScript",
        files: &[
            ("ts", "typescript"),
            ("tsx", "typescriptreact"),
            ("js", "javascript"),
            ("jsx", "javascriptreact"),
            ("mjs", "javascript"),
            ("cjs", "javascript"),
        ],
        program: "typescript-language-server",
        args: &["--stdio"],
        check: &["--version"],
        method: Method::Npm {
            packages: &["typescript-language-server", "typescript@6"],
            entry: "node_modules/typescript-language-server/lib/cli.mjs",
        },
    },
    Language {
        name: "Svelte",
        files: &[("svelte", "svelte")],
        program: "svelteserver",
        args: &["--stdio"],
        check: &[],
        method: Method::Npm {
            packages: &["svelte-language-server", "typescript@6"],
            entry: "node_modules/svelte-language-server/bin/server.js",
        },
    },
    Language {
        name: "Python",
        files: &[("py", "python")],
        program: "pyright-langserver",
        args: &["--stdio"],
        check: &[],
        method: Method::Npm {
            packages: &["pyright"],
            entry: "node_modules/pyright/langserver.index.js",
        },
    },
    Language {
        name: "Go",
        files: &[("go", "go")],
        program: "gopls",
        args: &[],
        check: &["version"],
        method: Method::Go {
            module: "golang.org/x/tools/gopls",
        },
    },
];

pub fn language_of(path: &str) -> Option<(&'static Language, &'static str)> {
    let extension = Path::new(path).extension()?.to_str()?;
    LANGUAGES.iter().find_map(|language| {
        language
            .files
            .iter()
            .find(|(held, _)| *held == extension)
            .map(|(_, id)| (language, *id))
    })
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Severity {
    Error,
    Warning,
    Information,
}

impl Severity {
    fn name(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Information => "info",
        }
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct Diagnostic {
    pub start: Pos,
    pub end: Pos,
    pub severity: Severity,
    pub message: String,
}

impl Diagnostic {
    pub fn covers(&self, pos: Pos) -> bool {
        if self.start == self.end {
            return pos.line == self.start.line && pos.col.abs_diff(self.start.col) <= 1;
        }
        self.start <= pos && pos < self.end
    }
}

#[derive(Debug, PartialEq)]
pub enum Event {
    Hover {
        path: String,
        pos: Pos,
        text: String,
    },
    Definition {
        path: String,
        start: Pos,
        end: Pos,
    },
}

enum Request {
    Hover { path: String, pos: Pos },
    Definition,
    Content { uri: String, range: Value },
}

struct Running {
    server: Server,
    pending: HashMap<u64, Request>,
}

enum Slot {
    Finding {
        found: Receiver<Result<Vec<String>, String>>,
        progress: Progress,
    },
    Running(Running),
    Failed(String),
}

impl Slot {
    fn running(&mut self) -> Option<&mut Running> {
        match self {
            Slot::Running(running) if !running.server.stopped => Some(running),
            _ => None,
        }
    }

    fn is_running(&self) -> bool {
        matches!(self, Slot::Running(running) if !running.server.stopped)
    }
}

struct Document {
    uri: String,
    language: &'static str,
    version: i64,
    revision: u64,
    lines: Vec<String>,
}

pub struct Lsp {
    root: PathBuf,
    canonical_root: PathBuf,
    sources: PathBuf,
    start: Option<(egui::Context, Installer)>,
    servers: HashMap<&'static str, Slot>,
    documents: HashMap<String, Document>,
    diagnostics: HashMap<String, Arc<Vec<Diagnostic>>>,
}

impl Lsp {
    pub fn new(root: PathBuf) -> Self {
        Self {
            canonical_root: root.canonicalize().unwrap_or_else(|_| root.clone()),
            root,
            sources: crate::dirs::cache()
                .unwrap_or_else(std::env::temp_dir)
                .join("sources"),
            start: None,
            servers: HashMap::new(),
            documents: HashMap::new(),
            diagnostics: HashMap::new(),
        }
    }

    pub fn enable(&mut self, ctx: egui::Context, installer: Installer) {
        self.start = Some((ctx, installer));
    }

    #[cfg(test)]
    pub fn connect(&mut self, language: &'static str, server: Server) {
        self.start.get_or_insert_with(|| {
            (
                egui::Context::default(),
                Installer::new(PathBuf::new(), String::new(), false),
            )
        });
        self.servers.insert(
            language,
            Slot::Running(Running {
                server,
                pending: HashMap::new(),
            }),
        );
    }

    fn running(&mut self, path: &str) -> Option<(&mut Running, &Document)> {
        let document = self.documents.get(path)?;
        let running = self.servers.get_mut(document.language)?.running()?;
        Some((running, document))
    }

    pub fn has_server(&self, path: &str) -> bool {
        self.documents.get(path).is_some_and(|document| {
            self.servers
                .get(document.language)
                .is_some_and(Slot::is_running)
        })
    }

    pub fn sync(&mut self, path: &str, buffer: &Buffer) {
        let Some((language, id)) = language_of(path) else {
            return;
        };
        if Path::new(path).starts_with(&self.sources) {
            return;
        }
        if self
            .documents
            .get(path)
            .is_some_and(|document| document.revision == buffer.revision())
        {
            return;
        }
        let uri = uri(&self.root.join(path));
        let Some((ctx, installer)) = &self.start else {
            return;
        };
        let Some(running) = self
            .servers
            .entry(language.name)
            .or_insert_with(|| find(language, installer, ctx))
            .running()
        else {
            return;
        };
        let text = buffer.text();
        match self.documents.get_mut(path) {
            Some(document) => {
                document.version += 1;
                document.revision = buffer.revision();
                document.lines = buffer.lines().to_vec();
                running.server.notify(
                    "textDocument/didChange",
                    json!({
                        "textDocument": {"uri": uri, "version": document.version},
                        "contentChanges": [{"text": text}],
                    }),
                );
            }
            None => {
                running.server.notify(
                    "textDocument/didOpen",
                    json!({"textDocument": {
                        "uri": uri, "languageId": id, "version": 1, "text": text,
                    }}),
                );
                self.documents.insert(
                    path.to_string(),
                    Document {
                        uri,
                        language: language.name,
                        version: 1,
                        revision: buffer.revision(),
                        lines: buffer.lines().to_vec(),
                    },
                );
            }
        }
    }

    pub fn saved(&mut self, path: &str) {
        self.tell(path, "textDocument/didSave");
    }

    pub fn close(&mut self, path: &str) {
        self.tell(path, "textDocument/didClose");
        self.documents.remove(path);
    }

    fn tell(&mut self, path: &str, method: &str) {
        if let Some((running, document)) = self.running(path)
            && document.uri.starts_with("file://")
        {
            let uri = document.uri.clone();
            running
                .server
                .notify(method, json!({"textDocument": {"uri": uri}}));
        }
    }

    fn ask(&mut self, path: &str, pos: Pos, method: &str, request: Request) {
        let Some((running, document)) = self.running(path) else {
            return;
        };
        let position = to_lsp(&document.lines, pos, running.server.encoding);
        let uri = document.uri.clone();
        let id = running.server.request(
            method,
            json!({"textDocument": {"uri": uri}, "position": position}),
        );
        running.pending.insert(id, request);
    }

    pub fn hover(&mut self, path: &str, pos: Pos) {
        let request = Request::Hover {
            path: path.to_string(),
            pos,
        };
        self.ask(path, pos, "textDocument/hover", request);
    }

    pub fn definition(&mut self, path: &str, pos: Pos) {
        self.ask(path, pos, "textDocument/definition", Request::Definition);
    }

    pub fn poll(&mut self) -> Vec<Event> {
        let mut events = Vec::new();
        let mut published = Vec::new();
        let mut locations = Vec::new();
        let mut contents = Vec::new();
        if let Some((ctx, installer)) = &self.start {
            for slot in self.servers.values_mut() {
                let Slot::Finding { found, .. } = slot else {
                    continue;
                };
                let started = match found.try_recv() {
                    Err(TryRecvError::Empty) => continue,
                    Err(TryRecvError::Disconnected) => {
                        Err("The language server could not be found".to_string())
                    }
                    Ok(command) => command.and_then(|command| {
                        Server::spawn(&command, &self.root, &installer.path_env, ctx.clone())
                    }),
                };
                *slot = match started {
                    Ok(server) => Slot::Running(Running {
                        server,
                        pending: HashMap::new(),
                    }),
                    Err(error) => Slot::Failed(error),
                };
            }
        }
        for (language, slot) in &mut self.servers {
            let Some(running) = slot.running() else {
                continue;
            };
            let messages = running.server.poll();
            let encoding = running.server.encoding;
            for message in messages {
                match message {
                    Message::Response { id, result } => match running.pending.remove(&id) {
                        Some(Request::Hover { path, pos }) => {
                            if let Some(text) = hover_text(&result["contents"]) {
                                events.push(Event::Hover { path, pos, text });
                            }
                        }
                        Some(Request::Definition) => match location(&result) {
                            Some((uri, range)) if !uri.starts_with("file://") => {
                                let id = running
                                    .server
                                    .request("workspace/textDocumentContent", json!({"uri": uri}));
                                running.pending.insert(id, Request::Content { uri, range });
                            }
                            Some(location) => locations.push((location, encoding)),
                            None => {}
                        },
                        Some(Request::Content { uri, range }) => {
                            if let Some(text) = result["text"].as_str() {
                                contents.push((*language, uri, range, text.to_string(), encoding));
                            }
                        }
                        None => {}
                    },
                    Message::Notification { method, params }
                        if method == "textDocument/publishDiagnostics" =>
                    {
                        published.push((params, encoding));
                    }
                    Message::Notification { .. } => {}
                }
            }
        }
        for (params, encoding) in published {
            let Some(path) = params["uri"].as_str().and_then(|uri| self.relative(uri)) else {
                continue;
            };
            let lines = self.lines_of(&path);
            let diagnostics: Vec<Diagnostic> = params["diagnostics"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|diagnostic| self::diagnostic(diagnostic, &lines, encoding))
                .collect();
            if diagnostics.is_empty() {
                self.diagnostics.remove(&path);
            } else {
                self.diagnostics.insert(path, Arc::new(diagnostics));
            }
        }
        for (language, uri, range, text, encoding) in contents {
            if let Some(path) = self.keep_source(language, uri, &text) {
                events.push(self.definition_event(path, &range, encoding));
            }
        }
        for ((uri, range), encoding) in locations {
            if let Some(path) = self.relative(&uri) {
                events.push(self.definition_event(path, &range, encoding));
            }
        }
        events
    }

    fn definition_event(&self, path: String, range: &Value, encoding: Encoding) -> Event {
        let lines = self.lines_of(&path);
        Event::Definition {
            start: from_lsp(&lines, &range["start"], encoding),
            end: from_lsp(&lines, &range["end"], encoding),
            path,
        }
    }

    fn keep_source(&mut self, language: &'static str, uri: String, text: &str) -> Option<String> {
        let (scheme, rest) = uri.split_once(':')?;
        let inner = path_of(&format!("file:///{}", rest.trim_start_matches('/')))?;
        let file = inner
            .components()
            .filter_map(|part| match part {
                std::path::Component::Normal(name) => Some(name),
                _ => None,
            })
            .fold(self.sources.join(scheme), |path, name| path.join(name));
        std::fs::create_dir_all(file.parent()?).ok()?;
        std::fs::write(&file, text).ok()?;
        let path = file.to_string_lossy().into_owned();
        self.documents.insert(
            path.clone(),
            Document {
                uri,
                language,
                version: 0,
                revision: 0,
                lines: text.lines().map(str::to_string).collect(),
            },
        );
        Some(path)
    }

    pub fn diagnostics(&self, path: &str) -> Arc<Vec<Diagnostic>> {
        self.diagnostics.get(path).cloned().unwrap_or_default()
    }

    pub fn problem_count(&self) -> usize {
        self.diagnostics.values().map(|all| all.len()).sum()
    }

    pub fn status(&self, path: &str) -> Option<String> {
        let (language, _) = language_of(path)?;
        self.start.as_ref()?;
        match self.servers.get(language.name)? {
            Slot::Failed(error) => Some(error.clone()),
            Slot::Finding { progress, .. } => {
                Some(progress.text().unwrap_or_else(|| {
                    format!("Looking for the {} language server…", language.name)
                }))
            }
            Slot::Running(running) if running.server.stopped => {
                Some(match running.server.last_complaint() {
                    Some(complaint) => format!(
                        "The {} language server stopped: {}",
                        language.name,
                        complaint.trim_end_matches('.')
                    ),
                    None => format!("The {} language server stopped", language.name),
                })
            }
            Slot::Running(running) if !running.server.is_ready() => {
                Some(format!("Starting the {} language server…", language.name))
            }
            Slot::Running(running) => running
                .server
                .progress()
                .map(|progress| format!("{}: {progress}", language.name)),
        }
    }

    pub fn report(&self, only: Option<&str>) -> String {
        let mut paths: Vec<&String> = self
            .diagnostics
            .keys()
            .filter(|path| only.is_none_or(|only| only == path.as_str()))
            .collect();
        paths.sort();
        let lines: Vec<String> = paths
            .into_iter()
            .flat_map(|path| {
                self.diagnostics[path].iter().map(move |diagnostic| {
                    format!(
                        "{path}:{}:{}: {}: {}",
                        diagnostic.start.line + 1,
                        diagnostic.start.col + 1,
                        diagnostic.severity.name(),
                        diagnostic.message
                    )
                })
            })
            .collect();
        if !lines.is_empty() {
            return lines.join("\n");
        }
        let running: Vec<&str> = self
            .servers
            .iter()
            .filter(|(_, slot)| slot.is_running())
            .map(|(name, _)| *name)
            .collect();
        match (only, running.is_empty()) {
            (_, true) => {
                "No problems: no language server is running; one starts when a file of its \
                          language is open"
                    .to_string()
            }
            (Some(path), false) => format!("No problems found in {path}"),
            (None, false) => format!(
                "No problems found by the language servers ({})",
                running.join(", ")
            ),
        }
    }

    pub fn shutdown(&mut self) {
        self.servers.clear();
    }

    fn relative(&self, uri: &str) -> Option<String> {
        let path = path_of(uri)?;
        let canonical = path.canonicalize().ok();
        let relative = [Some(&path), canonical.as_ref()]
            .into_iter()
            .flatten()
            .find_map(|path| {
                path.strip_prefix(&self.root)
                    .or_else(|_| path.strip_prefix(&self.canonical_root))
                    .ok()
            })
            .unwrap_or(&path);
        Some(relative.to_string_lossy().into_owned())
    }

    fn lines_of(&self, path: &str) -> Vec<String> {
        match self.documents.get(path) {
            Some(document) => document.lines.clone(),
            None => crate::editor::file_view::read(&self.root.join(path))
                .map(|text| text.lines().map(str::to_string).collect())
                .unwrap_or_default(),
        }
    }
}

fn find(language: &'static Language, installer: &Installer, ctx: &egui::Context) -> Slot {
    let (sender, found) = mpsc::channel();
    let progress = Progress::new(ctx.clone());
    let (installer, ctx, held) = (installer.clone(), ctx.clone(), progress.clone());
    std::thread::spawn(move || {
        let _ = sender.send(installer.find(language, &held));
        ctx.request_repaint();
    });
    Slot::Finding { found, progress }
}

const UNRESERVED: &[u8] = b"-._~/";

pub fn uri(path: &Path) -> String {
    let mut out = String::from("file://");
    for byte in path.to_string_lossy().bytes() {
        if byte.is_ascii_alphanumeric() || UNRESERVED.contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

pub fn path_of(uri: &str) -> Option<PathBuf> {
    let encoded = uri.strip_prefix("file://")?.as_bytes();
    let mut bytes = Vec::with_capacity(encoded.len());
    let mut at = 0;
    while at < encoded.len() {
        let hex = encoded
            .get(at + 1..at + 3)
            .and_then(|hex| std::str::from_utf8(hex).ok())
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match (encoded[at], hex) {
            (b'%', Some(byte)) => {
                bytes.push(byte);
                at += 3;
            }
            (byte, _) => {
                bytes.push(byte);
                at += 1;
            }
        }
    }
    Some(PathBuf::from(String::from_utf8_lossy(&bytes).into_owned()))
}

fn units(c: char, encoding: Encoding) -> usize {
    match encoding {
        Encoding::Utf16 => c.len_utf16(),
        Encoding::Utf32 => 1,
    }
}

pub fn to_lsp(lines: &[String], pos: Pos, encoding: Encoding) -> Value {
    let line = lines.get(pos.line).map_or("", String::as_str);
    let character: usize = line.chars().take(pos.col).map(|c| units(c, encoding)).sum();
    json!({"line": pos.line, "character": character})
}

pub fn from_lsp(lines: &[String], position: &Value, encoding: Encoding) -> Pos {
    let line = position["line"].as_u64().unwrap_or(0) as usize;
    let character = position["character"].as_u64().unwrap_or(0) as usize;
    let mut used = 0;
    let col = lines
        .get(line)
        .map_or("", String::as_str)
        .chars()
        .take_while(|c| {
            used += units(*c, encoding);
            used <= character
        })
        .count();
    Pos::new(line, col)
}

fn diagnostic(value: &Value, lines: &[String], encoding: Encoding) -> Option<Diagnostic> {
    let severity = match value["severity"].as_u64().unwrap_or(1) {
        1 => Severity::Error,
        2 => Severity::Warning,
        3 => Severity::Information,
        _ => return None,
    };
    Some(Diagnostic {
        start: from_lsp(lines, &value["range"]["start"], encoding),
        end: from_lsp(lines, &value["range"]["end"], encoding),
        severity,
        message: value["message"].as_str()?.to_string(),
    })
}

fn location(result: &Value) -> Option<(String, Value)> {
    let first = match result {
        Value::Array(all) => all.first()?,
        other => other,
    };
    let uri = first["uri"].as_str().or(first["targetUri"].as_str())?;
    let range = [&first["targetSelectionRange"], &first["range"]]
        .into_iter()
        .find(|range| range.is_object())?;
    Some((uri.to_string(), range.clone()))
}

fn hover_text(contents: &Value) -> Option<String> {
    let text = match contents {
        Value::String(text) => text.clone(),
        Value::Array(all) => all
            .iter()
            .filter_map(hover_text)
            .collect::<Vec<_>>()
            .join("\n\n"),
        Value::Object(_) => {
            let value = contents["value"].as_str()?;
            match contents["language"].as_str() {
                Some(language) => format!("```{language}\n{value}\n```"),
                None => value.to_string(),
            }
        }
        _ => return None,
    };
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}
