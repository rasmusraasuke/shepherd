use std::io::{BufRead as _, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;

use eframe::egui;
use serde_json::{Value, json};

use super::{transport, uri};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Encoding {
    Utf16,
    Utf32,
}

#[derive(Debug, PartialEq)]
pub enum Message {
    Response { id: u64, result: Value },
    Notification { method: String, params: Value },
}

struct Progress {
    token: Value,
    title: String,
    message: String,
}

pub struct Server {
    outgoing: Sender<Value>,
    incoming: Receiver<Value>,
    child: Option<Child>,
    complaint: Arc<Mutex<Option<String>>>,
    next_id: u64,
    ready: bool,
    waiting: Vec<Value>,
    progress: Vec<Progress>,
    pub encoding: Encoding,
    pub stopped: bool,
}

impl Server {
    pub fn spawn(
        command: &[String],
        root: &Path,
        path_env: &str,
        ctx: egui::Context,
    ) -> Result<Self, String> {
        let program = Path::new(command.first().ok_or("No language server to start")?);
        let mut child = Command::new(program)
            .args(&command[1..])
            .current_dir(root)
            .env("PATH", path_env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Cannot start {}: {e}", program.display()))?;
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            return Err(format!("Cannot talk to {}", program.display()));
        };
        let mut server = Self::connect(stdout, stdin, root, ctx);
        if let Some(stderr) = child.stderr.take() {
            let complaint = server.complaint.clone();
            thread::spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    if !line.trim().is_empty()
                        && let Ok(mut held) = complaint.lock()
                    {
                        *held = Some(line.trim().to_string());
                    }
                }
            });
        }
        server.child = Some(child);
        Ok(server)
    }

    pub fn connect(
        reader: impl Read + Send + 'static,
        writer: impl Write + Send + 'static,
        root: &Path,
        ctx: egui::Context,
    ) -> Self {
        let (outgoing, to_write) = mpsc::channel::<Value>();
        thread::spawn(move || {
            let mut writer = writer;
            for message in to_write {
                if transport::write(&mut writer, &message).is_err() {
                    break;
                }
            }
        });
        let (sender, incoming) = mpsc::channel();
        thread::spawn(move || {
            let mut reader = BufReader::new(reader);
            while let Ok(Some(message)) = transport::read(&mut reader) {
                if sender.send(message).is_err() {
                    break;
                }
                ctx.request_repaint();
            }
            drop(sender);
            ctx.request_repaint();
        });
        let _ = outgoing.send(json!({
            "jsonrpc": "2.0",
            "id": 0,
            "method": "initialize",
            "params": initialize_params(root),
        }));
        Self {
            outgoing,
            incoming,
            child: None,
            complaint: Arc::default(),
            next_id: 1,
            ready: false,
            waiting: Vec::new(),
            progress: Vec::new(),
            encoding: Encoding::Utf16,
            stopped: false,
        }
    }

    pub fn last_complaint(&self) -> Option<String> {
        self.complaint.lock().ok()?.clone()
    }

    pub fn is_ready(&self) -> bool {
        self.ready
    }

    pub fn progress(&self) -> Option<String> {
        let progress = self.progress.last()?;
        Some(
            [progress.title.as_str(), progress.message.as_str()]
                .into_iter()
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join(" "),
        )
    }

    pub fn request(&mut self, method: &str, params: Value) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        id
    }

    pub fn notify(&mut self, method: &str, params: Value) {
        self.send(json!({"jsonrpc": "2.0", "method": method, "params": params}));
    }

    fn send(&mut self, message: Value) {
        if self.ready {
            let _ = self.outgoing.send(message);
        } else {
            self.waiting.push(message);
        }
    }

    pub fn poll(&mut self) -> Vec<Message> {
        let mut messages = Vec::new();
        loop {
            match self.incoming.try_recv() {
                Ok(message) => messages.extend(self.handle(message)),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.stopped = true;
                    break;
                }
            }
        }
        messages
    }

    fn handle(&mut self, mut message: Value) -> Option<Message> {
        let method = message["method"].as_str().map(str::to_string);
        let params = message["params"].take();
        match (message.get("id").cloned(), method) {
            (Some(id), Some(method)) => {
                let result = match method.as_str() {
                    "workspace/configuration" => Value::Array(
                        params["items"]
                            .as_array()
                            .map_or(Vec::new(), |items| vec![Value::Null; items.len()]),
                    ),
                    _ => Value::Null,
                };
                let _ = self
                    .outgoing
                    .send(json!({"jsonrpc": "2.0", "id": id, "result": result}));
                None
            }
            (Some(id), None) => {
                let id = id.as_u64()?;
                let result = message["result"].take();
                if id == 0 {
                    self.initialized(&result);
                    return None;
                }
                Some(Message::Response { id, result })
            }
            (None, Some(method)) if method == "$/progress" => {
                self.track_progress(params);
                None
            }
            (None, Some(method)) => Some(Message::Notification { method, params }),
            (None, None) => None,
        }
    }

    fn initialized(&mut self, result: &Value) {
        if result["capabilities"]["positionEncoding"] == "utf-32" {
            self.encoding = Encoding::Utf32;
        }
        self.ready = true;
        self.notify("initialized", json!({}));
        for message in std::mem::take(&mut self.waiting) {
            let _ = self.outgoing.send(message);
        }
    }

    fn track_progress(&mut self, mut params: Value) {
        let token = params["token"].take();
        let value = &params["value"];
        let message = value["message"].as_str().map(str::to_string);
        let index = self.progress.iter().position(|held| held.token == token);
        match (value["kind"].as_str(), index) {
            (Some("begin"), _) => self.progress.push(Progress {
                token,
                title: value["title"].as_str().unwrap_or_default().to_string(),
                message: message.unwrap_or_default(),
            }),
            (Some("report"), Some(index)) => {
                if let Some(message) = message {
                    self.progress[index].message = message;
                }
            }
            (Some("end"), Some(index)) => {
                self.progress.remove(index);
            }
            _ => {}
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        if let Some(child) = &mut self.child {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn initialize_params(root: &Path) -> Value {
    let name = root
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    json!({
        "processId": std::process::id(),
        "clientInfo": {"name": "Shepherd", "version": env!("CARGO_PKG_VERSION")},
        "rootUri": uri(root),
        "workspaceFolders": [{"uri": uri(root), "name": name}],
        "capabilities": {
            "general": {"positionEncodings": ["utf-32", "utf-16"]},
            "window": {"workDoneProgress": true},
            "workspace": {
                "configuration": true,
                "workspaceFolders": true,
                "textDocumentContent": {}
            },
            "textDocument": {
                "synchronization": {"didSave": true},
                "hover": {"contentFormat": ["markdown", "plaintext"]},
                "definition": {"linkSupport": true},
                "publishDiagnostics": {}
            }
        }
    })
}
