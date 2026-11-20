use std::collections::HashMap;
use std::ops::Range;

use eframe::egui::{self, Align, Color32, Sense, Stroke, Vec2};

use super::comments::Comment;
use crate::editor::Rows;
use crate::editor::buffer::Buffer;
use crate::editor::code::CodeStyle;
use crate::editor::highlight::{self, Highlighter};
use crate::git::{Conflict, DiffLine, Hunk, LineKind};

use super::text::{FileText, Gap, gaps, row_colors, unchanged_line};
use super::{CONTEXT, FileReview, NEW_COLOR, View};

#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum Side {
    Ours,
    Theirs,
    Both,
}

#[derive(Debug, PartialEq)]
pub(super) enum Row {
    Unchanged(usize, usize),
    Folded(usize, usize),
    Hunk(usize, usize),
    Comment(usize, usize),
}

pub(super) struct CommentRows {
    kind: LineKind,
    number: u32,
    comment: usize,
    rows: usize,
}

fn folded(gaps: &[Gap], hunks: usize, gap: usize) -> Option<Range<usize>> {
    let start = if gap > 0 { CONTEXT } else { 0 };
    let end = if gap < hunks { CONTEXT } else { 0 };
    let lines = gaps[gap].lines.clone();
    (lines.len() > start + end + 1).then(|| lines.start + start..lines.end - end)
}

pub(super) fn rows(
    gaps: &[Gap],
    hunks: &[Hunk],
    expanded: &[bool],
    comments: &[CommentRows],
) -> (Vec<Row>, Vec<usize>) {
    let (mut rows, mut starts) = (Vec::new(), Vec::new());
    let add = |rows: &mut Vec<Row>, row: Row, kind: LineKind, lines: Range<usize>| {
        rows.push(row);
        for comment in comments
            .iter()
            .filter(|comment| comment.kind == kind && lines.contains(&(comment.number as usize)))
        {
            rows.extend((0..comment.rows).map(|line| Row::Comment(comment.comment, line)));
        }
    };
    for (index, gap) in gaps.iter().enumerate() {
        let unchanged = |rows: &mut Vec<Row>, lines: Range<usize>| {
            lines.for_each(|number| {
                add(
                    rows,
                    Row::Unchanged(index, number),
                    LineKind::Context,
                    number..number + 1,
                )
            })
        };
        match folded(gaps, hunks.len(), index).filter(|_| !expanded[index]) {
            Some(folded) => {
                unchanged(&mut rows, gap.lines.start..folded.start);
                add(
                    &mut rows,
                    Row::Folded(index, folded.len()),
                    LineKind::Context,
                    folded.clone(),
                );
                unchanged(&mut rows, folded.end..gap.lines.end);
            }
            None => unchanged(&mut rows, gap.lines.clone()),
        }
        if let Some(hunk) = hunks.get(index) {
            starts.push(rows.len());
            for (at, line) in hunk.lines.iter().enumerate() {
                let number = line.number() as usize;
                add(
                    &mut rows,
                    Row::Hunk(index, at),
                    line.kind,
                    number..number + 1,
                );
            }
        }
    }
    (rows, starts)
}

const GUTTER_DIGITS: usize = 21;

const COMMENT_MARK: f32 = 4.4;

pub(super) fn place_in_file(hunk: &Hunk, line: Option<&DiffLine>) -> usize {
    line.and_then(|line| line.new_no)
        .unwrap_or(hunk.ranges().1.start) as usize
        - 1
}

pub(super) enum Clicked {
    Row(usize),
    Hunk(usize),
    Line(usize, usize),
    DeleteComment(usize),
    Revert(usize),
    Resolve(usize, Side),
    AskConflict(usize),
    Open(usize),
}

