use std::fs;
use std::io::{BufRead as _, BufReader, Write as _};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use eframe::egui;
use serde::{Deserialize, Serialize};

/// The variable that tells programs in the editor's terminals where to reach the editor.
pub const SOCKET_VARIABLE: &str = "EDITOR_SOCKET";

/// A directory younger than this may belong to an editor that is starting and isn't listening yet.
const STARTING: Duration = Duration::from_secs(10);

/// How long a program waits for the editor to answer.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(10);

/// What the editor answers.
#[derive(Serialize, Deserialize)]
struct Answer {
    ok: bool,
    output: String,
}

/// A command a program sent to the editor, waiting for its answer.
pub struct Request {
    pub args: Vec<String>,
    reply: Sender<Result<String, String>>,
}

impl Request {
    pub fn answer(self, answer: Result<String, String>) {
        let _ = self.reply.send(answer);
    }
}

/// Lets programs in the editor's terminals, such as agents, ask the editor for what it shows
/// and have it do things, with `editor ctl`. The editor listens on a socket in a directory of
/// its own, which also has the `editor` command in it for the terminals' PATH.
pub struct Control {
    dir: PathBuf,
    requests: Receiver<Request>,
}

impl Control {
    pub fn start(ctx: egui::Context) -> Result<Self, String> {
        static STARTED: AtomicUsize = AtomicUsize::new(0);
        remove_stale(&std::env::temp_dir(), STARTING);
        let dir = std::env::temp_dir().join(format!("editor-{}-{}", std::process::id(), STARTED.fetch_add(1, Ordering::SeqCst)));
        let error = |e: std::io::Error| format!("Agents can't reach the editor: {e}");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("bin")).map_err(error)?;
        let listener = UnixListener::bind(dir.join("control.sock")).map_err(error)?;
        if let Ok(exe) = std::env::current_exe() {
            std::os::unix::fs::symlink(exe, dir.join("bin/editor")).map_err(error)?;
        }
        let (sender, requests) = mpsc::channel();
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let (sender, ctx) = (sender.clone(), ctx.clone());
                thread::spawn(move || serve(stream, &sender, &ctx));
            }
        });
        Ok(Self { dir, requests })
    }

    pub fn socket(&self) -> PathBuf {
        self.dir.join("control.sock")
    }

    /// The directory with the `editor` command in it.
    pub fn bin_dir(&self) -> PathBuf {
        self.dir.join("bin")
    }

    /// The commands sent since the last call.
    pub fn requests(&self) -> Vec<Request> {
        self.requests.try_iter().collect()
    }
}

impl Drop for Control {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// Removes the directories of editors that ended without removing their own, as when they were
/// killed: nothing listens on their sockets any more. Directories younger than `starting` are left.
fn remove_stale(parent: &Path, starting: Duration) {
    let Ok(entries) = fs::read_dir(parent) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let ours = name.strip_prefix("editor-").and_then(|rest| rest.split_once('-')).is_some_and(|(pid, n)| pid.parse::<u32>().is_ok() && n.parse::<u32>().is_ok());
        let old = entry.metadata().and_then(|meta| meta.modified()).is_ok_and(|modified| modified.elapsed().unwrap_or_default() >= starting);
        if ours && old && UnixStream::connect(entry.path().join("control.sock")).is_err() {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

/// Reads one command from a program, has the editor answer it and passes the answer on.
fn serve(stream: UnixStream, sender: &Sender<Request>, ctx: &egui::Context) {
    let mut line = String::new();
    if BufReader::new(&stream).read_line(&mut line).is_err() {
        return;
    }
    let answer = match serde_json::from_str::<Vec<String>>(&line) {
        Ok(args) => {
            let (reply, answer) = mpsc::channel();
            let _ = sender.send(Request { args, reply });
            ctx.request_repaint();
            answer.recv_timeout(ANSWER_TIMEOUT).unwrap_or_else(|_| Err("The editor didn't answer".to_string()))
        }
        Err(e) => Err(format!("Not a command: {e}")),
    };
    let answer = match answer {
        Ok(output) => Answer { ok: true, output },
        Err(output) => Answer { ok: false, output },
    };
    let _ = writeln!(&stream, "{}", serde_json::to_string(&answer).unwrap_or_default());
}

/// Sends a command to the editor listening on the socket and returns its answer.
pub fn ask(socket: &Path, args: &[String]) -> Result<String, String> {
    let stream = UnixStream::connect(socket).map_err(|e| format!("Cannot reach the editor at {}: {e}", socket.display()))?;
    writeln!(&stream, "{}", serde_json::to_string(args).unwrap_or_default()).map_err(|e| e.to_string())?;
    let mut line = String::new();
    BufReader::new(&stream).read_line(&mut line).map_err(|e| e.to_string())?;
    let answer: Answer = serde_json::from_str(&line).map_err(|e| format!("The editor gave no answer: {e}"))?;
    if answer.ok { Ok(answer.output) } else { Err(answer.output) }
}

/// Runs `editor ctl` with the arguments that follow it, printing the editor's answer. Returns
/// the exit code.
pub fn run_command(args: &[String]) -> i32 {
    let Some(socket) = std::env::var_os(SOCKET_VARIABLE) else {
        eprintln!("editor ctl works in the terminals of the editor only (no {SOCKET_VARIABLE} set)");
        return 2;
    };
    match ask(Path::new(&socket), args) {
        Ok(output) => {
            println!("{}", output.trim_end());
            0
        }
        Err(error) => {
            eprintln!("{}", error.trim_end());
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_the_directories_of_editors_that_are_gone() {
        let parent = crate::testing::TempDir::new("stale");
        let dir = |name: &str| {
            let dir = parent.path().join(name);
            fs::create_dir_all(&dir).unwrap();
            dir
        };
        // its editor was killed: its socket is left, but nobody listens on it. Not made by
        // binding a socket, as a process started by another test may hold on to it for a moment
        fs::write(dir("editor-1-0").join("control.sock"), "").unwrap();
        let _running = UnixListener::bind(dir("editor-2-0").join("control.sock")).unwrap();
        dir("editor-notes");
        remove_stale(parent.path(), STARTING);
        assert!(parent.path().join("editor-1-0").exists(), "a young directory may be of an editor starting");
        remove_stale(parent.path(), Duration::ZERO);
        let mut left: Vec<String> = fs::read_dir(parent.path()).unwrap().map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned()).collect();
        left.sort();
        assert_eq!(left, ["editor-2-0", "editor-notes"]);
    }

    #[test]
    fn answers_commands_sent_over_the_socket() {
        let control = Control::start(egui::Context::default()).unwrap();
        let socket = control.socket();
        let asking = thread::spawn(move || (ask(&socket, &["open".to_string(), "a.rs".to_string()]), ask(&socket, &["wrong".to_string()])));
        let mut answered = 0;
        while answered < 2 {
            for request in control.requests() {
                let answer = if request.args[0] == "open" { Ok(format!("opened {}", request.args[1])) } else { Err("no such command".to_string()) };
                request.answer(answer);
                answered += 1;
            }
            thread::sleep(Duration::from_millis(5));
        }
        let (opened, wrong) = asking.join().unwrap();
        assert_eq!(opened, Ok("opened a.rs".to_string()));
        assert_eq!(wrong, Err("no such command".to_string()));
        assert!(control.bin_dir().join("editor").exists());
    }
}
