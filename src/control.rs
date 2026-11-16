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

pub const SOCKET_VARIABLE: &str = "SHEPHERD_SOCKET";

const STARTING: Duration = Duration::from_secs(10);

const ANSWER_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Serialize, Deserialize)]
struct Answer {
    ok: bool,
    output: String,
}

pub struct Request {
    pub args: Vec<String>,
    reply: Sender<Result<String, String>>,
}

impl Request {
    pub fn answer(self, answer: Result<String, String>) {
        let _ = self.reply.send(answer);
    }
}

pub struct Control {
    dir: PathBuf,
    requests: Receiver<Request>,
}

impl Control {
    pub fn start(ctx: egui::Context) -> Result<Self, String> {
        static STARTED: AtomicUsize = AtomicUsize::new(0);
        remove_stale(&std::env::temp_dir(), STARTING);
        let dir = std::env::temp_dir().join(format!(
            "shepherd-{}-{}",
            std::process::id(),
            STARTED.fetch_add(1, Ordering::SeqCst)
        ));
        let error = |e: std::io::Error| format!("Agents can't reach the editor: {e}");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("bin")).map_err(error)?;
        let listener = UnixListener::bind(dir.join("control.sock")).map_err(error)?;
        if let Ok(exe) = std::env::current_exe() {
            std::os::unix::fs::symlink(exe, dir.join("bin/shepherd")).map_err(error)?;
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

    pub fn bin_dir(&self) -> PathBuf {
        self.dir.join("bin")
    }

    pub fn requests(&self) -> Vec<Request> {
        self.requests.try_iter().collect()
    }
}

impl Drop for Control {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn remove_stale(parent: &Path, starting: Duration) {
    let Ok(entries) = fs::read_dir(parent) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let ours = name
            .strip_prefix("shepherd-")
            .and_then(|rest| rest.split_once('-'))
            .is_some_and(|(pid, n)| pid.parse::<u32>().is_ok() && n.parse::<u32>().is_ok());
        let old = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .is_ok_and(|modified| modified.elapsed().unwrap_or_default() >= starting);
        if ours && old && UnixStream::connect(entry.path().join("control.sock")).is_err() {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

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
            answer
                .recv_timeout(ANSWER_TIMEOUT)
                .unwrap_or_else(|_| Err("The editor didn't answer".to_string()))
        }
        Err(e) => Err(format!("Not a command: {e}")),
    };
    let answer = match answer {
        Ok(output) => Answer { ok: true, output },
        Err(output) => Answer { ok: false, output },
    };
    let _ = writeln!(
        &stream,
        "{}",
        serde_json::to_string(&answer).unwrap_or_default()
    );
}

pub fn stop_leftovers(socket: &Path) {
    stop(|path| doomed(path, socket));
}

fn doomed(socket: &Path, ours: &Path) -> bool {
    socket == ours || !socket.exists()
}

fn stop(doomed: impl Fn(&Path) -> bool) {
    for _ in 0..3 {
        let pids = leftover_pids(&doomed);
        if pids.is_empty() {
            break;
        }
        kill(&pids);
        thread::sleep(Duration::from_millis(50));
    }
}

fn kill(pids: &[libc::pid_t]) {
    let own = std::process::id() as libc::pid_t;
    for pid in pids.iter().filter(|pid| **pid != own) {
        unsafe {
            libc::kill(*pid, libc::SIGKILL);
        }
    }
}

fn leftover_pids(doomed: &impl Fn(&Path) -> bool) -> Vec<libc::pid_t> {
    let Ok(output) = std::process::Command::new("ps")
        .args(["eww", "-ax"])
        .output()
    else {
        return Vec::new();
    };
    leftover_pids_in(&String::from_utf8_lossy(&output.stdout), doomed)
}

fn leftover_pids_in(listing: &str, doomed: &impl Fn(&Path) -> bool) -> Vec<libc::pid_t> {
    listing
        .lines()
        .filter_map(leftover)
        .filter(|(_, socket)| doomed(socket))
        .map(|(pid, _)| pid)
        .collect()
}

fn leftover(line: &str) -> Option<(libc::pid_t, PathBuf)> {
    let mut words = line.split_whitespace();
    let pid = words.next()?.parse().ok()?;
    let marker = format!("{SOCKET_VARIABLE}=");
    let socket = words.find_map(|word| word.strip_prefix(&marker))?;
    Some((pid, PathBuf::from(socket)))
}

pub fn ask(socket: &Path, args: &[String]) -> Result<String, String> {
    let stream = UnixStream::connect(socket)
        .map_err(|e| format!("Cannot reach the editor at {}: {e}", socket.display()))?;
    writeln!(
        &stream,
        "{}",
        serde_json::to_string(args).unwrap_or_default()
    )
    .map_err(|e| e.to_string())?;
    let mut line = String::new();
    BufReader::new(&stream)
        .read_line(&mut line)
        .map_err(|e| e.to_string())?;
    let answer: Answer =
        serde_json::from_str(&line).map_err(|e| format!("The editor gave no answer: {e}"))?;
    if answer.ok {
        Ok(answer.output)
    } else {
        Err(answer.output)
    }
}

pub fn run_command(args: &[String]) -> i32 {
    let Some(socket) = std::env::var_os(SOCKET_VARIABLE) else {
        eprintln!(
            "shepherd ctl works in the terminals of the editor only (no {SOCKET_VARIABLE} set)"
        );
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
        fs::write(dir("shepherd-1-0").join("control.sock"), "").unwrap();
        let _running = UnixListener::bind(dir("shepherd-2-0").join("control.sock")).unwrap();
        dir("shepherd-notes");
        remove_stale(parent.path(), STARTING);
        assert!(
            parent.path().join("shepherd-1-0").exists(),
            "a young directory may be of an editor starting"
        );
        remove_stale(parent.path(), Duration::ZERO);
        let mut left: Vec<String> = fs::read_dir(parent.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left, ["shepherd-2-0", "shepherd-notes"]);
    }

    #[test]
    fn reads_the_socket_a_process_was_started_with() {
        let line = "14509   ??  Ss     0:15.50 /opt/opencode serve --service TERM=xterm \
                    SHEPHERD_SOCKET=/tmp/shepherd-1-0/control.sock PATH=/usr/bin";
        assert_eq!(
            leftover(line),
            Some((14509, PathBuf::from("/tmp/shepherd-1-0/control.sock")))
        );
        assert_eq!(leftover("    1   ??  Ss     0:00.10 /sbin/launchd"), None);
    }

    #[test]
    fn picks_the_processes_of_dead_editors() {
        let dir = crate::testing::TempDir::new("sweep");
        let ours = dir.path().join("ours.sock");
        std::fs::write(&ours, "").unwrap();
        let other = dir.path().join("other.sock");
        std::fs::write(&other, "").unwrap();
        let dead = dir.path().join("dead.sock");
        let listing = format!(
            "    1   ??  Ss     0:00.10 /sbin/launchd\n\
             14509   ??  Ss     0:15.50 /opt/opencode serve --service TERM=xterm \
             SHEPHERD_SOCKET={} PATH=/usr/bin\n\
             14982   ??  Ss     0:00.05 /cache/opencode-pty daemon \
             SHEPHERD_SOCKET={} PATH=/usr/bin\n\
             15000   ??  Ss     0:00.05 /cache/opencode-pty daemon \
             SHEPHERD_SOCKET={} PATH=/usr/bin\n",
            ours.display(),
            dead.display(),
            other.display()
        );
        let pick = |socket: &Path| doomed(socket, &ours);
        assert_eq!(leftover_pids_in(&listing, &pick), [14509, 14982]);
    }

    #[test]
    fn kills_only_processes_other_than_itself() {
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        kill(&[std::process::id() as libc::pid_t, child.id() as libc::pid_t]);
        let start = std::time::Instant::now();
        while child.try_wait().unwrap().is_none() {
            assert!(
                start.elapsed() < Duration::from_secs(5),
                "the process is still there"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn answers_commands_sent_over_the_socket() {
        let control = Control::start(egui::Context::default()).unwrap();
        let socket = control.socket();
        let asking = thread::spawn(move || {
            (
                ask(&socket, &["open".to_string(), "a.rs".to_string()]),
                ask(&socket, &["wrong".to_string()]),
            )
        });
        let mut answered = 0;
        while answered < 2 {
            for request in control.requests() {
                let answer = if request.args[0] == "open" {
                    Ok(format!("opened {}", request.args[1]))
                } else {
                    Err("no such command".to_string())
                };
                request.answer(answer);
                answered += 1;
            }
            thread::sleep(Duration::from_millis(5));
        }
        let (opened, wrong) = asking.join().unwrap();
        assert_eq!(opened, Ok("opened a.rs".to_string()));
        assert_eq!(wrong, Err("no such command".to_string()));
        assert!(control.bin_dir().join("shepherd").exists());
    }
}
