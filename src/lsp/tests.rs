use std::io::{BufReader, PipeWriter};
use std::time::{Duration, Instant};

use super::*;
use crate::testing::TempDir;

pub struct Peer {
    messages: std::sync::mpsc::Receiver<Value>,
    writer: PipeWriter,
}

impl Peer {
    pub fn next(&mut self) -> Value {
        self.messages
            .recv_timeout(Duration::from_secs(5))
            .expect("no message from the editor")
    }

    pub fn send(&mut self, message: Value) {
        transport::write(&mut self.writer, &message).unwrap();
    }

    pub fn answer_initialize(&mut self) {
        let initialize = self.next();
        assert_eq!(initialize["method"], "initialize");
        self.send(
            json!({"jsonrpc": "2.0", "id": initialize["id"], "result": {"capabilities": {}}}),
        );
    }
}

pub fn connect(root: &Path) -> (Server, Peer) {
    let (from_server, to_peer) = std::io::pipe().unwrap();
    let (from_peer, to_server) = std::io::pipe().unwrap();
    let server = Server::connect(from_peer, to_peer, root, egui::Context::default());
    let (sender, messages) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(from_server);
        while let Ok(Some(message)) = transport::read(&mut reader) {
            if sender.send(message).is_err() {
                break;
            }
        }
    });
    let peer = Peer {
        messages,
        writer: to_server,
    };
    (server, peer)
}

fn lsp_with_peer(dir: &TempDir) -> (Lsp, Peer) {
    let (server, peer) = connect(dir.path());
    let mut lsp = Lsp::new(dir.path().to_path_buf());
    lsp.connect("Kotlin", server);
    (lsp, peer)
}

fn ready(lsp: &mut Lsp, peer: &mut Peer) {
    peer.answer_initialize();
    poll_until(lsp, |lsp| lsp.status("a.kt").is_none());
    assert_eq!(peer.next()["method"], "initialized");
}

fn started(dir: &TempDir) -> (Lsp, Peer) {
    let (mut lsp, mut peer) = lsp_with_peer(dir);
    ready(&mut lsp, &mut peer);
    (lsp, peer)
}

