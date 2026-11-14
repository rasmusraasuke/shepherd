use std::sync::Arc;

use eframe::egui::text::CCursor;
use eframe::egui::{self, Event, EventFilter, Galley, ImeEvent, Key, Modifiers, Sense, Vec2};

use crate::buffer::{Buffer, Move, Pos};
use crate::code::CodeStyle;

/// The text editing component: shows a buffer and changes it with the keyboard and the mouse.
#[derive(Default)]
pub struct Editor {
    /// Take the keyboard when shown next.
    focus: bool,
    /// Bring the cursor into view when shown next.
    scroll_to_cursor: bool,
    /// Length of the longest line and the revision of the text it was measured for; it gives
    /// the width of the horizontal scroll.
    longest_line: Option<(u64, usize)>,
    /// Where the cursor and the text were when the cursor last moved, and the time of that; it
    /// blinks only while resting.
    cursor_moved: Option<((Pos, u64), f64)>,
    /// Whether the editor or its find bar had the keyboard when shown last.
    active: bool,
    find: Option<Find>,
}

/// The bar for finding and replacing text in the file.
#[derive(Default)]
struct Find {
    query: String,
    replacement: String,
    /// What the query finds, and the query and the revision of the text it was found for.
    found: Vec<(Pos, Pos)>,
    found_for: Option<(String, u64)>,
    /// Put the keyboard into the query when shown next.
    focus: bool,
    has_focus: bool,
}

impl Find {
    fn update(&mut self, buffer: &Buffer) {
        if self.found_for.as_ref().is_none_or(|(query, revision)| *query != self.query || *revision != buffer.revision()) {
            self.found = buffer.find(&self.query);
            self.found_for = Some((self.query.clone(), buffer.revision()));
        }
    }

    /// Selects the nearest found place after the selection, or before it; from the other end of the text when there is none.
    fn go(&mut self, buffer: &mut Buffer, forward: bool) {
        self.update(buffer);
        let (start, end) = buffer.selection().unwrap_or((buffer.cursor(), buffer.cursor()));
        let next = if forward {
            self.found.iter().find(|found| found.0 >= end).or(self.found.first())
        } else {
            self.found.iter().rfind(|found| found.1 <= start).or(self.found.last())
        };
        if let Some((start, end)) = next {
            buffer.select(*start, *end);
        }
    }

    /// Shows the bar. Returns false when it was closed.
    fn ui(&mut self, ui: &mut egui::Ui, buffer: &mut Buffer, editor_active: bool) -> bool {
        self.update(buffer);
        let before = buffer.selection();
        let enter = ui.input(|i| i.key_pressed(Key::Enter));
        let entered = |response: &egui::Response| response.lost_focus() && enter;
        let mut open = !((self.has_focus || editor_active) && ui.input(|i| i.key_pressed(Key::Escape)));
        ui.horizontal_wrapped(|ui| {
            let query = ui.add(egui::TextEdit::singleline(&mut self.query).hint_text("Find").desired_width(180.0));
            if std::mem::take(&mut self.focus) {
                query.request_focus();
            }
            if query.changed() {
                // the nearest place from where the selection starts, so that typing more keeps it
                let start = before.map_or(buffer.cursor(), |(start, _)| start);
                buffer.set_cursor(start, false);
                self.go(buffer, true);
            }
            if entered(&query) {
                self.go(buffer, !ui.input(|i| i.modifiers.shift));
                query.request_focus();
            }
            let current = self.found.iter().position(|found| Some(*found) == buffer.selection()).map_or(0, |index| index + 1);
            ui.label(format!("{current}/{}", self.found.len()));
            // the keyboard stays in the bar after a click on one of its buttons
            let mut refocus = false;
            let mut clicked = |ui: &mut egui::Ui, label: &str, hint: &str| {
                let clicked = ui.button(label).on_hover_text(hint).clicked();
                refocus |= clicked;
                clicked
            };
            if clicked(ui, "Previous", "Shift+Enter") {
                self.go(buffer, false);
            }
            if clicked(ui, "Next", "Enter") {
                self.go(buffer, true);
            }
            let replacement = ui.add(egui::TextEdit::singleline(&mut self.replacement).hint_text("Replace").desired_width(180.0));
            let replace = entered(&replacement);
            if replace {
                replacement.request_focus();
            }
            if clicked(ui, "Replace", "Enter") || replace {
                if self.found.iter().any(|found| Some(*found) == buffer.selection()) {
                    buffer.insert(&self.replacement);
                }
                self.go(buffer, true);
            }
            if clicked(ui, "All", "Replace all") {
                buffer.replace_all(&self.query, &self.replacement);
            }
            self.focus |= refocus;
            open &= !ui.button("×").on_hover_text("Close (Esc)").clicked();
            self.has_focus = query.has_focus() || replacement.has_focus();
        });
        self.update(buffer);
        open
    }
}

impl Editor {
    /// Makes the editor take the keyboard when it is shown next.
    pub fn focus(&mut self) {
        self.focus = true;
    }