#[allow(clippy::too_many_arguments)]
pub(super) fn rows_ui(
    ui: &mut egui::Ui,
    path: &str,
    file: &FileReview,
    text: &FileText,
    view: &mut View,
    comments: &[(usize, &Comment)],
    buffer: &mut Buffer,
    highlighter: Option<&mut Highlighter>,
    can_ask: bool,
    editable: bool,
) -> (Option<Clicked>, Vec<DiffLine>) {
    let hunks = &file.diff.hunks;
    let gaps = gaps(hunks, text.work_lines.len());
    view.expanded.resize(gaps.len(), false);
    let ends: Vec<CommentRows> = comments
        .iter()
        .map(|&(index, comment)| CommentRows {
            kind: comment.last_line().kind,
            number: comment.last_line().number(),
            comment: index,
            rows: comment.text.lines().count().max(1),
        })
        .collect();
    let (mut rows, starts) = rows(&gaps, hunks, &view.expanded, &ends);
    if buffer.lines().len() > text.work_lines.len() && !gaps.is_empty() {
        rows.push(Row::Unchanged(gaps.len() - 1, buffer.lines().len()));
    }
    let style = CodeStyle::new(ui);
    let (row_height, digit) = (style.row_height, style.digit_width);
    let dark = ui.visuals().dark_mode;
    let width = digit * (GUTTER_DIGITS + text.longest + 2) as f32;
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(
            ui.available_width().max(width),
            row_height * rows.len() as f32,
        ),
        Sense::click_and_drag(),
    );
    let row_at = |index: usize| {
        egui::Rect::from_min_size(
            egui::pos2(rect.left(), rect.top() + row_height * index as f32),
            Vec2::new(rect.width(), row_height),
        )
    };
    let row_under = |y: f32| {
        (((y - rect.top()) / row_height).max(0.0) as usize).min(rows.len().saturating_sub(1))
    };
    if view.scroll_to_current
        && let Some(&start) = starts.get(view.current)
    {
        ui.scroll_to_rect(row_at(start.saturating_sub(3)), Some(Align::TOP));
    }
    let clip = ui.clip_rect();
    let first = ((clip.top() - rect.top()) / row_height).floor().max(0.0) as usize;
    let last =
        (((clip.bottom() - rect.top()) / row_height).ceil().max(0.0) as usize).min(rows.len());
    let gutter_left = rect.left().max(clip.left());
    let column = |digits: f32, row: egui::Rect| {
        egui::Rect::from_min_size(
            egui::pos2(gutter_left + digit * digits, row.top()),
            Vec2::splat(row_height),
        )
    };
    let in_file = |number: usize| number <= text.work_lines.len();

    let mut editor_rows = Rows {
        of_line: vec![None; buffer.lines().len()],
        lines: vec![None; rows.len()],
        marks: HashMap::new(),
    };
    for (index, row) in rows.iter().enumerate() {
        let line = match *row {
            Row::Unchanged(_, number) => number - 1,
            Row::Hunk(hunk, at) if hunks[hunk].lines[at].kind == LineKind::Added => {
                let line = hunks[hunk].lines[at].new_no.unwrap_or_default() as usize - 1;
                let mark = row_colors(LineKind::Added, dark)
                    .map_or(Color32::TRANSPARENT, |(_, mark)| {
                        fade(mark, editable && file.staged[hunk][at])
                    });
                editor_rows
                    .marks
                    .insert(line, (text.changed[hunk][at].clone(), mark));
                line
            }
            _ => continue,
        };
        if line < editor_rows.of_line.len() {
            editor_rows.of_line[line] = Some(index);
            editor_rows.lines[index] = Some(line);
        }
    }

    let is_line = |row: &Row| !matches!(row, Row::Comment(..));
    if response.drag_started()
        && let Some(origin) = ui.input(|i| i.pointer.press_origin())
        && column(COMMENT_MARK, row_at(0)).x_range().contains(origin.x)
        && rows.get(row_under(origin.y)).is_some_and(is_line)
    {
        view.selection = Some((row_under(origin.y), row_under(origin.y)));
        view.selecting = true;
    }
    if view.selecting {
        if let (Some((start, _)), Some(pointer)) = (view.selection, response.interact_pointer_pos())
        {
            view.selection = Some((start, row_under(pointer.y)));
        }
        view.selecting = response.dragged();
        view.focus_draft = !view.selecting;
    }
    let selected = view
        .selection
        .map(|(start, end)| start.min(end)..=start.max(end));

    for (index, row) in rows.iter().enumerate().take(last).skip(first) {
        let row_rect = row_at(index);
        match *row {
            Row::Hunk(hunk, at) => {
                let line = &hunks[hunk].lines[at];
                let conflict = hunks[hunk].conflict.is_some();
                let mark = line_background(
                    ui,
                    row_rect,
                    line.kind,
                    editable && file.staged[hunk][at],
                    conflict,
                );
                if line.kind == LineKind::Removed {
                    let (code, spans) = text.old.line(line.old_no);
                    let galley = ui.fonts_mut(|fonts| {
                        fonts.layout_job(highlight::layout_job(
                            code,
                            spans,
                            &text.changed[hunk][at],
                            mark,
                            &style.font,
                            style.text_color,
                            dark,
                        ))
                    });
                    ui.painter().galley(
                        egui::pos2(
                            rect.left() + digit * GUTTER_DIGITS as f32,
                            row_rect.top() + 1.0,
                        ),
                        galley,
                        style.text_color,
                    );
                }
            }
            Row::Comment(index, line) => {
                let Some((_, comment)) = comments.iter().find(|(at, _)| *at == index) else {
                    continue;
                };
                comment_line_ui(
                    ui,
                    &style,
                    egui::Rect::from_x_y_ranges(gutter_left..=clip.right(), row_rect.y_range()),
                    comment,
                    line,
                );
            }
            Row::Unchanged(..) | Row::Folded(..) => {}
        }
        if selected
            .as_ref()
            .is_some_and(|selected| selected.contains(&index))
        {
            ui.painter().rect_filled(
                row_rect,
                0.0,
                ui.visuals().selection.bg_fill.gamma_multiply(0.4),
            );
        }
    }

    let editing = view.editor.text_ui(
        ui,
        &format!("review {path}"),
        buffer,
        highlighter,
        rect,
        clip.intersect(rect),
        digit * GUTTER_DIGITS as f32,
        Some(&editor_rows),
    );
    let cursor = buffer.cursor().line;
    if view.editor.is_active()
        && editor_rows.of_line.get(cursor).is_some_and(Option::is_none)
        && let Some(gap) = gaps
            .iter()
            .position(|gap| gap.lines.contains(&(cursor + 1)))
    {
        view.expanded[gap] = true;
        ui.ctx().request_repaint();
    }

    let mut clicked = None;
    let hovered = ui
        .input(|i| i.pointer.hover_pos())
        .filter(|pointer| rect.contains(*pointer) && clip.contains(*pointer))
        .map(|pointer| row_under(pointer.y));
    for (index, row) in rows.iter().enumerate().take(last).skip(first) {
        let row_rect = row_at(index);
        match *row {
            Row::Folded(gap, hidden) => {
                let visible =
                    egui::Rect::from_x_y_ranges(gutter_left..=clip.right(), row_rect.y_range());
                if fold_ui(ui, visible, hidden, response.id.with(("fold", gap))) {
                    view.expanded[gap] = true;
                    view.selection = None;
                }
            }
            Row::Unchanged(gap, number) => {
                let numbers = (
                    in_file(number)
                        .then(|| u32::try_from(number as isize + gaps[gap].old_offset).ok())
                        .flatten(),
                    in_file(number).then_some(number as u32),
                );
                line_gutter(
                    ui,
                    &style,
                    row_rect,
                    gutter_left,
                    LineKind::Context,
                    numbers,
                    false,
                    false,
                );
            }
            Row::Hunk(hunk, at) => {
                let line = &hunks[hunk].lines[at];
                let staged = file.staged[hunk][at];
                let conflict = hunks[hunk].conflict.is_some();
                line_gutter(
                    ui,
                    &style,
                    row_rect,
                    gutter_left,
                    line.kind,
                    (line.old_no, line.new_no),
                    editable && staged,
                    conflict,
                );
                if hunk == view.current {
                    let bar = egui::Rect::from_min_size(
                        egui::pos2(gutter_left, row_rect.top()),
                        Vec2::new(3.0, row_height),
                    );
                    ui.painter()
                        .rect_filled(bar, 0.0, ui.visuals().selection.stroke.color);
                }
                if file.new[hunk] && !conflict {
                    let bar = egui::Rect::from_min_size(
                        egui::pos2(gutter_left + digit * 19.7, row_rect.top()),
                        Vec2::new(2.0, row_height),
                    );
                    ui.painter().rect_filled(bar, 0.0, NEW_COLOR);
                }
                let (done, total) = file.hunk_progress(hunk);
                if at == 0 && !conflict {
                    let state = if done == total {
                        Some(true)
                    } else if done == 0 {
                        Some(false)
                    } else {
                        None
                    };
                    if reviewed_mark_ui(
                        ui,
                        column(0.5, row_rect),
                        0.0,
                        state,
                        "Reviewed",
                        response.id.with(("hunk", hunk)),
                        editable,
                    ) {
                        clicked = Some(Clicked::Hunk(hunk));
                    }
                }
                if conflict
                    && at == 0
                    && let Some(conflict) = &hunks[hunk].conflict
                {
                    clicked = conflict_tools(
                        ui,
                        row_rect,
                        clip,
                        gutter_left + digit * GUTTER_DIGITS as f32,
                        hunk,
                        response.id.with(("conflict", hunk)),
                        conflict,
                        can_ask,
                    );
                }
                let partly = done > 0 && done < total;
                let shown = editable
                    && !conflict
                    && (hovered == Some(index) || partly)
                    && line.kind != LineKind::Context;
                if shown
                    && reviewed_mark_ui(
                        ui,
                        column(2.7, row_rect),
                        2.0,
                        Some(staged),
                        "Reviewed line",
                        response.id.with(("line", hunk, at)),
                        true,
                    )
                {
                    clicked = Some(Clicked::Line(hunk, at));
                }
            }
            Row::Comment(index, line) => {
                if line == 0
                    && delete_mark_ui(
                        ui,
                        column(0.5, row_rect),
                        response.id.with(("delete comment", index)),
                    )
                {
                    clicked = Some(Clicked::DeleteComment(index));
                }
            }
        }
        if hovered == Some(index)
            && !view.selecting
            && matches!(row, Row::Unchanged(..) | Row::Hunk(..))
            && comment_mark_ui(
                ui,
                column(COMMENT_MARK, row_rect),
                response.id.with(("comment", index)),
            )
        {
            let shift = ui.input(|i| i.modifiers.shift);
            view.selection = Some(match view.selection {
                Some((start, _)) if shift => (start, index),
                _ => (index, index),
            });
            view.focus_draft = true;
        }
    }

    let pointer_row = || {
        ui.input(|i| i.pointer.interact_pos())
            .map(|pointer| row_under(pointer.y))
    };
    if clicked.is_none()
        && (response.clicked() || editing.clicked())
        && let Some(&Row::Hunk(hunk, _)) = pointer_row().and_then(|row| rows.get(row))
    {
        clicked = Some(Clicked::Row(hunk));
    }
    let menu = response.union(editing);
    if menu.secondary_clicked() {
        view.menu_row = pointer_row();
    }
    menu.context_menu(|ui| {
        let mut item = |label: &str, then: Clicked| {
            if ui.button(label).clicked() {
                clicked = Some(then);
            }
        };
        match view.menu_row.and_then(|row| rows.get(row)) {
            Some(&Row::Hunk(hunk, at)) if hunks[hunk].conflict.is_some() => {
                item("Take ours", Clicked::Resolve(hunk, Side::Ours));
                item("Take theirs", Clicked::Resolve(hunk, Side::Theirs));
                item("Take both", Clicked::Resolve(hunk, Side::Both));
                if can_ask {
                    item("Ask agent", Clicked::AskConflict(hunk));
                }
                item(
                    "Open in file (o)",
                    Clicked::Open(place_in_file(&hunks[hunk], Some(&hunks[hunk].lines[at]))),
                );
            }
            Some(&Row::Hunk(hunk, at)) => {
                if editable {
                    item("Revert", Clicked::Revert(hunk));
                }
                item(
                    "Open in file (o)",
                    Clicked::Open(place_in_file(&hunks[hunk], Some(&hunks[hunk].lines[at]))),
                );
            }
            Some(&Row::Unchanged(_, number)) => item("Open in file", Clicked::Open(number - 1)),
            _ => ui.close(),
        }
    });

    let selected = view
        .selection
        .and_then(|(start, end)| rows.get(start.min(end)..=start.max(end)));
    let lines = selected
        .unwrap_or_default()
        .iter()
        .flat_map(|row| match *row {
            Row::Unchanged(gap, number) => {
                vec![unchanged_line(&text.work_lines, &gaps[gap], number)]
            }
            Row::Folded(gap, _) => folded(&gaps, hunks.len(), gap)
                .unwrap_or_default()
                .map(|number| unchanged_line(&text.work_lines, &gaps[gap], number))
                .collect(),
            Row::Hunk(hunk, at) => vec![hunks[hunk].lines[at].clone()],
            Row::Comment(..) => Vec::new(),
        });
    (clicked, lines.collect())
}