fn poll_until(lsp: &mut Lsp, done: impl Fn(&Lsp) -> bool) -> Vec<Event> {
    let start = Instant::now();
    let mut events = Vec::new();
    loop {
        events.extend(lsp.poll());
        if done(lsp) {
            return events;
        }
        assert!(start.elapsed() < Duration::from_secs(5), "timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn events_until(lsp: &mut Lsp, count: usize) -> Vec<Event> {
    let start = Instant::now();
    let mut events = Vec::new();
    while events.len() < count {
        events.extend(lsp.poll());
        assert!(start.elapsed() < Duration::from_secs(5), "timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
    events
}

#[test]
fn knows_the_language_of_a_file() {
    assert_eq!(
        language_of("src/Main.kt").map(|(l, id)| (l.name, id)),
        Some(("Kotlin", "kotlin"))
    );
    assert_eq!(
        language_of("build.gradle.kts").map(|(_, id)| id),
        Some("kotlin")
    );
    assert_eq!(
        language_of("web/App.tsx").map(|(l, id)| (l.name, id)),
        Some(("TypeScript", "typescriptreact"))
    );
    assert_eq!(
        language_of("src/routes/+page.svelte").map(|(l, id)| (l.name, id)),
        Some(("Svelte", "svelte"))
    );
    assert!(language_of("README.md").is_none());
    assert!(language_of("Makefile").is_none());
}

#[test]
fn turns_paths_into_uris_and_back() {
    let path = Path::new("/work/my project/Ä.kt");
    assert_eq!(uri(path), "file:///work/my%20project/%C3%84.kt");
    assert_eq!(path_of(&uri(path)).unwrap(), path);
    assert_eq!(path_of("jar:file:///a.jar!/b.kt"), None);
}

#[test]
fn counts_columns_in_the_units_of_the_server() {
    let lines = vec!["a😀b".to_string()];
    assert_eq!(
        to_lsp(&lines, Pos::new(0, 2), Encoding::Utf16)["character"],
        3
    );
    assert_eq!(
        to_lsp(&lines, Pos::new(0, 2), Encoding::Utf32)["character"],
        2
    );
    let at = |character: usize, encoding| {
        from_lsp(
            &lines,
            &json!({"line": 0, "character": character}),
            encoding,
        )
    };
    assert_eq!(at(3, Encoding::Utf16), Pos::new(0, 2));
    assert_eq!(at(0, Encoding::Utf16), Pos::new(0, 0));
    assert_eq!(at(1, Encoding::Utf16), Pos::new(0, 1));
    assert_eq!(at(9, Encoding::Utf16), Pos::new(0, 3));
    assert_eq!(at(2, Encoding::Utf32), Pos::new(0, 2));
}

#[test]
fn opens_a_file_once_the_server_is_ready_and_sends_its_edits() {
    let dir = TempDir::new("lsp-sync");
    let (mut lsp, mut peer) = lsp_with_peer(&dir);
    let mut buffer = Buffer::new("fun main() {}\n");
    lsp.sync("a.kt", &buffer);
    lsp.sync("a.kt", &buffer);
    assert_eq!(
        lsp.status("a.kt").unwrap(),
        "Starting the Kotlin language server…"
    );

    ready(&mut lsp, &mut peer);
    let opened = peer.next();
    assert_eq!(opened["method"], "textDocument/didOpen");
    let document = &opened["params"]["textDocument"];
    assert_eq!(document["uri"], uri(&dir.path().join("a.kt")));
    assert_eq!(document["languageId"], "kotlin");
    assert_eq!(document["text"], "fun main() {}\n");

    buffer.type_text("// ");
    lsp.sync("a.kt", &buffer);
    let changed = peer.next();
    assert_eq!(changed["method"], "textDocument/didChange");
    assert_eq!(changed["params"]["textDocument"]["version"], 2);
    assert_eq!(
        changed["params"]["contentChanges"][0]["text"],
        "// fun main() {}\n"
    );

    lsp.saved("a.kt");
    assert_eq!(peer.next()["method"], "textDocument/didSave");
    lsp.close("a.kt");
    assert_eq!(peer.next()["method"], "textDocument/didClose");
    lsp.saved("a.kt");
    lsp.sync("b.txt", &buffer);
    lsp.hover("a.kt", Pos::default());
    lsp.sync("a.kt", &buffer);
    assert_eq!(peer.next()["method"], "textDocument/didOpen");
}

#[test]
fn shows_the_diagnostics_of_a_file_in_characters() {
    let dir = TempDir::new("lsp-diagnostics");
    let (mut lsp, mut peer) = started(&dir);
    lsp.sync("a.kt", &Buffer::new("val ä = 😀x\n"));
    peer.next();
    let range = |line, from, to| json!({"start": {"line": line, "character": from}, "end": {"line": line, "character": to}});
    peer.send(
        json!({"jsonrpc": "2.0", "method": "textDocument/publishDiagnostics", "params": {
            "uri": uri(&dir.path().join("a.kt")),
            "diagnostics": [
                {"range": range(0, 10, 11), "severity": 1, "message": "Unresolved reference 'x'"},
                {"range": range(0, 4, 5), "severity": 2, "message": "Unused variable"},
                {"range": range(0, 0, 3), "severity": 4, "message": "Hint"},
            ],
        }}),
    );
    poll_until(&mut lsp, |lsp| lsp.problem_count() > 0);
    let diagnostics = lsp.diagnostics("a.kt");
    assert_eq!(diagnostics.len(), 2);
    assert_eq!(
        (diagnostics[0].start, diagnostics[0].end),
        (Pos::new(0, 9), Pos::new(0, 10))
    );
    assert_eq!(diagnostics[0].severity, Severity::Error);
    assert!(diagnostics[0].covers(Pos::new(0, 9)));
    assert!(!diagnostics[0].covers(Pos::new(0, 10)));
    assert_eq!(
        lsp.report(None),
        "a.kt:1:10: error: Unresolved reference 'x'\na.kt:1:5: warning: Unused variable"
    );
    assert_eq!(lsp.report(Some("b.kt")), "No problems found in b.kt");

    peer.send(
        json!({"jsonrpc": "2.0", "method": "textDocument/publishDiagnostics", "params": {
            "uri": uri(&dir.path().join("a.kt")), "diagnostics": [],
        }}),
    );
    poll_until(&mut lsp, |lsp| lsp.problem_count() == 0);
    assert!(lsp.diagnostics("a.kt").is_empty());
}

#[test]
fn asks_for_hover_text_at_a_place() {
    let dir = TempDir::new("lsp-hover");
    let (mut lsp, mut peer) = started(&dir);
    lsp.sync("a.kt", &Buffer::new("😀 main()\n"));
    peer.next();
    lsp.hover("a.kt", Pos::new(0, 3));
    let request = peer.next();
    assert_eq!(request["method"], "textDocument/hover");
    assert_eq!(
        request["params"]["position"],
        json!({"line": 0, "character": 4})
    );
    peer.send(json!({"jsonrpc": "2.0", "id": request["id"], "result": {
        "contents": {"kind": "markdown", "value": "```kotlin\nfun main(): Unit\n```\n\nRuns it"},
    }}));
    let events = events_until(&mut lsp, 1);
    assert_eq!(
        events,
        [Event::Hover {
            path: "a.kt".to_string(),
            pos: Pos::new(0, 3),
            text: "fun main(): Unit\n\nRuns it".to_string(),
        }]
    );
}

#[test]
fn goes_to_a_definition_in_another_file() {
    let dir = TempDir::new("lsp-definition");
    dir.write("src/B.kt", "class 😀B\n");
    let (mut lsp, mut peer) = started(&dir);
    lsp.sync("a.kt", &Buffer::new("val b = B()\n"));
    peer.next();
    lsp.definition("a.kt", Pos::new(0, 8));
    let request = peer.next();
    assert_eq!(request["method"], "textDocument/definition");
    peer.send(json!({"jsonrpc": "2.0", "id": request["id"], "result": [{
        "targetUri": uri(&dir.path().join("src/B.kt")),
        "targetRange": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 9}},
        "targetSelectionRange": {"start": {"line": 0, "character": 8}, "end": {"line": 0, "character": 9}},
    }]}));
    let events = events_until(&mut lsp, 1);
    assert_eq!(
        events,
        [Event::Definition {
            path: "src/B.kt".to_string(),
            start: Pos::new(0, 7),
            end: Pos::new(0, 8),
        }]
    );
}

#[test]
fn answers_what_the_server_asks() {
    let dir = TempDir::new("lsp-requests");
    let (mut lsp, mut peer) = started(&dir);
    peer.send(
        json!({"jsonrpc": "2.0", "id": 7, "method": "workspace/configuration",
        "params": {"items": [{"section": "a"}, {"section": "b"}]}}),
    );
    peer.send(
        json!({"jsonrpc": "2.0", "id": "x", "method": "window/workDoneProgress/create",
        "params": {"token": "t"}}),
    );
    peer.send(json!({"jsonrpc": "2.0", "method": "textDocument/publishDiagnostics", "params": {
        "uri": uri(&dir.path().join("a.kt")),
        "diagnostics": [{"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 0}}, "message": "m"}],
    }}));
    poll_until(&mut lsp, |lsp| lsp.problem_count() > 0);
    assert_eq!(
        peer.next(),
        json!({"jsonrpc": "2.0", "id": 7, "result": [null, null]})
    );
    assert_eq!(
        peer.next(),
        json!({"jsonrpc": "2.0", "id": "x", "result": null})
    );
}

#[test]
fn shows_what_the_server_is_busy_with() {
    let dir = TempDir::new("lsp-progress");
    let (mut lsp, mut peer) = started(&dir);
    lsp.sync("a.kt", &Buffer::new(""));
    peer.next();
    let progress = |value: Value| json!({"jsonrpc": "2.0", "method": "$/progress", "params": {"token": 1, "value": value}});
    peer.send(progress(
        json!({"kind": "begin", "title": "Importing Gradle project"}),
    ));
    poll_until(&mut lsp, |lsp| lsp.status("a.kt").is_some());
    assert_eq!(
        lsp.status("a.kt").unwrap(),
        "Kotlin: Importing Gradle project"
    );
    peer.send(progress(json!({"kind": "report", "message": "3/10"})));
    poll_until(&mut lsp, |lsp| {
        lsp.status("a.kt").is_some_and(|s| s.ends_with("3/10"))
    });
    assert_eq!(
        lsp.status("a.kt").unwrap(),
        "Kotlin: Importing Gradle project 3/10"
    );
    peer.send(progress(json!({"kind": "end"})));
    poll_until(&mut lsp, |lsp| lsp.status("a.kt").is_none());
}

#[test]
fn tells_when_the_server_stops() {
    let dir = TempDir::new("lsp-stopped");
    let (mut lsp, peer) = started(&dir);
    lsp.sync("a.kt", &Buffer::new(""));
    assert!(lsp.has_server("a.kt"));
    drop(peer);
    poll_until(&mut lsp, |lsp| lsp.status("a.kt").is_some());
    assert_eq!(
        lsp.status("a.kt").unwrap(),
        "The Kotlin language server stopped"
    );
    assert!(!lsp.has_server("a.kt"));
}

#[test]
fn tells_why_a_server_stopped() {
    let dir = TempDir::new("lsp-complaint");
    let command = ["/bin/sh", "-c", "echo 'no such toolchain' >&2"].map(str::to_string);
    let server = Server::spawn(&command, dir.path(), "", egui::Context::default()).unwrap();
    let mut lsp = Lsp::new(dir.path().to_path_buf());
    lsp.connect("Kotlin", server);
    poll_until(&mut lsp, |lsp| {
        lsp.status("a.kt")
            .is_some_and(|status| status.contains("toolchain"))
    });
    assert_eq!(
        lsp.status("a.kt").unwrap(),
        "The Kotlin language server stopped: no such toolchain"
    );
}

#[test]
fn looks_for_a_server_away_from_the_editor() {
    let dir = TempDir::new("lsp-missing");
    let mut lsp = Lsp::new(dir.path().to_path_buf());
    lsp.sync("a.kt", &Buffer::new(""));
    assert_eq!(lsp.status("a.kt"), None);
    let installer = Installer::new(
        dir.path().join("servers"),
        dir.path().display().to_string(),
        false,
    );
    lsp.enable(egui::Context::default(), installer);
    lsp.sync("a.kt", &Buffer::new(""));
    lsp.sync("notes.md", &Buffer::new(""));
    assert!(lsp.status("a.kt").is_some());
    assert_eq!(lsp.status("notes.md"), None);
    poll_until(&mut lsp, |lsp| {
        lsp.status("a.kt")
            .is_some_and(|status| status.starts_with("No Kotlin"))
    });
    assert_eq!(
        lsp.status("a.kt").unwrap(),
        "No Kotlin language server found: kotlin-lsp is not on the PATH and downloading \
         language servers is turned off"
    );
    assert!(
        lsp.report(None)
            .starts_with("No problems: no language server is running")
    );
}