    fn longest_line(&mut self, buffer: &Buffer) -> usize {
        match self.longest_line {
            Some((revision, length)) if revision == buffer.revision() => length,
            _ => {
                let length = buffer.lines().iter().map(|line| line.chars().count()).max().unwrap_or(0);
                self.longest_line = Some((buffer.revision(), length));
                length
            }
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, id_salt: &str, buffer: &mut Buffer) {
        if self.active && ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::F)) {
            let selected = buffer.selected_text().filter(|text| !text.contains('\n'));
            let query = selected.or(self.find.take().map(|find| find.query)).unwrap_or_default();
            self.find = Some(Find { query, focus: true, ..Default::default() });
        }
        if let Some(find) = &mut self.find {
            let selection = buffer.selection();
            if !find.ui(ui, buffer, self.active) {
                self.find = None;
                self.focus = true;
            }
            self.scroll_to_cursor |= selection != buffer.selection();
        }
        let style = CodeStyle::new(ui);
        let number_width = style.digit_width * (buffer.lines().len().to_string().len().max(3) + 1) as f32;
        let code_x = number_width + style.digit_width * 2.0;
        let width = code_x + style.digit_width * (self.longest_line(buffer) + 2) as f32;
        let found = self.find.as_ref().map_or(&[][..], |find| &find.found);
        let find_has_focus = self.find.as_ref().is_some_and(|find| find.has_focus);
        let layout = |ui: &egui::Ui, line: &str| ui.painter().layout_no_wrap(line.to_string(), style.font.clone(), style.text_color);
        ui.spacing_mut().item_spacing.y = 0.0;
        egui::ScrollArea::both().id_salt(id_salt).auto_shrink(false).show_viewport(ui, |ui, viewport| {
            let page_rows = (viewport.height() / style.row_height) as isize - 1;
            let size = Vec2::new(width, style.row_height * buffer.lines().len() as f32).max(ui.available_size());
            let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
            // an id of its own keeps the keyboard with the editor when the find bar comes and goes
            let response = ui.interact(rect, egui::Id::unique(("editor", id_salt)), Sense::click_and_drag());
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::MultilineTextInput, ui.is_enabled(), "Editor"));
            let code_left = rect.left() + code_x;

            if let Some(pointer) = response.interact_pointer_pos() {
                let line = (((pointer.y - rect.top()) / style.row_height).max(0.0) as usize).min(buffer.lines().len() - 1);
                // the text under the line numbers can't be clicked: it goes to the start of what is visible
                let x = pointer.x.max(rect.left() + viewport.left() + code_x);
                let col = layout(ui, &buffer.lines()[line]).cursor_from_pos(Vec2::new(x - code_left, 0.0)).index.0;
                let pos = Pos::new(line, col);
                if response.triple_clicked() {
                    buffer.select_line(line);
                } else if response.double_clicked() {
                    buffer.select_word(pos);
                } else if response.is_pointer_button_down_on() && ui.input(|i| i.pointer.primary_pressed()) {
                    response.request_focus();
                    buffer.set_cursor(pos, ui.input(|i| i.modifiers.shift));
                } else if response.dragged() {
                    buffer.set_cursor(pos, true);
                    self.scroll_to_cursor = true;
                }
            }
            if std::mem::take(&mut self.focus) {
                response.request_focus();
            }
            if response.clicked_elsewhere() {
                // or what is typed for another tab, such as the keys of the review, would end up in the text
                response.surrender_focus();
            }
            self.active = response.has_focus() || find_has_focus;
            if response.has_focus() {
                // the keys that would move the focus elsewhere are for the text
                let filter = EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: false };
                ui.memory_mut(|m| m.set_focus_lock_filter(response.id, filter));
                let before = (buffer.cursor(), buffer.revision());
                for event in ui.input(|i| i.events.clone()) {
                    handle_event(ui.ctx(), buffer, event, page_rows);
                }
                self.scroll_to_cursor |= before != (buffer.cursor(), buffer.revision());
            }

            let rows = buffer.lines().len();
            let first = ((viewport.top() / style.row_height) as usize).min(rows);
            let last = ((viewport.bottom() / style.row_height) as usize + 1).min(rows);
            let row_top = |line: usize| rect.top() + style.row_height * line as f32;
            let x_at = |galley: &Arc<Galley>, col: usize| code_left + galley.pos_from_cursor(CCursor::new(col)).left();
            let painter = ui.painter();
            for line in first..last {
                let galley = layout(ui, &buffer.lines()[line]);
                if let Some((start, end)) = buffer.selection().filter(|(start, end)| (start.line..=end.line).contains(&line)) {
                    let left = if line == start.line { x_at(&galley, start.col) } else { code_left };
                    // a selected line break shows as a bit of selection after the line
                    let right = if line == end.line { x_at(&galley, end.col) } else { code_left + galley.size().x + style.digit_width };
                    let selected = egui::Rect::from_min_max(egui::pos2(left, row_top(line)), egui::pos2(right, row_top(line + 1)));
                    painter.rect_filled(selected, 0.0, ui.visuals().selection.bg_fill);
                }
                for (start, end) in &found[found.partition_point(|(start, _)| start.line < line)..] {
                    if start.line != line {
                        break;
                    }
                    let place = egui::Rect::from_min_max(egui::pos2(x_at(&galley, start.col), row_top(line)), egui::pos2(x_at(&galley, end.col), row_top(line + 1)));
                    painter.rect_filled(place, 2.0, ui.visuals().warn_fg_color.gamma_multiply(0.35));
                }
                painter.galley(egui::pos2(code_left, row_top(line) + 1.0), galley, style.text_color);
            }

            let cursor = buffer.cursor();
            let x = x_at(&layout(ui, &buffer.lines()[cursor.line]), cursor.col);
            let cursor_rect = egui::Rect::from_min_max(egui::pos2(x, row_top(cursor.line)), egui::pos2(x + 1.5, row_top(cursor.line + 1)));
            if response.has_focus() && ui.input(|i| i.focused) {
                let now = ui.input(|i| i.time);
                // the cursor stays visible while it moves, and blinks once it rests
                if self.cursor_moved.is_none_or(|(at, _)| at != (cursor, buffer.revision())) {
                    self.cursor_moved = Some(((cursor, buffer.revision()), now));
                }
                let since = self.cursor_moved.map_or(0.0, |(_, time)| now - time);
                egui::text_selection::visuals::paint_text_cursor(ui, painter, cursor_rect, since);
                ui.output_mut(|o| o.ime = Some(egui::output::IMEOutput { rect, cursor_rect, purpose: Default::default(), should_interrupt_composition: false }));
            }

            // the line numbers stay at the left edge when the text scrolls sideways, and cover it
            let gutter_left = rect.left() + viewport.left();
            let gutter = egui::Rect::from_min_max(egui::pos2(gutter_left, viewport.top() + rect.top()), egui::pos2(gutter_left + code_x - style.digit_width, viewport.bottom() + rect.top()));
            painter.rect_filled(gutter, 0.0, ui.visuals().panel_fill);
            for line in first..last {
                style.number(painter, gutter_left + number_width, row_top(line) + 1.0, Some(line as u32 + 1));
            }

            if std::mem::take(&mut self.scroll_to_cursor) {
                // room on the left for the line numbers, so that the cursor doesn't end up under them
                let around = egui::Rect::from_min_max(cursor_rect.min - Vec2::new(code_x + style.digit_width * 4.0, style.row_height), cursor_rect.max + Vec2::new(style.digit_width * 4.0, style.row_height));
                ui.scroll_to_rect(around, None);
            }
        });
    }
}