pub(super) fn fade(color: Color32, reviewed: bool) -> Color32 {
    if reviewed {
        color.gamma_multiply(0.4)
    } else {
        color
    }
}

fn conflict_colors(dark_mode: bool) -> (Color32, Color32) {
    if dark_mode {
        (Color32::from_rgb(58, 38, 36), Color32::from_rgb(94, 58, 52))
    } else {
        (
            Color32::from_rgb(255, 236, 232),
            Color32::from_rgb(244, 198, 188),
        )
    }
}

fn line_background(
    ui: &egui::Ui,
    row: egui::Rect,
    kind: LineKind,
    reviewed: bool,
    conflict: bool,
) -> Color32 {
    let colors = if conflict {
        Some(conflict_colors(ui.visuals().dark_mode))
    } else {
        row_colors(kind, ui.visuals().dark_mode)
    };
    match colors {
        Some((bg, mark)) => {
            ui.painter().rect_filled(row, 0.0, fade(bg, reviewed));
            fade(mark, reviewed)
        }
        None => Color32::TRANSPARENT,
    }
}

#[allow(clippy::too_many_arguments)]
fn line_gutter(
    ui: &egui::Ui,
    style: &CodeStyle,
    row: egui::Rect,
    gutter_left: f32,
    kind: LineKind,
    (old_no, new_no): (Option<u32>, Option<u32>),
    reviewed: bool,
    conflict: bool,
) {
    let painter = ui.painter();
    let digit = style.digit_width;
    let gutter = egui::Rect::from_min_size(
        egui::pos2(gutter_left, row.top()),
        Vec2::new(digit * (GUTTER_DIGITS - 1) as f32, row.height()),
    );
    painter.rect_filled(gutter, 0.0, ui.visuals().panel_fill);
    line_background(ui, gutter, kind, reviewed, conflict);
    let y = row.top() + 1.0;
    style.number(painter, gutter_left + digit * 12.0, y, old_no);
    style.number(painter, gutter_left + digit * 17.0, y, new_no);
    let sign = match (conflict, kind) {
        (true, _) | (false, LineKind::Context) => " ",
        (false, LineKind::Added) => "+",
        (false, LineKind::Removed) => "-",
    };
    style.text(
        painter,
        gutter_left + digit * 18.5,
        y,
        sign,
        style.weak_color,
    );
}

