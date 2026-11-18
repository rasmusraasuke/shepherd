use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use alacritty_terminal::event::{Event as TermEvent, EventListener, Notify, OnResize, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, Msg, Notifier};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Osc52, Term, TermMode};
use alacritty_terminal::tty::{self, Shell};
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor, Rgb};
use eframe::egui::{self, Color32, Event, EventFilter, Key, Modifiers, Sense, Vec2};

use crate::agents::{self, Agent, Start};
use crate::code::CodeStyle;
use crate::images;
use crate::terminal_text;

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
        model: Option<&str>,
        message: &str,
    ) -> Result<(), String> {
        let command = agent.background_command(message, self.guide.as_deref(), model);
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
        model: Option<&str>,
    ) -> Option<String> {
        self.show(ui, id, Some((agent, session, model)))
    }

    #[allow(clippy::too_many_arguments)]
    fn spawn(
        &self,
        id: u64,
        agent: Option<(Agent, &str, Option<&str>)>,
        start: Start,
        size: Size,
        style: &CodeStyle,
        ctx: &egui::Context,
        focus: bool,
    ) -> Result<Terminal, String> {
        let mut env = self.env.clone();
        let resumed = agent.is_some() && matches!(start, Start::Continue);
        let shell = match agent {
            Some((agent, session, model)) => {
                let command = agent.command(session, start, self.guide.as_deref(), model);
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
        agent: Option<(Agent, &str, Option<&str>)>,
    ) -> Option<String> {
        let size = egui::TextStyle::Monospace.resolve(ui.style()).size;
        let fonts_ready = terminal_text::fonts_ready(ui.ctx(), self.system_fonts);
        let style = CodeStyle::with_font(
            ui,
            egui::FontId::new(size, terminal_text::family(fonts_ready, false, false)),
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
                    let target = agent.map(|(agent, _, model)| (agent, session.as_str(), model));
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

struct Terminal {
    term: Arc<FairMutex<Term<Listener>>>,
    notifier: Notifier,
    events: Receiver<TermEvent>,
    worker: Option<Worker>,
    child: u32,
    title: Option<String>,
    output: Arc<Mutex<Output>>,
    activity: Activity,
    last_input: Option<Instant>,
    worked: bool,
    size: Size,
    focus: bool,
    scroll: f32,
    clipboard_request: Option<ClipboardAnswer>,
    clipboard_allowed: bool,
    mouse_button: Option<u8>,
    mouse_cell: Option<(usize, usize)>,
    restart: bool,
    agent: Option<Agent>,
    resumed: bool,
    images: Vec<PathBuf>,
    notice: Option<String>,
    _slave: Option<std::fs::File>,
}

fn window_size(size: Size, style: &CodeStyle) -> WindowSize {
    WindowSize {
        num_lines: size.lines as u16,
        num_cols: size.columns as u16,
        cell_width: style.digit_width as u16,
        cell_height: style.row_height as u16,
    }
}

fn keep_slave(pty: &tty::Pty) -> Option<std::fs::File> {
    use std::os::fd::AsRawFd as _;
    use std::os::unix::fs::OpenOptionsExt as _;

    static LOCK: Mutex<()> = Mutex::new(());
    let _guard = LOCK.lock().ok()?;
    let name = unsafe { libc::ptsname(pty.file().as_raw_fd()) };
    if name.is_null() {
        return None;
    }
    let name = unsafe { std::ffi::CStr::from_ptr(name) }.to_str().ok()?;
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NOCTTY)
        .open(name)
        .ok()
}

impl Terminal {
    fn spawn(
        root: &std::path::Path,
        shell: Option<Shell>,
        mut env: HashMap<String, String>,
        size: Size,
        style: &CodeStyle,
        ctx: &egui::Context,
        focus: bool,
    ) -> Result<Self, String> {
        env.extend([
            ("TERM".to_string(), "xterm-256color".to_string()),
            ("COLORTERM".to_string(), "truecolor".to_string()),
        ]);
        let options = tty::Options {
            shell,
            working_directory: Some(root.to_path_buf()),
            drain_on_exit: true,
            env,
        };
        let pty = tty::new(&options, window_size(size, style), 0)
            .map_err(|e| format!("Cannot start the terminal: {e}"))?;
        let slave = keep_slave(&pty);
        let child = pty.child().id();
        let (sender, events) = mpsc::channel();
        let output = Arc::new(Mutex::new(Output::default()));
        let listener = Listener {
            events: sender,
            ctx: ctx.clone(),
            output: output.clone(),
        };
        let config = Config {
            osc52: Osc52::CopyPaste,
            ..Default::default()
        };
        let term = Arc::new(FairMutex::new(Term::new(config, &size, listener.clone())));
        let event_loop = EventLoop::new(term.clone(), listener, pty, true, false)
            .map_err(|e| format!("Cannot start the terminal: {e}"))?;
        let notifier = Notifier(event_loop.channel());
        let worker = event_loop.spawn();
        Ok(Self {
            term,
            notifier,
            events,
            worker: Some(worker),
            child,
            title: None,
            output,
            activity: Activity::Idle,
            last_input: None,
            worked: false,
            size,
            focus,
            scroll: 0.0,
            clipboard_request: None,
            clipboard_allowed: false,
            mouse_button: None,
            mouse_cell: None,
            restart: false,
            agent: None,
            resumed: false,
            images: Vec::new(),
            notice: None,
            _slave: slave,
        })
    }

    fn write(&self, bytes: impl Into<Vec<u8>>) {
        self.notifier.notify(bytes.into());
    }

    fn update_activity(&mut self) {
        let Ok(output) = self.output.lock() else {
            return;
        };
        let working = output.last.is_some_and(|last| {
            last.elapsed() < BUSY
                && self
                    .last_input
                    .is_none_or(|input| last.saturating_duration_since(input) > ECHO)
        });
        self.worked |= working;
        self.activity = match () {
            _ if output.ended => Activity::Ended,
            _ if working => Activity::Working,
            _ if self.worked => Activity::Waiting,
            _ => Activity::Idle,
        };
    }

    fn resume_failed(&self) -> bool {
        self.resumed
            && self
                .output
                .lock()
                .is_ok_and(|output| output.ended && output.failed)
    }

    fn handle_reports(
        &mut self,
        ctx: &egui::Context,
        style: &CodeStyle,
        palette: &Palette,
        read_clipboard: fn() -> Option<String>,
    ) {
        for event in self.events.try_iter().collect::<Vec<_>>() {
            match event {
                TermEvent::Title(title) => self.title = Some(title),
                TermEvent::ResetTitle => self.title = None,
                TermEvent::PtyWrite(text) => self.write(text),
                TermEvent::ClipboardStore(_, text) => ctx.copy_text(text),
                TermEvent::ClipboardLoad(_, answer) => self.clipboard_request = Some(answer),
                TermEvent::ColorRequest(index, answer) => {
                    let color = palette.color(
                        &self.term.lock(),
                        Color::Indexed(index.min(255) as u8),
                        index,
                    );
                    self.write(answer(Rgb {
                        r: color.r(),
                        g: color.g(),
                        b: color.b(),
                    }));
                }
                TermEvent::TextAreaSizeRequest(answer) => {
                    self.write(answer(window_size(self.size, style)))
                }
                _ => {}
            }
        }
        if self.clipboard_allowed
            && let Some(answer) = self.clipboard_request.take()
        {
            self.write(answer(&read_clipboard().unwrap_or_default()));
        }
    }

    fn clipboard_bar(&mut self, ui: &mut egui::Ui) {
        if self.clipboard_request.is_none() {
            return;
        }
        ui.horizontal(|ui| {
            ui.label("A program in this terminal wants to read the clipboard.");
            if ui
                .button("Allow")
                .on_hover_text("Allow it for this terminal until its program ends")
                .clicked()
            {
                self.clipboard_allowed = true;
            }
            if ui.button("Deny").clicked() {
                self.clipboard_request = None;
            }
        });
    }

    fn notice_bar(&mut self, ui: &mut egui::Ui) {
        let Some(notice) = self.notice.clone() else {
            return;
        };
        ui.horizontal(|ui| {
            ui.colored_label(ui.visuals().warn_fg_color, notice);
            if ui.small_button("Dismiss").clicked() {
                self.notice = None;
            }
        });
    }

    fn image_text(
        &mut self,
        ui: &egui::Ui,
        rect: egui::Rect,
        focused: bool,
        image_dir: &Path,
    ) -> Option<String> {
        let agent = self.agent?;
        let mut paths = dropped_images(ui, rect);
        if focused {
            for picture in images::take_pasted(ui.ctx()) {
                match images::save(image_dir, &picture) {
                    Ok(path) => {
                        self.images.push(path.clone());
                        paths.push(path);
                    }
                    Err(error) => self.notice = Some(error),
                }
            }
        }
        if paths.is_empty() {
            return None;
        }
        Some(images::hand(Some(agent), &paths))
    }

    fn resize(&mut self, size: Size, style: &CodeStyle) {
        if size != self.size {
            self.size = size;
            self.term.lock().resize(size);
            self.notifier.on_resize(window_size(size, style));
        }
    }

    fn ui(
        &mut self,
        ui: &mut egui::Ui,
        id: u64,
        style: &CodeStyle,
        fonts_ready: bool,
        read_clipboard: fn() -> Option<String>,
        image_dir: &Path,
    ) -> bool {
        let palette = Palette::new(ui.visuals());
        self.update_activity();
        self.notice_bar(ui);
        self.clipboard_bar(ui);
        self.handle_reports(ui.ctx(), style, &palette, read_clipboard);
        let size = size_for(ui.available_rect_before_wrap(), style);
        self.resize(size, style);

        let (rect, _) = ui.allocate_exact_size(ui.available_size(), Sense::hover());
        let response = ui.interact(
            rect,
            egui::Id::unique(("terminal", id)),
            Sense::click_and_drag(),
        );
        let origin = rect.min + Vec2::splat(PADDING);
        if std::mem::take(&mut self.focus) || response.clicked() {
            response.request_focus();
        }

        if let Some(text) = self.image_text(ui, rect, response.has_focus(), image_dir) {
            let mut term = self.term.lock();
            let mode = *term.mode();
            term.scroll_display(Scroll::Bottom);
            term.selection = None;
            drop(term);
            self.last_input = Some(Instant::now());
            self.write(paste_bytes(&text, mode));
        }

        let mut term = self.term.lock();
        response.widget_info(|| {
            let label = self.agent.map_or("Terminal", Agent::name);
            let mut info = egui::WidgetInfo::labeled(egui::Role::Terminal, true, label);
            info.current_text_value = Some(screen_text(&term));
            info
        });

        let screen_cell = |pos: egui::Pos2| {
            let x = ((pos.x - origin.x) / style.digit_width).max(0.0);
            let row =
                (((pos.y - origin.y) / style.row_height).max(0.0) as usize).min(size.lines - 1);
            (
                (x as usize).min(size.columns - 1),
                row,
                if x.fract() < 0.5 {
                    Side::Left
                } else {
                    Side::Right
                },
            )
        };
        let mode = *term.mode();
        let mouse_reports =
            mode.intersects(TermMode::MOUSE_MODE) && !ui.input(|i| i.modifiers.shift);
        if mouse_reports {
            let mut input = Vec::new();
            for event in ui.input(|i| i.events.clone()) {
                match event {
                    Event::PointerButton {
                        pos,
                        button,
                        pressed,
                        modifiers,
                    } if rect.contains(pos) || (!pressed && self.mouse_button.is_some()) => {
                        let button = match button {
                            egui::PointerButton::Primary => 0,
                            egui::PointerButton::Middle => 1,
                            egui::PointerButton::Secondary => 2,
                            _ => continue,
                        };
                        let (column, row, _) = screen_cell(pos);
                        self.mouse_button = pressed.then_some(button);
                        self.mouse_cell = Some((column, row));
                        input.extend(mouse_bytes(button, modifiers, (column, row), pressed, mode));
                    }
                    Event::PointerMoved(pos)
                        if rect.contains(pos) || self.mouse_button.is_some() =>
                    {
                        let (column, row, _) = screen_cell(pos);
                        let moved = self.mouse_cell != Some((column, row));
                        let wanted = mode.contains(TermMode::MOUSE_MOTION)
                            || (mode.contains(TermMode::MOUSE_DRAG) && self.mouse_button.is_some());
                        if moved && wanted {
                            self.mouse_cell = Some((column, row));
                            let modifiers = ui.input(|i| i.modifiers);
                            input.extend(mouse_bytes(
                                self.mouse_button.unwrap_or(3) + 32,
                                modifiers,
                                (column, row),
                                true,
                                mode,
                            ));
                        }
                    }
                    _ => {}
                }
            }
            if !input.is_empty() {
                drop(term);
                self.write(input);
                term = self.term.lock();
            }
        } else if let Some(pointer) = response.interact_pointer_pos() {
            let (column, row, side) = screen_cell(pointer);
            let point = Point::new(
                Line(row as i32 - term.grid().display_offset() as i32),
                Column(column),
            );
            if ui.input(|i| i.pointer.primary_pressed()) {
                let kind = if response.triple_clicked() {
                    SelectionType::Lines
                } else if response.double_clicked() {
                    SelectionType::Semantic
                } else {
                    SelectionType::Simple
                };
                term.selection = Some(Selection::new(kind, point, side));
            } else if response.dragged()
                && let Some(selection) = &mut term.selection
            {
                selection.update(point, side);
            }
        }
        if response.hovered() {
            let delta = ui.input(|i| i.smooth_scroll_delta.y);
            self.scroll += delta / style.row_height;
            let lines = self.scroll.trunc() as i32;
            self.scroll -= lines as f32;
            if lines != 0 {
                let bytes = if mouse_reports {
                    let (column, row, _) =
                        screen_cell(ui.input(|i| i.pointer.hover_pos()).unwrap_or(origin));
                    let modifiers = ui.input(|i| i.modifiers);
                    Some(mouse_bytes(
                        if lines > 0 { 64 } else { 65 },
                        modifiers,
                        (column, row),
                        true,
                        mode,
                    ))
                } else if mode.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL) {
                    key_bytes(
                        if lines > 0 {
                            Key::ArrowUp
                        } else {
                            Key::ArrowDown
                        },
                        Modifiers::NONE,
                        mode,
                    )
                } else {
                    term.scroll_display(Scroll::Delta(lines));
                    None
                };
                if let Some(bytes) = bytes {
                    drop(term);
                    self.write(bytes.repeat(lines.unsigned_abs() as usize));
                    term = self.term.lock();
                }
            }
        }

        if response.has_focus() {
            let filter = EventFilter {
                tab: true,
                horizontal_arrows: true,
                vertical_arrows: true,
                escape: true,
            };
            ui.memory_mut(|m| m.set_focus_lock_filter(response.id, filter));
            let mode = *term.mode();
            let mut input = Vec::new();
            for event in ui.input(|i| i.events.clone()) {
                match event {
                    Event::Text(text) => input.extend(text.into_bytes()),
                    Event::Paste(text) => input.extend(paste_bytes(&text, mode)),
                    Event::Copy => {
                        if let Some(text) = term.selection_to_string() {
                            ui.ctx().copy_text(text);
                        }
                    }
                    Event::Key {
                        key: Key::Enter,
                        pressed: true,
                        ..
                    } if self.activity == Activity::Ended => self.restart = true,
                    Event::Key {
                        key,
                        pressed: true,
                        modifiers,
                        ..
                    } => {
                        if key == Key::K && modifiers.command {
                            term.grid_mut().clear_history();
                        } else if let Some(bytes) = key_bytes(key, modifiers, mode) {
                            input.extend(bytes);
                        }
                    }
                    _ => {}
                }
            }
            self.worked = false;
            if !input.is_empty() {
                self.last_input = Some(Instant::now());
                term.scroll_display(Scroll::Bottom);
                term.selection = None;
                drop(term);
                self.write(input);
                term = self.term.lock();
            }
        }

        paint(
            ui,
            &term,
            rect,
            origin,
            style,
            &palette,
            response.has_focus(),
            fonts_ready,
        );
        drop(term);
        if self.activity == Activity::Ended {
            let bottom_right = egui::pos2(rect.right() - PADDING, rect.bottom() - PADDING);
            let place = egui::Rect::from_min_max(
                bottom_right - Vec2::new(320.0, 2.0 * style.row_height),
                bottom_right,
            );
            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(place)
                    .layout(egui::Layout::right_to_left(egui::Align::Max)),
                |ui| {
                    let (ended, again) = match self.agent {
                        Some(agent) => (format!("{} has ended", agent.name()), "Start again"),
                        None => ("The process has ended".to_string(), "Restart"),
                    };
                    self.restart |= ui.button(again).on_hover_text("Enter").clicked();
                    ui.weak(ended);
                },
            );
        }
        response.has_focus()
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        self.stop();
        images::remove(&self.images);
    }
}

impl Terminal {
    fn stop(&mut self) {
        if self.worker.take().is_none() {
            return;
        }
        let _ = self.notifier.0.send(Msg::Shutdown);
        unsafe {
            libc::kill(-(self.child as libc::pid_t), libc::SIGHUP);
        }
    }

    fn kill(&self) {
        unsafe {
            libc::kill(-(self.child as libc::pid_t), libc::SIGKILL);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn paint(
    ui: &egui::Ui,
    term: &Term<Listener>,
    rect: egui::Rect,
    origin: egui::Pos2,
    style: &CodeStyle,
    palette: &Palette,
    focused: bool,
    fonts_ready: bool,
) {
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    let background = palette.color(
        term,
        Color::Named(NamedColor::Background),
        NamedColor::Background as usize,
    );
    painter.rect_filled(rect, 0.0, background);

    let content = term.renderable_content();
    let offset = content.display_offset as i32;
    let cell_rect = |point: Point, width: usize| {
        let top_left = origin
            + Vec2::new(
                point.column.0 as f32 * style.digit_width,
                (point.line.0 + offset) as f32 * style.row_height,
            );
        egui::Rect::from_min_size(
            top_left,
            Vec2::new(style.digit_width * width as f32, style.row_height),
        )
    };
    let cursor = content.cursor;
    let show_cursor =
        cursor.shape != CursorShape::Hidden && term.mode().contains(TermMode::SHOW_CURSOR);

    for cell in content.display_iter {
        if cell.flags.contains(Flags::WIDE_CHAR_SPACER)
            || cell.flags.contains(Flags::LEADING_WIDE_CHAR_SPACER)
        {
            continue;
        }
        let width = if cell.flags.contains(Flags::WIDE_CHAR) {
            2
        } else {
            1
        };
        let rect = cell_rect(cell.point, width);
        let mut fg = palette.cell_color(term, cell.fg, cell.flags, true);
        let mut bg = palette.cell_color(term, cell.bg, cell.flags, false);
        if cell.flags.contains(Flags::INVERSE) {
            std::mem::swap(&mut fg, &mut bg);
        }
        if content
            .selection
            .is_some_and(|selection| selection.contains_cell(&cell, cursor.point, cursor.shape))
        {
            bg = ui.visuals().selection.bg_fill;
        }
        let under_cursor = show_cursor
            && focused
            && cell.point == cursor.point
            && cursor.shape == CursorShape::Block;
        if under_cursor {
            (fg, bg) = (
                background,
                palette.color(
                    term,
                    Color::Named(NamedColor::Cursor),
                    NamedColor::Cursor as usize,
                ),
            );
        }
        if bg != background {
            painter.rect_filled(rect, 0.0, bg);
        }
        let pixel = 1.0 / ui.ctx().pixels_per_point();
        if cell.c != ' '
            && !cell.flags.contains(Flags::HIDDEN)
            && !terminal_text::draw(&painter, rect, cell.c, fg, pixel)
        {
            let family = terminal_text::family(
                fonts_ready,
                cell.flags.contains(Flags::BOLD),
                cell.flags.contains(Flags::ITALIC),
            );
            let format = egui::TextFormat {
                font_id: egui::FontId::new(style.font.size, family),
                color: fg,
                underline: if cell.flags.intersects(Flags::ALL_UNDERLINES) {
                    egui::Stroke::new(1.0, fg)
                } else {
                    egui::Stroke::NONE
                },
                strikethrough: if cell.flags.contains(Flags::STRIKEOUT) {
                    egui::Stroke::new(1.0, fg)
                } else {
                    egui::Stroke::NONE
                },
                ..Default::default()
            };
            let mut text = cell.c.to_string();
            text.extend(cell.zerowidth().into_iter().flatten());
            let galley = ui.fonts_mut(|fonts| {
                fonts.layout_job(egui::text::LayoutJob::single_section(text, format))
            });
            painter.galley(rect.min + Vec2::new(0.0, 1.0), galley, fg);
        }
    }

    if show_cursor && (!focused || cursor.shape != CursorShape::Block) {
        let rect = cell_rect(cursor.point, 1);
        let color = palette.color(
            term,
            Color::Named(NamedColor::Cursor),
            NamedColor::Cursor as usize,
        );
        match cursor.shape {
            CursorShape::Beam => painter.rect_filled(
                egui::Rect::from_min_size(rect.min, Vec2::new(2.0, rect.height())),
                0.0,
                color,
            ),
            CursorShape::Underline => painter.rect_filled(
                egui::Rect::from_min_max(egui::pos2(rect.left(), rect.bottom() - 2.0), rect.max),
                0.0,
                color,
            ),
            _ => painter.rect_stroke(
                rect,
                0.0,
                egui::Stroke::new(1.0, color),
                egui::StrokeKind::Inside,
            ),
        };
    }
}

fn screen_text(term: &Term<Listener>) -> String {
    let grid = term.grid();
    let mut lines = Vec::new();
    for line in 0..grid.screen_lines() {
        let row = &grid[Line(line as i32 - grid.display_offset() as i32)];
        let text: String = (0..grid.columns())
            .map(|column| row[Column(column)].c)
            .collect();
        lines.push(text.trim_end().to_string());
    }
    lines.join("\n").trim_end().to_string()
}

struct Palette {
    dark: bool,
    foreground: Color32,
    background: Color32,
}

const DARK_COLORS: [u32; 16] = [
    0x1d1f21, 0xcc6666, 0xb5bd68, 0xf0c674, 0x81a2be, 0xb294bb, 0x8abeb7, 0xc5c8c6, 0x666666,
    0xd54e53, 0xb9ca4a, 0xe7c547, 0x7aa6da, 0xc397d8, 0x70c0b1, 0xeaeaea,
];
const LIGHT_COLORS: [u32; 16] = [
    0x000000, 0xc91b00, 0x00a600, 0xa08000, 0x0225c7, 0xc930c7, 0x00a5b2, 0xbfbfbf, 0x686868,
    0xff6e67, 0x2fb500, 0xb59c00, 0x6871ff, 0xff77ff, 0x00b8c2, 0xffffff,
];

fn rgb(color: u32) -> Color32 {
    Color32::from_rgb((color >> 16) as u8, (color >> 8) as u8, color as u8)
}

impl Palette {
    fn new(visuals: &egui::Visuals) -> Self {
        Self {
            dark: visuals.dark_mode,
            foreground: visuals.text_color(),
            background: visuals.panel_fill,
        }
    }

    fn standard(&self, index: usize) -> Color32 {
        let named = if self.dark { DARK_COLORS } else { LIGHT_COLORS };
        match index {
            0..16 => rgb(named[index]),
            16..232 => {
                let level = |value: usize| {
                    if value == 0 {
                        0
                    } else {
                        (55 + value * 40) as u8
                    }
                };
                let index = index - 16;
                Color32::from_rgb(level(index / 36), level(index / 6 % 6), level(index % 6))
            }
            232..256 => Color32::from_gray((8 + (index - 232) * 10) as u8),
            _ if index == NamedColor::Background as usize => self.background,
            _ if index == NamedColor::Cursor as usize => self.foreground,
            _ if index == NamedColor::BrightForeground as usize => self.foreground,
            _ if index == NamedColor::DimForeground as usize => {
                self.foreground.gamma_multiply(0.66)
            }
            _ if (NamedColor::DimBlack as usize..=NamedColor::DimWhite as usize)
                .contains(&index) =>
            {
                let base = self.standard(index - NamedColor::DimBlack as usize);
                Color32::from_rgb(
                    (base.r() as f32 * 0.66) as u8,
                    (base.g() as f32 * 0.66) as u8,
                    (base.b() as f32 * 0.66) as u8,
                )
            }
            _ => self.foreground,
        }
    }

    fn color(&self, term: &Term<Listener>, color: Color, index: usize) -> Color32 {
        match color {
            Color::Spec(Rgb { r, g, b }) => Color32::from_rgb(r, g, b),
            _ => term.colors()[index].map_or_else(
                || self.standard(index),
                |Rgb { r, g, b }| Color32::from_rgb(r, g, b),
            ),
        }
    }

    fn cell_color(
        &self,
        term: &Term<Listener>,
        color: Color,
        flags: Flags,
        foreground: bool,
    ) -> Color32 {
        let index = match color {
            Color::Named(named) => named as usize,
            Color::Indexed(index) => index as usize,
            Color::Spec(_) => 0,
        };
        let index = if foreground
            && flags.contains(Flags::BOLD)
            && index < 8
            && !matches!(color, Color::Spec(_))
        {
            index + 8
        } else {
            index
        };
        let color = self.color(term, color, index);
        if foreground && flags.contains(Flags::DIM) {
            color.gamma_multiply(0.66)
        } else {
            color
        }
    }
}

fn mouse_bytes(
    button: u8,
    modifiers: Modifiers,
    (column, row): (usize, usize),
    pressed: bool,
    mode: TermMode,
) -> Vec<u8> {
    let modifiers = 4 * modifiers.shift as u8 + 8 * modifiers.alt as u8 + 16 * modifiers.ctrl as u8;
    if mode.contains(TermMode::SGR_MOUSE) {
        let end = if pressed { 'M' } else { 'm' };
        return format!(
            "\x1b[<{};{};{}{end}",
            button + modifiers,
            column + 1,
            row + 1
        )
        .into_bytes();
    }
    let button = if pressed { button } else { 3 };
    if column > 222 || row > 222 {
        return Vec::new();
    }
    vec![
        0x1b,
        b'[',
        b'M',
        32 + button + modifiers,
        33 + column as u8,
        33 + row as u8,
    ]
}

fn dropped_images(ui: &egui::Ui, rect: egui::Rect) -> Vec<PathBuf> {
    let over = ui
        .input(|i| i.pointer.hover_pos())
        .is_some_and(|pos| rect.contains(pos) && ui.ctx().layer_id_at(pos) == Some(ui.layer_id()));
    if !over {
        return Vec::new();
    }
    ui.input(|input| {
        input
            .raw
            .dropped_files
            .iter()
            .map(|file| file.path().to_path_buf())
            .filter(|path| images::is_image(path))
            .collect()
    })
}

fn paste_bytes(text: &str, mode: TermMode) -> Vec<u8> {
    let text = text.replace("\r\n", "\r").replace('\n', "\r");
    if mode.contains(TermMode::BRACKETED_PASTE) {
        format!("\x1b[200~{}\x1b[201~", text.replace("\x1b[201~", "")).into_bytes()
    } else {
        text.into_bytes()
    }
}

fn key_bytes(key: Key, modifiers: Modifiers, mode: TermMode) -> Option<Vec<u8>> {
    let mac = cfg!(target_os = "macos");
    let code = 1 + modifiers.shift as u8 + 2 * modifiers.alt as u8 + 4 * modifiers.ctrl as u8;
    let csi = |end: &str| {
        if code > 1 {
            format!("\x1b[1;{code}{end}")
        } else {
            format!("\x1b[{end}")
        }
    };
    let cursor = |end: &str| {
        if code == 1 && mode.contains(TermMode::APP_CURSOR) {
            format!("\x1bO{end}")
        } else {
            csi(end)
        }
    };
    let tilde = |number: u8| {
        if code > 1 {
            format!("\x1b[{number};{code}~")
        } else {
            format!("\x1b[{number}~")
        }
    };
    let only_alt = modifiers.alt && !modifiers.ctrl && !modifiers.shift && !modifiers.command;
    let text = match key {
        Key::ArrowLeft if mac && modifiers.mac_cmd => "\x01".to_string(),
        Key::ArrowRight if mac && modifiers.mac_cmd => "\x05".to_string(),
        Key::Backspace if mac && modifiers.mac_cmd => "\x15".to_string(),
        _ if modifiers.command && !modifiers.ctrl => return None,
        Key::ArrowLeft if only_alt => "\x1bb".to_string(),
        Key::ArrowRight if only_alt => "\x1bf".to_string(),
        Key::ArrowUp => cursor("A"),
        Key::ArrowDown => cursor("B"),
        Key::ArrowRight => cursor("C"),
        Key::ArrowLeft => cursor("D"),
        Key::Home => cursor("H"),
        Key::End => cursor("F"),
        Key::Insert => tilde(2),
        Key::Delete => tilde(3),
        Key::PageUp => tilde(5),
        Key::PageDown => tilde(6),
        Key::F1 => "\x1bOP".to_string(),
        Key::F2 => "\x1bOQ".to_string(),
        Key::F3 => "\x1bOR".to_string(),
        Key::F4 => "\x1bOS".to_string(),
        Key::F5 => tilde(15),
        Key::F6 => tilde(17),
        Key::F7 => tilde(18),
        Key::F8 => tilde(19),
        Key::F9 => tilde(20),
        Key::F10 => tilde(21),
        Key::F11 => tilde(23),
        Key::F12 => tilde(24),
        Key::Enter if modifiers.shift || modifiers.alt => "\x1b\r".to_string(),
        Key::Enter => "\r".to_string(),
        Key::Tab if modifiers.shift => "\x1b[Z".to_string(),
        Key::Tab => "\t".to_string(),
        Key::Backspace if modifiers.ctrl => "\x08".to_string(),
        Key::Backspace if modifiers.alt => "\x1b\x7f".to_string(),
        Key::Backspace => "\x7f".to_string(),
        Key::Escape => "\x1b".to_string(),
        Key::Space if modifiers.ctrl => "\0".to_string(),
        _ if modifiers.ctrl => {
            let letter = key
                .name()
                .chars()
                .next()
                .filter(|c| key.name().len() == 1 && c.is_ascii_alphabetic())?;
            let control = (letter.to_ascii_uppercase() as u8) & 0x1f;
            if modifiers.alt {
                format!("\x1b{}", control as char)
            } else {
                (control as char).to_string()
            }
        }
        _ => return None,
    };
    Some(text.into_bytes())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable as _;

    use super::*;
    use crate::testing::{Dropped, TempDir};

    fn sent(key: Key, modifiers: Modifiers) -> String {
        String::from_utf8(key_bytes(key, modifiers, TermMode::default()).unwrap_or_default())
            .unwrap()
    }

    #[test]
    fn keys_send_what_xterm_sends() {
        assert_eq!(sent(Key::Enter, Modifiers::NONE), "\r");
        assert_eq!(sent(Key::Enter, Modifiers::SHIFT), "\x1b\r");
        assert_eq!(sent(Key::Backspace, Modifiers::NONE), "\x7f");
        assert_eq!(sent(Key::Backspace, Modifiers::ALT), "\x1b\x7f");
        assert_eq!(sent(Key::Tab, Modifiers::SHIFT), "\x1b[Z");
        assert_eq!(sent(Key::Escape, Modifiers::NONE), "\x1b");
        assert_eq!(sent(Key::ArrowUp, Modifiers::NONE), "\x1b[A");
        assert_eq!(sent(Key::ArrowLeft, Modifiers::CTRL), "\x1b[1;5D");
        assert_eq!(sent(Key::ArrowRight, Modifiers::SHIFT), "\x1b[1;2C");
        assert_eq!(sent(Key::ArrowLeft, Modifiers::ALT), "\x1bb");
        assert_eq!(sent(Key::Delete, Modifiers::NONE), "\x1b[3~");
        assert_eq!(sent(Key::F5, Modifiers::NONE), "\x1b[15~");
        assert_eq!(sent(Key::C, Modifiers::CTRL), "\x03");
        assert_eq!(sent(Key::D, Modifiers::CTRL), "\x04");
        assert_eq!(sent(Key::A, Modifiers::NONE), "");
        assert_eq!(sent(Key::P, Modifiers::COMMAND), "");
    }

    #[test]
    fn keys_follow_the_modes_the_program_asked_for() {
        assert_eq!(
            key_bytes(Key::ArrowUp, Modifiers::NONE, TermMode::APP_CURSOR).unwrap(),
            b"\x1bOA"
        );
    }

    #[test]
    fn mouse_buttons_send_what_xterm_sends() {
        let sgr = TermMode::SGR_MOUSE;
        assert_eq!(
            mouse_bytes(0, Modifiers::NONE, (0, 0), true, sgr),
            b"\x1b[<0;1;1M"
        );
        assert_eq!(
            mouse_bytes(2, Modifiers::CTRL, (9, 4), false, sgr),
            b"\x1b[<18;10;5m"
        );
        assert_eq!(
            mouse_bytes(65, Modifiers::NONE, (1, 2), true, sgr),
            b"\x1b[<65;2;3M"
        );
        assert_eq!(
            mouse_bytes(32, Modifiers::SHIFT, (3, 3), true, TermMode::default()),
            [0x1b, b'[', b'M', 32 + 36, 36, 36]
        );
        assert_eq!(
            mouse_bytes(0, Modifiers::NONE, (3, 3), false, TermMode::default()),
            [0x1b, b'[', b'M', 35, 36, 36]
        );
        assert!(mouse_bytes(0, Modifiers::NONE, (300, 3), true, TermMode::default()).is_empty());
    }

    #[test]
    fn pastes_line_breaks_as_enter_and_marks_the_paste_when_asked() {
        assert_eq!(paste_bytes("a\nb\r\nc", TermMode::default()), b"a\rb\rc");
        assert_eq!(
            paste_bytes("ls\x1b[201~rm", TermMode::BRACKETED_PASTE),
            b"\x1b[200~lsrm\x1b[201~"
        );
    }

    #[test]
    fn has_the_standard_256_colors() {
        let palette = Palette {
            dark: true,
            foreground: Color32::WHITE,
            background: Color32::BLACK,
        };
        assert_eq!(palette.standard(1), rgb(DARK_COLORS[1]));
        assert_eq!(palette.standard(16), Color32::from_rgb(0, 0, 0));
        assert_eq!(palette.standard(21), Color32::from_rgb(0, 0, 255));
        assert_eq!(palette.standard(196), Color32::from_rgb(255, 0, 0));
        assert_eq!(palette.standard(232), Color32::from_gray(8));
        assert_eq!(
            palette.standard(NamedColor::Background as usize),
            Color32::BLACK
        );
    }

    fn harness(name: &str, script: &str) -> (Harness<'static, Terminals>, TempDir) {
        let dir = TempDir::new(name);
        let mut terminals = Terminals::new(dir.path().to_path_buf());
        terminals.image_dir = dir.path().join("images");
        terminals.run("/bin/sh", &["-c", script]);
        let harness = Harness::builder()
            .with_size(Vec2::new(600.0, 400.0))
            .with_max_steps(8)
            .build_ui_state(
                |ui, terminals: &mut Terminals| terminals.ui(ui, 1),
                terminals,
            );
        (harness, dir)
    }

    fn agent_harness(
        name: &str,
        script: &str,
        fresh: bool,
    ) -> (Harness<'static, Terminals>, TempDir) {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = TempDir::new(name);
        let program = dir.write("agent", script);
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut terminals = Terminals::new(dir.path().to_path_buf());
        terminals.image_dir = dir.path().join("images");
        terminals.run_agents_with(&program);
        if fresh {
            terminals.start(1, Prompt::default());
        }
        let harness = Harness::builder()
            .with_size(Vec2::new(600.0, 400.0))
            .with_max_steps(8)
            .build_ui_state(
                |ui, terminals: &mut Terminals| {
                    terminals.agent_ui(ui, 1, Agent::Claude, "s1", None);
                },
                terminals,
            );
        (harness, dir)
    }

    fn wait(
        harness: &mut Harness<'_, Terminals>,
        what: &str,
        done: impl Fn(&Harness<'_, Terminals>) -> bool,
    ) {
        let start = Instant::now();
        while !done(harness) {
            assert!(
                start.elapsed() < Duration::from_secs(5),
                "no {what} on the screen:\n{}",
                harness.state().screen(1).unwrap()
            );
            harness.run();
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn wait_for_screen(harness: &mut Harness<'_, Terminals>, text: &str) {
        wait(harness, text, |harness| {
            harness.state().screen(1).unwrap().contains(text)
        });
    }

    const SHOW_INPUT: &str =
        "stty -icanon -echo; dd bs=1 count=COUNT 2>/dev/null | od -An -c; sleep 5";

    #[test]
    fn runs_a_program_in_the_project_and_shows_its_output() {
        let (mut harness, dir) = harness(
            "terminal",
            "pwd; printf '\\033]0;working\\007\\033[31mred\\033[0m'; read line; echo got $line",
        );
        wait_for_screen(&mut harness, "red");
        assert!(
            harness
                .state()
                .screen(1)
                .unwrap()
                .replace('\n', "")
                .starts_with(&dir.path().canonicalize().unwrap().display().to_string())
        );
        wait(&mut harness, "title", |harness| {
            harness.state().title(1).as_deref() == Some("working")
        });

        harness.state().terminals[&1]
            .as_ref()
            .unwrap()
            .write("hello\r");
        wait_for_screen(&mut harness, "got hello");
        wait(&mut harness, "end", |harness| {
            harness.state().title(1).as_deref() == Some("working (exited)")
        });
    }

    #[test]
    fn tells_when_the_program_works_and_when_it_stopped() {
        let (mut harness, _dir) = harness(
            "terminal-activity",
            "sleep 0.5; for i in 1 2 3 4 5 6; do echo $i; sleep 0.2; done; sleep 10",
        );
        wait(&mut harness, "work", |harness| {
            harness.state().activity(1) == Some(Activity::Working)
        });
        wait(&mut harness, "stop", |harness| {
            harness.state().activity(1) == Some(Activity::Waiting)
        });
    }

    #[test]
    fn sends_text_to_agent_tabs_as_if_typed_with_enter() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = TempDir::new("terminal-send");
        let script = dir.write("agent", "#!/bin/sh\nread line\necho hi $line\nsleep 5\n");
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut terminals = Terminals::new(dir.path().to_path_buf());
        terminals.run_agents_with(&script);
        let mut harness = Harness::builder()
            .with_size(Vec2::new(600.0, 400.0))
            .with_max_steps(8)
            .build_ui_state(
                |ui, terminals: &mut Terminals| {
                    terminals.agent_ui(ui, 1, Agent::Claude, "s1", None);
                },
                terminals,
            );
        harness.run();
        assert_eq!(harness.state().agents(), [(1, "Claude Code".to_string())]);
        harness.state_mut().send(1, "hello");
        wait_for_screen(&mut harness, "hi hello");
    }

    #[test]
    fn starts_an_agent_fresh_or_continues_its_session() {
        let script = "#!/bin/sh\necho \"args: $@\"\nsleep 5\n";
        let (mut fresh, _dir) = agent_harness("agent-new", script, true);
        fresh.run();
        wait_for_screen(&mut fresh, "args: --session-id s1");

        let (mut resumed, _dir) = agent_harness("agent-resume", script, false);
        resumed.run();
        wait_for_screen(&mut resumed, "args: --resume s1");
    }

    #[test]
    fn never_sends_text_into_a_plain_terminal() {
        let (mut harness, _dir) = harness("terminal-no-send", "sleep 5");
        harness.state_mut().send(1, "hello");
        harness.run_steps(4);
        assert_eq!(harness.state().screen(1).unwrap(), "");
    }

    #[test]
    fn shutdown_ends_the_program_of_every_terminal() {
        let (mut harness, _dir) = harness(
            "terminal-shutdown",
            "trap '' HUP; echo ready; while true; do sleep 0.1; done",
        );
        wait_for_screen(&mut harness, "ready");
        let pid = harness.state().terminals[&1].as_ref().unwrap().child as libc::pid_t;

        harness.state_mut().shutdown();

        assert!(!harness.state().is_open(1));
        let start = Instant::now();
        while unsafe { libc::kill(pid, 0) } != -1 {
            assert!(
                start.elapsed() < Duration::from_secs(5),
                "the program is still there"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn restarts_the_program_after_it_ended() {
        let (mut harness, _dir) = harness("terminal-restart", "echo started-$$");
        wait(&mut harness, "end", |harness| {
            harness
                .state()
                .title(1)
                .is_some_and(|title| title.ends_with("(exited)"))
        });
        let first = harness.state().screen(1).unwrap();
        harness.get_by_label("Restart").click();
        wait(&mut harness, "new start", |harness| {
            harness.state().screen(1).unwrap().starts_with("started-")
                && harness.state().screen(1).unwrap() != first
        });

        wait(&mut harness, "end", |harness| {
            harness
                .state()
                .title(1)
                .is_some_and(|title| title.ends_with("(exited)"))
        });
        let second = harness.state().screen(1).unwrap();
        harness.key_press(Key::Enter);
        wait(&mut harness, "new start", |harness| {
            harness.state().screen(1).unwrap().starts_with("started-")
                && harness.state().screen(1).unwrap() != second
        });
    }

    #[test]
    fn programs_read_the_clipboard_only_when_allowed() {
        let script = format!(
            "printf '\\033]52;c;?\\007'; {}",
            SHOW_INPUT.replace("COUNT", "12")
        );
        let (mut harness, _dir) = harness("terminal-clipboard", &script);
        harness.state_mut().read_clipboard = || Some("hi".to_string());
        wait(&mut harness, "question", |harness| {
            harness.query_by_label("Allow").is_some()
        });
        harness.get_by_label("Allow").click();
        wait_for_screen(&mut harness, "a   G   k   =");
        assert!(harness.query_by_label("Allow").is_none());

        let (mut denied, _dir) = self::harness("terminal-clipboard-denied", &script);
        wait(&mut denied, "question", |harness| {
            harness.query_by_label("Deny").is_some()
        });
        denied.get_by_label("Deny").click();
        denied.run();
        assert!(denied.query_by_label("Deny").is_none());
        assert_eq!(denied.state().screen(1).unwrap(), "");
    }

    #[test]
    fn mouse_clicks_go_to_programs_that_asked_for_them() {
        let script = format!(
            "printf '\\033[?1000h\\033[?1006h'; {}",
            SHOW_INPUT.replace("COUNT", "18")
        );
        let (mut harness, _dir) = harness("terminal-mouse", &script);
        let start = Instant::now();
        while !harness.state().terminals[&1]
            .as_ref()
            .unwrap()
            .term
            .lock()
            .mode()
            .contains(TermMode::SGR_MOUSE)
        {
            assert!(
                start.elapsed() < Duration::from_secs(5),
                "the program didn't ask for the mouse"
            );
            harness.run();
        }
        let cell =
            harness.get_by_role(egui::Role::Terminal).rect().min + Vec2::splat(PADDING + 2.0);
        harness.event(Event::PointerMoved(cell));
        for pressed in [true, false] {
            harness.event(Event::PointerButton {
                pos: cell,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            });
        }
        let sent = "033[<0;1;1M033[<0;1;1m";
        wait(&mut harness, sent, |harness| {
            harness
                .state()
                .screen(1)
                .unwrap()
                .split_whitespace()
                .collect::<String>()
                .contains(sent)
        });
    }

    fn picture() -> Arc<egui::ColorImage> {
        Arc::new(egui::ColorImage::new([1, 1], vec![egui::Color32::WHITE]))
    }

    fn image_harness(name: &str) -> (Harness<'static, Terminals>, TempDir, PathBuf) {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = TempDir::new(name);
        let capture = dir.path().join("input");
        let program = dir.write(
            "agent",
            format!(
                "#!/bin/sh\nstty -icanon -echo\nprintf ready\ncat > '{}'\n",
                capture.display()
            ),
        );
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut terminals = Terminals::new(dir.path().to_path_buf());
        terminals.image_dir = dir.path().join("images");
        terminals.run_agents_with(&program);
        terminals.start(1, Prompt::default());
        let harness = Harness::builder()
            .with_size(Vec2::new(600.0, 400.0))
            .with_max_steps(8)
            .build_ui_state(
                |ui, terminals: &mut Terminals| {
                    terminals.agent_ui(ui, 1, Agent::Claude, "s1", None);
                },
                terminals,
            );
        (harness, dir, capture)
    }

    fn click_terminal(harness: &mut Harness<'_, Terminals>) {
        let cell =
            harness.get_by_role(egui::Role::Terminal).rect().min + Vec2::splat(PADDING + 2.0);
        harness.event(Event::PointerMoved(cell));
        for pressed in [true, false] {
            harness.event(Event::PointerButton {
                pos: cell,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            });
        }
        harness.run();
    }

    fn wait_for_input(path: &Path, expected: &str) {
        let start = Instant::now();
        while !fs::read_to_string(path).is_ok_and(|text| text == expected) {
            assert!(
                start.elapsed() < Duration::from_secs(5),
                "the program didn't get {expected:?}, it has {:?}",
                fs::read_to_string(path).unwrap_or_default()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn image_files(dir: &Path) -> Vec<PathBuf> {
        fs::read_dir(dir)
            .map(|entries| entries.flatten().map(|entry| entry.path()).collect())
            .unwrap_or_default()
    }

    #[test]
    fn pastes_a_picture_into_an_agent_tab_as_the_path_of_a_file() {
        let (mut harness, dir, capture) = image_harness("agent-image-paste");
        wait_for_screen(&mut harness, "ready");
        click_terminal(&mut harness);
        harness.event(Event::PasteImage(picture()));
        harness.run();

        let images = image_files(&dir.path().join("images"));
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].extension().unwrap(), "png");
        wait_for_input(&capture, &images[0].display().to_string());
        drop(harness);
        assert!(!images[0].exists());
    }

    #[test]
    fn pastes_text_as_text_and_writes_no_picture() {
        let (mut harness, dir, capture) = image_harness("agent-image-text");
        wait_for_screen(&mut harness, "ready");
        click_terminal(&mut harness);
        harness.event(Event::Paste("hello".to_string()));
        harness.run();
        wait_for_input(&capture, "hello");
        assert!(image_files(&dir.path().join("images")).is_empty());
    }

    #[test]
    fn drops_pictures_on_an_agent_tab_as_their_paths() {
        let (mut harness, dir, capture) = image_harness("agent-image-drop");
        wait_for_screen(&mut harness, "ready");
        let picture = dir.write("shot.png", "picture");
        let notes = dir.write("notes.txt", "text");
        let cell = harness.get_by_role(egui::Role::Terminal).rect().center();
        harness.event(Event::PointerMoved(cell));
        harness.step();
        harness
            .input_mut()
            .dropped_files
            .push(Arc::new(Dropped(picture.clone())));
        harness
            .input_mut()
            .dropped_files
            .push(Arc::new(Dropped(notes)));
        harness.step();
        wait_for_input(&capture, &picture.display().to_string());
        assert!(image_files(&dir.path().join("images")).is_empty());
        drop(harness);
        assert!(picture.exists());
    }

    #[test]
    fn leaves_a_plain_terminal_alone_when_pictures_are_pasted_or_dropped() {
        let (mut harness, dir) = harness(
            "terminal-no-image",
            "stty -icanon -echo; printf ready; cat > input",
        );
        wait_for_screen(&mut harness, "ready");
        click_terminal(&mut harness);
        harness.event(Event::PasteImage(picture()));
        harness.run_steps(2);

        let picture = dir.write("shot.png", "picture");
        let cell = harness.get_by_role(egui::Role::Terminal).rect().center();
        harness.event(Event::PointerMoved(cell));
        harness.step();
        harness
            .input_mut()
            .dropped_files
            .push(Arc::new(Dropped(picture)));
        harness.run_steps(2);

        let input = fs::read_to_string(dir.path().join("input")).unwrap_or_default();
        assert!(input.is_empty(), "{input:?}");
        assert!(image_files(&dir.path().join("images")).is_empty());
    }
}