fn handle_event(ctx: &egui::Context, buffer: &mut Buffer, event: Event, page_rows: isize) {
    match event {
        Event::Text(text) | Event::Ime(ImeEvent::Commit(text)) => buffer.type_text(&text),
        Event::Paste(text) => buffer.insert(&text),
        Event::Copy | Event::Cut => {
            if let Some(text) = buffer.selected_text() {
                ctx.copy_text(text);
                if event == Event::Cut {
                    buffer.insert("");
                }
            }
        }
        Event::Key { key, pressed: true, modifiers, .. } => handle_key(buffer, key, modifiers, page_rows),
        _ => {}
    }
}

fn handle_key(buffer: &mut Buffer, key: Key, modifiers: Modifiers, page_rows: isize) {
    // by words with Alt on macOS and with Ctrl elsewhere; Cmd takes to the ends of the line and of the text
    let by_word = if cfg!(target_os = "macos") { modifiers.alt } else { modifiers.ctrl };
    let to_end = modifiers.mac_cmd;
    let sideways = |word, end, step| if to_end { end } else if by_word { word } else { step };
    let to = match key {
        Key::ArrowLeft | Key::Backspace => sideways(Move::WordLeft, Move::LineStart, Move::Left),
        Key::ArrowRight | Key::Delete => sideways(Move::WordRight, Move::LineEnd, Move::Right),
        Key::ArrowUp if to_end => Move::Start,
        Key::ArrowDown if to_end => Move::End,
        Key::ArrowUp => Move::Rows(-1),
        Key::ArrowDown => Move::Rows(1),
        Key::PageUp => Move::Rows(-page_rows),
        Key::PageDown => Move::Rows(page_rows),
        Key::Home if modifiers.command => Move::Start,
        Key::End if modifiers.command => Move::End,
        Key::Home => Move::LineStart,
        Key::End => Move::LineEnd,
        Key::Enter => return buffer.newline(),
        Key::Tab if modifiers.shift => return buffer.outdent(),
        Key::Tab => return buffer.indent(),
        Key::A if modifiers.command => return buffer.select_all(),
        Key::Z if modifiers.command && modifiers.shift => return buffer.redo(),
        Key::Z if modifiers.command => return buffer.undo(),
        Key::Y if modifiers.command => return buffer.redo(),
        _ => return,
    };
    match key {
        Key::Backspace | Key::Delete => buffer.delete(to),
        _ => buffer.move_cursor(to, modifiers.shift),
    }
}