#[allow(clippy::too_many_arguments)]
fn conflict_tools(
    ui: &egui::Ui,
    row: egui::Rect,
    clip: egui::Rect,
    left: f32,
    hunk: usize,
    id: egui::Id,
    conflict: &Conflict,
    can_ask: bool,
) -> Option<Clicked> {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let ours = format!("Keep {}", conflict.ours);
    let theirs = format!("Keep {}", conflict.theirs);
    let mut items: Vec<(&str, &str, Option<Side>)> = vec![
        ("Ours", &ours, Some(Side::Ours)),
        ("Theirs", &theirs, Some(Side::Theirs)),
        ("Both", "Keep both", Some(Side::Both)),
    ];
    if can_ask {
        items.push(("Ask agent", "Ask an agent to resolve it", None));
    }
    let widths: Vec<f32> = items
        .iter()
        .map(|(label, _, _)| {
            ui.painter()
                .layout_no_wrap((*label).to_string(), font.clone(), Color32::PLACEHOLDER)
                .size()
                .x
                + 14.0
        })
        .collect();
    let gap = 4.0;
    let total: f32 = widths.iter().sum::<f32>() + gap * (items.len() - 1) as f32;
    let mut x = (clip.right() - total - 6.0).max(left);
    let mut clicked = None;
    for ((label, hint, side), width) in items.iter().zip(widths) {
        let rect = egui::Rect::from_min_size(
            egui::pos2(x, row.top() + 2.0),
            Vec2::new(width, row.height() - 4.0),
        );
        if conflict_button(ui, rect, label, hint, id.with(*label)) {
            clicked = Some(match side {
                Some(side) => Clicked::Resolve(hunk, *side),
                None => Clicked::AskConflict(hunk),
            });
        }
        x += width + gap;
    }
    clicked
}

