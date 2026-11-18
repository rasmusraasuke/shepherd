pub mod buffer;
pub mod code;
pub mod file_view;
pub mod highlight;

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use eframe::egui::text::CCursor;
use eframe::egui::{
    self, Color32, Event, EventFilter, Galley, ImeEvent, Key, Modifiers, Sense, Vec2,
};

use buffer::{Buffer, Move, Pos};
use code::CodeStyle;
use highlight::Highlighter;

#[derive(Default)]
pub struct Editor {
    focus: bool,
    scroll_to_cursor: bool,
    center_cursor: bool,
    longest_line: Option<(u64, usize)>,
    cursor_moved: Option<((Pos, u64), f64)>,
    active: bool,
    find: Option<Find>,
}

pub struct Rows {
    pub of_line: Vec<Option<usize>>,
    pub lines: Vec<Option<usize>>,
    pub marks: HashMap<usize, (Vec<Range<usize>>, Color32)>,
}

#[derive(Default)]
struct Find {
    query: String,
    replacement: String,
    found: Vec<(Pos, Pos)>,
    found_for: Option<(String, u64)>,
    focus: bool,
    has_focus: bool,
}

impl Find {
    fn update(&mut self, buffer: &Buffer) {
        if self
            .found_for
            .as_ref()
            .is_none_or(|(query, revision)| *query != self.query || *revision != buffer.revision())
        {
            self.found = buffer.find(&self.query);
            self.found_for = Some((self.query.clone(), buffer.revision()));
        }
    }

