mod input;
mod paint;
mod session;
mod text;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use alacritty_terminal::event::{Event as TermEvent, EventListener};
use alacritty_terminal::event_loop::EventLoop;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::tty::{self, Shell};
use eframe::egui;

use crate::agents::images;
use crate::agents::{self, Agent, Start, Tuning};
use crate::editor::code::CodeStyle;
use input::paste_bytes;
use paint::screen_text;
use session::Terminal;

const PADDING: f32 = 4.0;

type ClipboardAnswer = Arc<dyn Fn(&str) -> String + Sync + Send>;

type Worker = std::thread::JoinHandle<(
    EventLoop<tty::Pty, Listener>,
    alacritty_terminal::event_loop::State,
)>;

const BUSY: Duration = Duration::from_millis(1500);

const ECHO: Duration = Duration::from_millis(1000);

const STOP_WAIT: Duration = Duration::from_millis(300);

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Activity {
    Working,
    Waiting,
    Idle,
    Ended,
}

#[derive(Default)]
struct Output {
    last: Option<Instant>,
    ended: bool,
    failed: bool,
}

#[derive(Clone, Copy, PartialEq, Debug)]
struct Size {
    columns: usize,
    lines: usize,
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.lines
    }

    fn screen_lines(&self) -> usize {
        self.lines
    }

    fn columns(&self) -> usize {
        self.columns
    }
}

#[derive(Clone)]
struct Listener {
    events: Sender<TermEvent>,
    ctx: egui::Context,
    output: Arc<Mutex<Output>>,
}

impl EventListener for Listener {
    fn send_event(&self, event: TermEvent) {
        if let Ok(mut output) = self.output.lock() {
            match event {
                TermEvent::Wakeup => output.last = Some(Instant::now()),
                TermEvent::ChildExit(status) => {
                    output.ended = true;
                    output.failed = !status.success();
                }
                TermEvent::Exit => output.ended = true,
                _ => {}
            }
        }
        let _ = self.events.send(event);
        self.ctx.request_repaint();
    }
}

pub struct Terminals {
    root: PathBuf,
    shell: Option<Shell>,
    terminals: HashMap<u64, Result<Terminal, String>>,
    focus_on_start: HashSet<u64>,
    read_clipboard: fn() -> Option<String>,
    image_dir: PathBuf,
    system_fonts: bool,
    last_used: Option<u64>,
    env: HashMap<String, String>,
    guide: Option<PathBuf>,
    agent_program: Option<String>,
    starts: HashMap<u64, Prompt>,
    background: Arc<Mutex<Vec<u32>>>,
}

fn read_clipboard() -> Option<String> {
    arboard::Clipboard::new().ok()?.get_text().ok()
}

#[derive(Default)]
pub struct Prompt {
    pub message: Option<String>,
    pub images: Vec<PathBuf>,
}

fn size_for(rect: egui::Rect, style: &CodeStyle) -> Size {
    Size {
        columns: (((rect.width() - 2.0 * PADDING) / style.digit_width) as usize).max(2),
        lines: (((rect.height() - 2.0 * PADDING) / style.row_height) as usize).max(1),
    }
}