fn conflict_button(ui: &egui::Ui, rect: egui::Rect, label: &str, hint: &str, id: egui::Id) -> bool {
    let response = ui
        .interact(rect, id, Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), label));
    let visuals = ui.style().interact(&response);
    ui.painter().rect(
        rect,
        3.0,
        visuals.bg_fill,
        visuals.bg_stroke,
        egui::StrokeKind::Inside,
    );
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        egui::TextStyle::Body.resolve(ui.style()),
        visuals.text_color(),
    );
    response.on_hover_text(hint).clicked()
}

fn comment_colors(dark_mode: bool) -> (Color32, Color32) {
    if dark_mode {
        (
            Color32::from_rgb(58, 54, 38),
            Color32::from_rgb(200, 170, 80),
        )
    } else {
        (
            Color32::from_rgb(255, 247, 214),
            Color32::from_rgb(214, 170, 40),
        )
    }
}

fn comment_line_ui(
    ui: &egui::Ui,
    style: &CodeStyle,
    row: egui::Rect,
    comment: &Comment,
    line: usize,
) {
    let painter = ui.painter();
    let (bg, bar) = comment_colors(ui.visuals().dark_mode);
    let fade = |color: Color32| {
        if comment.sent {
            color.gamma_multiply(0.5)
        } else {
            color
        }
    };
    painter.rect_filled(row, 0.0, fade(bg));
    let left = row.left() + style.digit_width * GUTTER_DIGITS as f32;
    painter.rect_filled(
        egui::Rect::from_min_size(
            egui::pos2(left - style.digit_width, row.top()),
            Vec2::new(3.0, row.height()),
        ),
        0.0,
        fade(bar),
    );
    let font = egui::TextStyle::Body.resolve(ui.style());
    let text = comment.text.lines().nth(line).unwrap_or_default();
    let end = painter.text(
        egui::pos2(left, row.center().y),
        egui::Align2::LEFT_CENTER,
        text,
        font.clone(),
        fade(style.text_color),
    );
    if line == 0 && comment.sent {
        painter.text(
            egui::pos2(end.right() + 12.0, row.center().y),
            egui::Align2::LEFT_CENTER,
            "sent",
            font,
            style.weak_color,
        );
    }
}