    fn go(&mut self, buffer: &mut Buffer, forward: bool) {
        self.update(buffer);
        let (start, end) = buffer
            .selection()
            .unwrap_or((buffer.cursor(), buffer.cursor()));
        let next = if forward {
            self.found
                .iter()
                .find(|found| found.0 >= end)
                .or(self.found.first())
        } else {
            self.found
                .iter()
                .rfind(|found| found.1 <= start)
                .or(self.found.last())
        };
        if let Some((start, end)) = next {
            buffer.select(*start, *end);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, buffer: &mut Buffer, editor_active: bool) -> bool {
        self.update(buffer);
        let before = buffer.selection();
        let enter = ui.input(|i| i.key_pressed(Key::Enter));
        let entered = |response: &egui::Response| response.lost_focus() && enter;
        let mut open =
            !((self.has_focus || editor_active) && ui.input(|i| i.key_pressed(Key::Escape)));
        ui.horizontal_wrapped(|ui| {
            let query = ui.add(
                egui::TextEdit::singleline(&mut self.query)
                    .hint_text("Find")
                    .desired_width(180.0),
            );
            if std::mem::take(&mut self.focus) {
                query.request_focus();
            }
            if query.changed() {
                let start = before.map_or(buffer.cursor(), |(start, _)| start);
                buffer.set_cursor(start, false);
                self.go(buffer, true);
            }
            if entered(&query) {
                self.go(buffer, !ui.input(|i| i.modifiers.shift));
                query.request_focus();
            }
            let current = self
                .found
                .iter()
                .position(|found| Some(*found) == buffer.selection())
                .map_or(0, |index| index + 1);
            ui.label(format!("{current}/{}", self.found.len()));
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
            let replacement = ui.add(
                egui::TextEdit::singleline(&mut self.replacement)
                    .hint_text("Replace")
                    .desired_width(180.0),
            );
            let replace = entered(&replacement);
            if replace {
                replacement.request_focus();
            }
            if clicked(ui, "Replace", "Enter") || replace {
                if self
                    .found
                    .iter()
                    .any(|found| Some(*found) == buffer.selection())
                {
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
    pub fn focus(&mut self) {
        self.focus = true;
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn reveal(&mut self, focus: bool) {
        self.focus |= focus;
        self.center_cursor = true;
    }

    fn longest_line(&mut self, buffer: &Buffer) -> usize {
        match self.longest_line {
            Some((revision, length)) if revision == buffer.revision() => length,
            _ => {
                let length = buffer
                    .lines()
                    .iter()
                    .map(|line| line.chars().count())
                    .max()
                    .unwrap_or(0);
                self.longest_line = Some((buffer.revision(), length));
                length
            }
        }
    }

    pub fn find_ui(&mut self, ui: &mut egui::Ui, buffer: &mut Buffer) {
        if self.active && ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::F)) {
            let selected = buffer.selected_text().filter(|text| !text.contains('\n'));
            let query = selected
                .or(self.find.take().map(|find| find.query))
                .unwrap_or_default();
            self.find = Some(Find {
                query,
                focus: true,
                ..Default::default()
            });
        }
        if let Some(find) = &mut self.find {
            let selection = buffer.selection();
            if !find.ui(ui, buffer, self.active) {
                self.find = None;
                self.focus = true;
            }
            self.scroll_to_cursor |= selection != buffer.selection();
        }
    }

    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        id_salt: &str,
        buffer: &mut Buffer,
        highlighter: Option<&mut Highlighter>,
    ) {
        self.find_ui(ui, buffer);
        let style = CodeStyle::new(ui);
        let number_width =
            style.digit_width * (buffer.lines().len().to_string().len().max(3) + 1) as f32;
        let code_x = number_width + style.digit_width * 2.0;
        let width = code_x + style.digit_width * (self.longest_line(buffer) + 2) as f32;
        ui.spacing_mut().item_spacing.y = 0.0;
        egui::ScrollArea::both()
            .id_salt(id_salt)
            .auto_shrink(false)
            .show_viewport(ui, |ui, viewport| {
                let size = Vec2::new(width, style.row_height * buffer.lines().len() as f32)
                    .max(ui.available_size());
                let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
                let visible = viewport.translate(rect.min.to_vec2());
                self.text_ui(
                    ui,
                    id_salt,
                    buffer,
                    highlighter,
                    rect,
                    visible,
                    code_x,
                    None,
                );

                let painter = ui.painter();
                let gutter = egui::Rect::from_min_max(
                    visible.min,
                    egui::pos2(
                        visible.left() + code_x - style.digit_width,
                        visible.bottom(),
                    ),
                );
                painter.rect_filled(gutter, 0.0, ui.visuals().panel_fill);
                let first = (viewport.top() / style.row_height) as usize;
                let last =
                    ((viewport.bottom() / style.row_height) as usize + 1).min(buffer.lines().len());
                for line in first.min(last)..last {
                    style.number(
                        painter,
                        visible.left() + number_width,
                        rect.top() + style.row_height * line as f32 + 1.0,
                        Some(line as u32 + 1),
                    );
                }
            });
    }

    #[allow(clippy::too_many_arguments)]
    pub fn text_ui(
        &mut self,
        ui: &mut egui::Ui,
        id_salt: &str,
        buffer: &mut Buffer,
        highlighter: Option<&mut Highlighter>,
        rect: egui::Rect,
        visible: egui::Rect,
        code_x: f32,
        rows: Option<&Rows>,
    ) -> egui::Response {
        let style = CodeStyle::new(ui);
        let dark = ui.visuals().dark_mode;
        let line_count = buffer.lines().len();
        let row_of =
            |line: usize| rows.map_or(Some(line), |rows| rows.of_line.get(line).copied().flatten());
        let line_at = |row: usize| match rows {
            None => row.min(line_count - 1),
            Some(rows) => rows
                .lines
                .get(row..)
                .and_then(|after| after.iter().flatten().next().copied())
                .unwrap_or(line_count - 1),
        };
        let found = self.find.as_ref().map_or(&[][..], |find| &find.found);
        let find_has_focus = self.find.as_ref().is_some_and(|find| find.has_focus);
        let layout = |ui: &egui::Ui, line: &str| {
            ui.painter()
                .layout_no_wrap(line.to_string(), style.font.clone(), style.text_color)
        };
        let page_rows = (visible.height() / style.row_height) as isize - 1;
        let code_left = rect.left() + code_x;
        let interactive = if rows.is_some() {
            rect.intersect(egui::Rect::everything_right_of(
                visible.left() + code_x - style.digit_width,
            ))
        } else {
            rect
        };
        let response = ui.interact(
            interactive,
            egui::Id::unique(("editor", id_salt)),
            Sense::click_and_drag(),
        );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::MultilineTextInput, ui.is_enabled(), "Editor")
        });

        if let Some(pointer) = response.interact_pointer_pos() {
            let line = line_at(((pointer.y - rect.top()) / style.row_height).max(0.0) as usize);
            let x = pointer.x.max(visible.left() + code_x);
            let col = layout(ui, &buffer.lines()[line])
                .cursor_from_pos(Vec2::new(x - code_left, 0.0))
                .index
                .0;
            let pos = Pos::new(line, col);
            if response.triple_clicked() {
                buffer.select_line(line);
            } else if response.double_clicked() {
                buffer.select_word(pos);
            } else if response.is_pointer_button_down_on()
                && ui.input(|i| i.pointer.primary_pressed())
            {
                response.request_focus();
                buffer.set_cursor(pos, ui.input(|i| i.modifiers.shift));
            } else if response.dragged() {
                buffer.set_cursor(pos, true);
                self.scroll_to_cursor = true;
            }
        }
        let took_focus = std::mem::take(&mut self.focus);
        if took_focus {
            response.request_focus();
        }
        if response.clicked_elsewhere() {
            response.surrender_focus();
        }
        self.active = response.has_focus() || find_has_focus;
        if response.has_focus() && !took_focus {
            let filter = EventFilter {
                tab: true,
                horizontal_arrows: true,
                vertical_arrows: true,
                escape: false,
            };
            ui.memory_mut(|m| m.set_focus_lock_filter(response.id, filter));
            let before = (buffer.cursor(), buffer.revision());
            for event in ui.input(|i| i.events.clone()) {
                handle_event(ui.ctx(), buffer, event, page_rows);
            }
            self.scroll_to_cursor |= before != (buffer.cursor(), buffer.revision());
        }

        let row_count = rows.map_or(buffer.lines().len(), |rows| rows.lines.len());
        let first_row =
            (((visible.top() - rect.top()) / style.row_height).max(0.0) as usize).min(row_count);
        let last_row = (((visible.bottom() - rect.top()) / style.row_height).max(0.0) as usize + 1)
            .min(row_count);
        let shown: Vec<usize> = match rows {
            None => (first_row..last_row).collect(),
            Some(rows) => rows.lines[first_row..last_row]
                .iter()
                .flatten()
                .copied()
                .collect(),
        };
        let row_top = |row: usize| rect.top() + style.row_height * row as f32;
        let x_at = |galley: &Arc<Galley>, col: usize| {
            code_left + galley.pos_from_cursor(CCursor::new(col)).left()
        };
        let painter = ui.painter();
        let first = shown.first().copied().unwrap_or(0);
        let spans = match highlighter {
            Some(highlighter) => {
                highlighter.update(buffer);
                Some(
                    highlighter
                        .spans(first..shown.last().map_or(first, |last| last + 1))
                        .to_vec(),
                )
            }
            None => {
                buffer.take_changes();
                None
            }
        };
        for &line in &shown {
            let Some(row) = row_of(line) else { continue };
            let text = &buffer.lines()[line];
            let (marked, mark) = rows
                .and_then(|rows| rows.marks.get(&line))
                .map_or((&[][..], Color32::TRANSPARENT), |(marked, mark)| {
                    (marked.as_slice(), *mark)
                });
            let line_spans = spans
                .as_ref()
                .and_then(|spans| spans.get(line - first))
                .map_or(&[][..], Vec::as_slice);
            let galley = ui.fonts_mut(|fonts| {
                fonts.layout_job(highlight::layout_job(
                    text,
                    line_spans,
                    marked,
                    mark,
                    &style.font,
                    style.text_color,
                    dark,
                ))
            });
            if let Some((start, end)) = buffer
                .selection()
                .filter(|(start, end)| (start.line..=end.line).contains(&line))
            {
                let left = if line == start.line {
                    x_at(&galley, start.col)
                } else {
                    code_left
                };
                let right = if line == end.line {
                    x_at(&galley, end.col)
                } else {
                    code_left + galley.size().x + style.digit_width
                };
                let selected = egui::Rect::from_min_max(
                    egui::pos2(left, row_top(row)),
                    egui::pos2(right, row_top(row + 1)),
                );
                painter.rect_filled(selected, 0.0, ui.visuals().selection.bg_fill);
            }
            for (start, end) in &found[found.partition_point(|(start, _)| start.line < line)..] {
                if start.line != line {
                    break;
                }
                let place = egui::Rect::from_min_max(
                    egui::pos2(x_at(&galley, start.col), row_top(row)),
                    egui::pos2(x_at(&galley, end.col), row_top(row + 1)),
                );
                painter.rect_filled(place, 2.0, ui.visuals().warn_fg_color.gamma_multiply(0.35));
            }
            painter.galley(
                egui::pos2(code_left, row_top(row) + 1.0),
                galley,
                style.text_color,
            );
        }

        let cursor = buffer.cursor();
        let Some(cursor_row) = row_of(cursor.line) else {
            return response;
        };
        let x = x_at(&layout(ui, &buffer.lines()[cursor.line]), cursor.col);
        let cursor_rect = egui::Rect::from_min_max(
            egui::pos2(x, row_top(cursor_row)),
            egui::pos2(x + 1.5, row_top(cursor_row + 1)),
        );
        if response.has_focus() && ui.input(|i| i.focused) {
            let now = ui.input(|i| i.time);
            if self
                .cursor_moved
                .is_none_or(|(at, _)| at != (cursor, buffer.revision()))
            {
                self.cursor_moved = Some(((cursor, buffer.revision()), now));
            }
            let since = self.cursor_moved.map_or(0.0, |(_, time)| now - time);
            egui::text_selection::visuals::paint_text_cursor(ui, painter, cursor_rect, since);
            ui.output_mut(|o| {
                o.ime = Some(egui::output::IMEOutput {
                    rect,
                    cursor_rect,
                    purpose: Default::default(),
                    should_interrupt_composition: false,
                })
            });
        }
        if std::mem::take(&mut self.center_cursor) {
            self.scroll_to_cursor = false;
            ui.scroll_to_rect(cursor_rect, Some(egui::Align::Center));
        }
        if std::mem::take(&mut self.scroll_to_cursor) {
            let around = egui::Rect::from_min_max(
                cursor_rect.min - Vec2::new(code_x + style.digit_width * 4.0, style.row_height),
                cursor_rect.max + Vec2::new(style.digit_width * 4.0, style.row_height),
            );
            ui.scroll_to_rect(around, None);
        }
        response
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
        Event::Key {
            key,
            pressed: true,
            modifiers,
            ..
        } => handle_key(buffer, key, modifiers, page_rows),
        _ => {}
    }
}

fn handle_key(buffer: &mut Buffer, key: Key, modifiers: Modifiers, page_rows: isize) {
    let by_word = if cfg!(target_os = "macos") {
        modifiers.alt
    } else {
        modifiers.ctrl
    };
    let to_end = modifiers.mac_cmd;
    let sideways = |word, end, step| {
        if to_end {
            end
        } else if by_word {
            word
        } else {
            step
        }
    };
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