impl Terminals {
    pub fn new(root: PathBuf) -> Self {
        let image_dir = images::dir();
        images::remove_old(&image_dir, images::OLD);
        Self {
            root,
            shell: None,
            terminals: HashMap::new(),
            focus_on_start: HashSet::new(),
            read_clipboard,
            image_dir,
            system_fonts: false,
            last_used: None,
            env: HashMap::new(),
            guide: None,
            agent_program: None,
            starts: HashMap::new(),
            background: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn image_dir(&self) -> PathBuf {
        self.image_dir.clone()
    }

    pub fn set_env(&mut self, name: &str, value: String) {
        self.env.insert(name.to_string(), value);
    }

    pub fn set_guide(&mut self, guide: PathBuf) {
        self.guide = Some(guide);
    }

    pub fn set_system_fonts(&mut self, system_fonts: bool) {
        self.system_fonts = system_fonts;
    }

    pub fn start(&mut self, id: u64, prompt: Prompt) {
        self.starts.insert(id, prompt);
        self.focus(id);
    }

    pub fn run_background(
        &self,
        agent: Agent,
        tuning: &Tuning,
        message: &str,
    ) -> Result<(), String> {
        let command = agent.background_command(message, self.guide.as_deref(), tuning);
        let program = self
            .agent_program
            .clone()
            .unwrap_or_else(|| command.program.to_string());
        let mut child = std::process::Command::new(&program);
        child
            .args(&command.args)
            .current_dir(&self.root)
            .envs(&self.env)
            .envs(command.env)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        let mut child = child
            .spawn()
            .map_err(|e| format!("Cannot start {program}: {e}"))?;
        let id = child.id();
        self.background.lock().unwrap().push(id);
        let background = self.background.clone();
        std::thread::spawn(move || {
            let _ = child.wait();
            background.lock().unwrap().retain(|pid| *pid != id);
        });
        Ok(())
    }

    fn running(&self) -> impl Iterator<Item = (u64, &Terminal)> {
        self.terminals
            .iter()
            .filter_map(|(id, terminal)| Some((*id, terminal.as_ref().ok()?)))
    }

    pub fn update(&mut self, ctx: &egui::Context) {
        let mut working = false;
        for terminal in self.terminals.values_mut().flatten() {
            terminal.update_activity();
            working |= terminal.activity == Activity::Working;
        }
        if working {
            ctx.request_repaint_after(BUSY / 3);
        }
    }

    pub fn activity(&self, id: u64) -> Option<Activity> {
        Some(self.terminals.get(&id)?.as_ref().ok()?.activity)
    }

    pub fn agents(&self) -> Vec<(u64, String)> {
        let mut agents: Vec<(u64, String)> = self
            .running()
            .filter(|(_, terminal)| terminal.activity != Activity::Ended)
            .filter_map(|(id, terminal)| Some((id, terminal.agent?.name().to_string())))
            .collect();
        agents.sort_by_key(|(id, _)| (Some(*id) != self.last_used, *id));
        agents
    }

    pub fn send(&mut self, id: u64, text: &str) {
        if let Some(Ok(terminal)) = self.terminals.get_mut(&id)
            && terminal.agent.is_some()
        {
            let mode = *terminal.term.lock().mode();
            let mut input = paste_bytes(text, mode);
            input.push(b'\r');
            terminal.last_input = Some(Instant::now());
            terminal.write(input);
        }
    }

    pub fn title(&self, id: u64) -> Option<String> {
        let terminal = self.terminals.get(&id)?.as_ref().ok()?;
        let title = match terminal.agent {
            Some(agent) => Some(agent.name().to_string()),
            None => terminal.title.clone().filter(|title| !title.is_empty()),
        };
        if terminal.activity == Activity::Ended {
            Some(format!(
                "{} (exited)",
                title.as_deref().unwrap_or("Terminal")
            ))
        } else {
            title
        }
    }

    pub fn focus(&mut self, id: u64) {
        match self.terminals.get_mut(&id) {
            Some(Ok(terminal)) => terminal.focus = true,
            _ => {
                self.focus_on_start.insert(id);
            }
        }
    }

    pub fn close(&mut self, id: u64) {
        self.terminals.remove(&id);
        self.focus_on_start.remove(&id);
        if let Some(pending) = self.starts.remove(&id) {
            images::remove(&pending.images);
        }
    }

    pub fn shutdown(&mut self) {
        let mut stopping: Vec<Terminal> = self
            .terminals
            .drain()
            .filter_map(|(_, terminal)| terminal.ok())
            .collect();
        for terminal in &mut stopping {
            terminal.stop();
        }
        if !stopping.is_empty() {
            std::thread::sleep(STOP_WAIT);
        }
        for terminal in &stopping {
            terminal.kill();
        }
        for pending in self.starts.drain().map(|(_, pending)| pending) {
            images::remove(&pending.images);
        }
        self.focus_on_start.clear();
        let pids = std::mem::take(&mut *self.background.lock().unwrap());
        for pid in pids {
            unsafe {
                libc::kill(pid as libc::pid_t, libc::SIGKILL);
            }
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, id: u64) {
        self.show(ui, id, None);
    }

    pub fn agent_ui(
        &mut self,
        ui: &mut egui::Ui,
        id: u64,
        agent: Agent,
        session: &str,
        tuning: &Tuning,
    ) -> Option<String> {
        self.show(ui, id, Some((agent, session, tuning)))
    }

    #[allow(clippy::too_many_arguments)]
    fn spawn(
        &self,
        id: u64,
        agent: Option<(Agent, &str, &Tuning)>,
        start: Start,
        size: Size,
        style: &CodeStyle,
        ctx: &egui::Context,
        focus: bool,
    ) -> Result<Terminal, String> {
        let mut env = self.env.clone();
        let resumed = agent.is_some() && matches!(start, Start::Continue);
        let shell = match agent {
            Some((agent, session, tuning)) => {
                let command = agent.command(session, start, self.guide.as_deref(), tuning);
                env.extend(command.env);
                env.insert(agents::AGENT_VARIABLE.to_string(), id.to_string());
                let program = self.agent_program.clone();
                Some(Shell::new(
                    program.unwrap_or_else(|| command.program.to_string()),
                    command.args,
                ))
            }
            None => self.shell.clone(),
        };
        let mut terminal = Terminal::spawn(&self.root, shell, env, size, style, ctx, focus)?;
        terminal.agent = agent.map(|(agent, _, _)| agent);
        terminal.resumed = resumed;
        Ok(terminal)
    }

    fn show(
        &mut self,
        ui: &mut egui::Ui,
        id: u64,
        agent: Option<(Agent, &str, &Tuning)>,
    ) -> Option<String> {
        let size = egui::TextStyle::Monospace.resolve(ui.style()).size;
        let fonts_ready = text::fonts_ready(ui.ctx(), self.system_fonts);
        let style = CodeStyle::with_font(
            ui,
            egui::FontId::new(size, text::family(fonts_ready, false, false)),
        );
        let size = size_for(ui.available_rect_before_wrap(), &style);
        if !self.terminals.contains_key(&id) {
            let pending = self.starts.remove(&id);
            let start = match &pending {
                Some(pending) => Start::New(pending.message.clone()),
                None => Start::Continue,
            };
            let focus = self.focus_on_start.remove(&id);
            let mut terminal = self.spawn(id, agent, start, size, &style, ui.ctx(), focus);
            if let (Ok(terminal), Some(pending)) = (&mut terminal, pending) {
                terminal.images = pending.images;
            }
            self.terminals.insert(id, terminal);
        }
        match self.terminals.get_mut(&id) {
            Some(Ok(terminal)) => {
                if terminal.ui(
                    ui,
                    id,
                    &style,
                    fonts_ready,
                    self.read_clipboard,
                    &self.image_dir,
                ) {
                    self.last_used = Some(id);
                }
                let again = if terminal.restart {
                    Some(Start::Continue)
                } else if terminal.resume_failed() {
                    Some(Start::New(None))
                } else {
                    None
                };
                if let Some(start) = again {
                    let fresh = matches!(start, Start::New(_));
                    let session = match (agent, fresh) {
                        (Some((agent, _, _)), true) => agent.new_session(),
                        (_, _) => {
                            agent.map_or_else(String::new, |(_, session, _)| session.to_string())
                        }
                    };
                    let target = agent.map(|(agent, _, tuning)| (agent, session.as_str(), tuning));
                    let images = match self.terminals.get_mut(&id) {
                        Some(Ok(old)) => std::mem::take(&mut old.images),
                        _ => Vec::new(),
                    };
                    let mut terminal = self.spawn(id, target, start, size, &style, ui.ctx(), true);
                    if let Ok(terminal) = &mut terminal {
                        terminal.images = images;
                    }
                    self.terminals.insert(id, terminal);
                    if fresh {
                        return Some(session);
                    }
                }
                None
            }
            Some(Err(message)) => {
                ui.centered_and_justified(|ui| {
                    ui.colored_label(ui.visuals().error_fg_color, message.as_str())
                });
                None
            }
            None => None,
        }
    }

    #[cfg(test)]
    pub fn run_agents_with(&mut self, program: &std::path::Path) {
        self.agent_program = Some(program.display().to_string());
    }

    #[cfg(test)]
    pub fn keep_images_in(&mut self, dir: PathBuf) {
        self.image_dir = dir;
    }

    #[cfg(test)]
    pub fn run(&mut self, program: &str, args: &[&str]) {
        self.shell = Some(Shell::new(
            program.to_string(),
            args.iter().map(|arg| arg.to_string()).collect(),
        ));
    }

    #[cfg(test)]
    pub fn is_open(&self, id: u64) -> bool {
        self.terminals.contains_key(&id)
    }

    pub fn screen(&self, id: u64) -> Option<String> {
        let terminal = self.terminals.get(&id)?.as_ref().ok()?;
        Some(screen_text(&terminal.term.lock()))
    }
}

#[cfg(test)]
mod tests;
