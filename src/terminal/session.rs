use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use alacritty_terminal::event::{Event as TermEvent, Notify, OnResize, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, Msg, Notifier};
use alacritty_terminal::grid::Scroll;
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::{Config, Osc52, Term, TermMode};
use alacritty_terminal::tty::{self, Shell};
use alacritty_terminal::vte::ansi::{Color, Rgb};
use eframe::egui::{self, Event, EventFilter, Key, Modifiers, Sense, Vec2};

use super::input::{dropped_images, key_bytes, mouse_bytes, paste_bytes};
use super::paint::{Palette, paint, screen_text};
use super::{
    Activity, BUSY, ClipboardAnswer, ECHO, Listener, Output, PADDING, Size, Worker, size_for,
};
use crate::agents::Agent;
use crate::agents::images;
use crate::editor::code::CodeStyle;

pub(super) struct Terminal {
    pub(super) term: Arc<FairMutex<Term<Listener>>>,
    notifier: Notifier,
    events: Receiver<TermEvent>,
    worker: Option<Worker>,
    pub(super) child: u32,
    pub(super) title: Option<String>,
    output: Arc<Mutex<Output>>,
    pub(super) activity: Activity,
    pub(super) last_input: Option<Instant>,
    worked: bool,
    size: Size,
    pub(super) focus: bool,
    scroll: f32,
    clipboard_request: Option<ClipboardAnswer>,
    clipboard_allowed: bool,
    mouse_button: Option<u8>,
    mouse_cell: Option<(usize, usize)>,
    pub(super) restart: bool,
    pub(super) agent: Option<Agent>,
    pub(super) resumed: bool,
    pub(super) images: Vec<PathBuf>,
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
    pub(super) fn spawn(
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

    pub(super) fn write(&self, bytes: impl Into<Vec<u8>>) {
        self.notifier.notify(bytes.into());
    }

    pub(super) fn update_activity(&mut self) {
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

    pub(super) fn resume_failed(&self) -> bool {
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

    pub(super) fn ui(
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
        let over = |pos: egui::Pos2| {
            rect.contains(pos) && ui.ctx().layer_id_at(pos) == Some(ui.layer_id())
        };
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
                    } if over(pos) || (!pressed && self.mouse_button.is_some()) => {
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
                    Event::PointerMoved(pos) if over(pos) || self.mouse_button.is_some() => {
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
    pub(super) fn stop(&mut self) {
        if self.worker.take().is_none() {
            return;
        }
        let _ = self.notifier.0.send(Msg::Shutdown);
        unsafe {
            libc::kill(-(self.child as libc::pid_t), libc::SIGHUP);
        }
    }

    pub(super) fn kill(&self) {
        unsafe {
            libc::kill(-(self.child as libc::pid_t), libc::SIGKILL);
        }
    }
}