fn gutter_mark_ui(ui: &egui::Ui, rect: egui::Rect, sign: &str, label: &str, id: egui::Id) -> bool {
    let response = ui
        .interact(rect, id, Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), label));
    let visuals = ui.style().interact(&response);
    let mark = rect.shrink(2.0);
    ui.painter().rect(
        mark,
        3.0,
        visuals.bg_fill,
        visuals.bg_stroke,
        egui::StrokeKind::Inside,
    );
    ui.painter().text(
        mark.center(),
        egui::Align2::CENTER_CENTER,
        sign,
        egui::TextStyle::Body.resolve(ui.style()),
        visuals.text_color(),
    );
    response.on_hover_text(label).clicked()
}

fn comment_mark_ui(ui: &egui::Ui, rect: egui::Rect, id: egui::Id) -> bool {
    gutter_mark_ui(ui, rect, "+", "Comment on this line", id)
}

fn delete_mark_ui(ui: &egui::Ui, rect: egui::Rect, id: egui::Id) -> bool {
    gutter_mark_ui(ui, rect, "×", "Delete comment", id)
}

fn fold_ui(ui: &egui::Ui, rect: egui::Rect, hidden: usize, id: egui::Id) -> bool {
    let label = format!("Show {hidden} hidden lines");
    let response = ui
        .interact(rect, id, Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), &label));
    let color = if response.hovered() {
        ui.visuals().text_color()
    } else {
        ui.visuals().weak_text_color().gamma_multiply(0.45)
    };
    let (amplitude, wavelength) = (1.5, 12.0);
    let points = (0..=(rect.width() / 2.0) as usize)
        .map(|step| {
            let x = step as f32 * 2.0;
            egui::pos2(
                rect.left() + x,
                rect.center().y + amplitude * (x / wavelength * std::f32::consts::TAU).sin(),
            )
        })
        .collect();
    ui.painter()
        .add(egui::Shape::line(points, Stroke::new(1.0, color)));
    response.on_hover_text(label).clicked()
}

pub(super) fn reviewed_mark_ui(
    ui: &egui::Ui,
    rect: egui::Rect,
    shrink: f32,
    reviewed: Option<bool>,
    label: &str,
    id: egui::Id,
    interactive: bool,
) -> bool {
    let response = ui.interact(rect, id, Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::Role::CheckBox,
            ui.is_enabled(),
            reviewed == Some(true),
            label,
        )
    });
    let visuals = ui.style().interact(&response);
    let mark =
        egui::Rect::from_center_size(rect.center(), Vec2::splat(rect.height() - 5.0 - shrink));
    ui.painter().rect(
        mark,
        2.0,
        visuals.bg_fill,
        visuals.bg_stroke,
        egui::StrokeKind::Inside,
    );
    let at = |x: f32, y: f32| {
        egui::pos2(
            mark.left() + mark.width() * x,
            mark.top() + mark.height() * y,
        )
    };
    let sign = match reviewed {
        Some(true) => vec![at(0.2, 0.5), at(0.45, 0.75), at(0.8, 0.25)],
        Some(false) => return interactive && response.clicked(),
        None => vec![at(0.25, 0.5), at(0.75, 0.5)],
    };
    ui.painter().add(egui::Shape::line(sign, visuals.fg_stroke));
    interactive && response.clicked()
}
